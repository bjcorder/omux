//! Application shell.
//!
//! `AppShell` glues together:
//! * the [`WorkspaceManager`] (persistence),
//! * the [`Sidebar`] widget (UI mirror of the workspace list),
//! * a map of live [`PaneTree`]s — one per workspace the user has
//!   opened in this session — and the currently-mounted one,
//! * the dialogs for create / rename / delete.
//!
//! Live-tree map: each workspace's tree is kept alive across switches so
//! terminals (running shells, scrollback, agent state) and browser pages
//! survive when the user clicks away and back. Trees are dropped only
//! when their workspace is deleted or the app exits. The original
//! design.md §1 "one live tree" model was a wrong call — it killed the
//! shells on every switch — so we widened to the map model after the
//! first round of live smoke testing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use crate::agent::hook_installer;
use crate::agent::manifest::CompiledManifest;
use crate::agent::status::StatusEvent;
use crate::ipc::{HookEvent, HookEventKind, SocketService};
use crate::pane::terminal::TerminalPane;
use crate::pane::tree::PaneTree;
use crate::workspace::WorkspaceConfig;
use crate::workspace::WorkspaceManager;
use crate::workspace::snapshot::LayoutNode;

use super::sidebar::{Sidebar, WorkspaceRowData};

type PaneRegistry = Rc<RefCell<HashMap<Uuid, TerminalPane>>>;
type LiveTrees = Rc<RefCell<HashMap<String, PaneTree>>>;

const DEFAULT_WORKSPACE_NAME: &str = "default";

#[derive(Clone)]
pub struct AppShell {
    window: adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    sidebar: Sidebar,
    content_bin: adw::Bin,
    /// All workspace trees that have been opened at least once during
    /// this session. Kept alive so switching workspaces preserves their
    /// terminals (running shells, scrollback, agent state, browser
    /// history). Dropped when the workspace is deleted or the app exits.
    trees: LiveTrees,
    /// Name of the currently mounted workspace, or `None` if no
    /// workspaces exist.
    active_name: Rc<RefCell<Option<String>>>,
    manifests: Rc<Vec<CompiledManifest>>,
    pane_registry: PaneRegistry,
    socket_service: Rc<RefCell<Option<SocketService>>>,
}

impl AppShell {
    pub fn build(
        app: &adw::Application,
        manager: WorkspaceManager,
        manifests: Vec<CompiledManifest>,
    ) -> Self {
        let manager = Rc::new(RefCell::new(manager));
        let manifests = Rc::new(manifests);

        // Window + chrome.
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("omux")
            .default_width(1280)
            .default_height(800)
            .build();

        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        // Sidebar + content split. `gtk::Paned` (rather than
        // `adw::OverlaySplitView`) so the divider is user-draggable —
        // the workspace labels are short and most users want the
        // sidebar narrower than Adwaita's default.
        let sidebar = Sidebar::new();
        let content_bin = adw::Bin::new();
        let split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(sidebar.widget())
            .end_child(&content_bin)
            // Sidebar keeps its width on window resize; only content grows.
            .resize_start_child(false)
            .resize_end_child(true)
            // Allow shrinking down to children's natural minimum.
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(140)
            .build();
        // Persist the user's chosen width across restarts.
        wire_sidebar_width_persistence(&split, &manager);

        toolbar.set_content(Some(&split));
        window.set_content(Some(&toolbar));

        let trees: LiveTrees = Rc::new(RefCell::new(HashMap::new()));
        let active_name: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let pane_registry: PaneRegistry = Rc::new(RefCell::new(HashMap::new()));
        let socket_service: Rc<RefCell<Option<SocketService>>> = Rc::new(RefCell::new(None));

        let shell = Self {
            window,
            manager,
            sidebar,
            content_bin,
            trees,
            active_name,
            manifests,
            pane_registry,
            socket_service,
        };

        ensure_default_workspace(&shell.manager);
        shell.refresh_sidebar();
        shell.wire_callbacks();
        shell.start_socket_service();
        shell.start_badge_refresh_timer();
        shell.restore_initial_workspace();
        shell.maybe_show_hook_install_dialog();

        shell
    }

    /// Every 250 ms, walk the active workspace's PaneTree to:
    ///
    /// 1. Sync each tab + sidebar badge to its pane's current status.
    /// 2. Refresh the pane registry so hook events can route to panes
    ///    added since the last workspace switch (new-tab, split, etc.).
    ///
    /// Cheap; the alternative (status-change + add-pane callbacks routed
    /// through every mutation site) adds threading-style complexity
    /// without measurable UX win.
    fn start_badge_refresh_timer(&self) {
        use crate::agent::status::PaneStatus;
        let active_name = self.active_name.clone();
        let trees = self.trees.clone();
        let sidebar = self.sidebar.clone();
        let registry = self.pane_registry.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let active = active_name.borrow().clone();
            let Some(name) = active else {
                return glib::ControlFlow::Continue;
            };
            let trees_ref = trees.borrow();
            let Some(tree) = trees_ref.get(&name) else {
                return glib::ControlFlow::Continue;
            };
            tree.refresh_badges();
            let panes = tree.terminal_panes();

            // Reconcile registry with the live tree.
            let mut reg = registry.borrow_mut();
            let live_ids: std::collections::HashSet<Uuid> =
                panes.iter().map(|p| p.pane_id()).collect();
            reg.retain(|id, _| live_ids.contains(id));
            for p in &panes {
                reg.entry(p.pane_id()).or_insert_with(|| p.clone());
            }
            drop(reg);

            let count = panes
                .iter()
                .filter(|t| t.status() == PaneStatus::NeedsAttention)
                .count();
            sidebar.set_workspace_badge(&name, count);

            glib::ControlFlow::Continue
        });
    }

    fn maybe_show_hook_install_dialog(&self) {
        if hook_installer::already_installed() {
            return;
        }
        let body_text = match hook_installer::settings_path() {
            Some(p) => format!(
                "omux can install Stop and Notification hooks into your Claude Code settings ({}) so panes light up when Claude finishes a turn or needs input. The original file will be backed up to <…>.omux-backup.",
                p.display(),
            ),
            None => "omux could not resolve $HOME; the hook install is unavailable on this system."
                .to_string(),
        };
        let dialog =
            adw::AlertDialog::new(Some("Enable Claude Code notifications?"), Some(&body_text));
        dialog.add_responses(&[("skip", "Not now"), ("install", "Install hooks")]);
        dialog.set_response_appearance("install", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("install"));
        dialog.set_close_response("skip");

        let window = self.window.clone();
        dialog.connect_response(None, move |dialog, resp| {
            if resp == "install" {
                match hook_installer::install() {
                    Ok(result) if result.installed_now => {
                        tracing::info!(
                            backup = %result.backup_path.display(),
                            "installed Claude Code hooks",
                        );
                        show_info(
                            &window,
                            "Hooks installed",
                            "Claude Code will now notify omux when a turn ends.",
                        );
                    }
                    Ok(_) => {
                        tracing::info!("hooks already present; nothing to install");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "hook install failed");
                        show_info(&window, "Couldn't install hooks", &format!("Reason: {e}"));
                    }
                }
            }
            dialog.close();
        });

        dialog.present(Some(&self.window));
    }

    fn start_socket_service(&self) {
        let registry = self.pane_registry.clone();
        let handler: crate::ipc::socket_service::EventHandler = Rc::new(move |event: HookEvent| {
            deliver_event(&registry, event);
        });

        // Drain any events buffered while omux wasn't running.
        SocketService::drain_pending(&handler);

        match SocketService::start(handler) {
            Ok(service) => *self.socket_service.borrow_mut() = Some(service),
            Err(e) => tracing::warn!(error = %e, "could not start control socket"),
        }
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// A cheap clone that shares all inner state. Use this when capturing
    /// the shell into long-lived closures (keyboard-shortcut callbacks etc.).
    pub fn handle(&self) -> Self {
        self.clone()
    }

    pub fn window_for_shortcuts(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// Run `f` against the currently mounted PaneTree, if any.
    pub fn with_active_tree<F: FnOnce(&PaneTree)>(&self, f: F) {
        let Some(name) = self.active_name.borrow().clone() else {
            return;
        };
        let trees = self.trees.borrow();
        if let Some(tree) = trees.get(&name) {
            f(tree);
        }
    }

    fn refresh_sidebar(&self) {
        let entries: Vec<WorkspaceRowData> = self
            .manager
            .borrow()
            .entries()
            .iter()
            .map(|e| WorkspaceRowData {
                name: e.config.name.clone(),
                pinned: e.config.pinned,
            })
            .collect();
        self.sidebar.set_workspaces(&entries);
        self.sidebar
            .set_active(self.manager.borrow().active_workspace_name());
    }

    fn restore_initial_workspace(&self) {
        let target = {
            let mgr = self.manager.borrow();
            mgr.active_workspace_name()
                .map(str::to_string)
                .or_else(|| mgr.entries().first().map(|e| e.config.name.clone()))
        };
        if let Some(name) = target {
            self.switch_to(&name);
        }
    }

    fn wire_callbacks(&self) {
        // on_select → switch workspace
        let manager = self.manager.clone();
        let trees = self.trees.clone();
        let active_name = self.active_name.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        let manifests = self.manifests.clone();
        self.sidebar.on_select(move |name| {
            switch_workspace(
                &manager,
                &trees,
                &active_name,
                &content_bin,
                name,
                &manifests,
            );
            sidebar.set_active(Some(name));
        });

        // on_new → "new workspace" dialog
        let manager = self.manager.clone();
        let trees = self.trees.clone();
        let active_name = self.active_name.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        let manifests = self.manifests.clone();
        self.sidebar.on_new(move || {
            show_new_workspace_dialog(
                &window,
                manager.clone(),
                trees.clone(),
                active_name.clone(),
                content_bin.clone(),
                sidebar.clone(),
                manifests.clone(),
            );
        });

        // on_rename → rename dialog
        let manager = self.manager.clone();
        let trees = self.trees.clone();
        let active_name = self.active_name.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        self.sidebar.on_rename(move |old, _placeholder| {
            show_rename_dialog(
                &window,
                manager.clone(),
                trees.clone(),
                active_name.clone(),
                sidebar.clone(),
                old.to_string(),
            );
        });

        // on_delete → confirm + delete
        let manager = self.manager.clone();
        let trees = self.trees.clone();
        let active_name = self.active_name.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        let manifests = self.manifests.clone();
        self.sidebar.on_delete(move |name| {
            show_delete_dialog(
                &window,
                manager.clone(),
                trees.clone(),
                active_name.clone(),
                content_bin.clone(),
                sidebar.clone(),
                name.to_string(),
                manifests.clone(),
            );
        });

        // on_pin_toggle → flip pinned + refresh
        let manager = self.manager.clone();
        let sidebar = self.sidebar.clone();
        self.sidebar.on_pin_toggle(move |name| {
            let currently_pinned = manager
                .borrow()
                .get(name)
                .map(|e| e.config.pinned)
                .unwrap_or(false);
            if let Err(e) = manager.borrow_mut().set_pinned(name, !currently_pinned) {
                tracing::warn!(error = %e, workspace = %name, "set_pinned failed");
            }
            refresh_sidebar(&manager, &sidebar);
        });

        // on_reorder → persist + refresh
        let manager = self.manager.clone();
        let sidebar = self.sidebar.clone();
        self.sidebar.on_reorder(move |names_in_order| {
            if let Err(e) = manager.borrow_mut().reorder(&names_in_order) {
                tracing::warn!(error = %e, "reorder failed");
            }
            refresh_sidebar(&manager, &sidebar);
        });

        // Window close → snapshot every live tree (so the layouts persist).
        let manager = self.manager.clone();
        let trees = self.trees.clone();
        self.window.connect_close_request(move |_| {
            persist_all_layouts(&manager, &trees);
            glib::Propagation::Proceed
        });
    }

    fn switch_to(&self, name: &str) {
        switch_workspace(
            &self.manager,
            &self.trees,
            &self.active_name,
            &self.content_bin,
            name,
            &self.manifests,
        );
        self.sidebar.set_active(Some(name));
    }
}

fn deliver_event(registry: &PaneRegistry, event: HookEvent) {
    let pane = registry.borrow().get(&event.pane_id).cloned();
    match pane {
        Some(p) => {
            let status_event = match event.kind {
                HookEventKind::Stop
                | HookEventKind::Notification
                | HookEventKind::RegexFallback => StatusEvent::AttentionRequested,
                HookEventKind::SessionStart => StatusEvent::AgentStarted,
            };
            p.apply_status_event(status_event);
        }
        None => {
            tracing::debug!(
                pane_id = %event.pane_id,
                kind = ?event.kind,
                "hook event for unknown pane (workspace probably switched away)",
            );
        }
    }
}

/// Wire the gtk::Paned divider's position to a persisted key in the
/// app's state DB. Reads the stored value (if any) and applies it
/// asynchronously after the widget is realized; saves the new value
/// whenever the user drags the divider.
fn wire_sidebar_width_persistence(paned: &gtk::Paned, manager: &Rc<RefCell<WorkspaceManager>>) {
    const KEY: &str = "sidebar_width";

    // Restore.
    if let Ok(Some(value)) = manager.borrow().app_state_get(KEY)
        && let Ok(px) = value.parse::<i32>()
        && px > 0
    {
        // glib::idle_add_local because position must be set after
        // initial allocation; otherwise GTK overrides our position
        // with its computed default.
        let paned_w = paned.clone();
        glib::idle_add_local_once(move || {
            paned_w.set_position(px);
        });
    }

    // Save on change. notify::position fires on every drag pixel —
    // throttle by waiting until the user pauses (500 ms after the
    // last change).
    let manager = manager.clone();
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    paned.connect_position_notify(move |p| {
        let pos = p.position();
        // Cancel the previous pending save.
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let manager = manager.clone();
        let pending_for_clear = pending.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
            let _ = manager.borrow().app_state_set(KEY, &pos.to_string());
            pending_for_clear.borrow_mut().take();
        });
        *pending.borrow_mut() = Some(id);
    });
}

fn ensure_default_workspace(manager: &Rc<RefCell<WorkspaceManager>>) {
    let mut mgr = manager.borrow_mut();
    if !mgr.entries().is_empty() {
        return;
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let cfg = WorkspaceConfig::new(DEFAULT_WORKSPACE_NAME, home);
    if let Err(e) = mgr.upsert(cfg) {
        tracing::warn!(error = %e, "could not seed default workspace");
        return;
    }
    if let Err(e) = mgr.set_active(Some(DEFAULT_WORKSPACE_NAME)) {
        tracing::warn!(error = %e, "could not mark default workspace active");
    }
}

fn refresh_sidebar(manager: &Rc<RefCell<WorkspaceManager>>, sidebar: &Sidebar) {
    let entries: Vec<WorkspaceRowData> = manager
        .borrow()
        .entries()
        .iter()
        .map(|e| WorkspaceRowData {
            name: e.config.name.clone(),
            pinned: e.config.pinned,
        })
        .collect();
    sidebar.set_workspaces(&entries);
    sidebar.set_active(manager.borrow().active_workspace_name());
}

/// Mount the target workspace's tree in the content bin. Reuses any
/// already-live tree from `trees` so terminal state (running shells,
/// scrollback, browser history) survives workspace switches. Only the
/// first time a workspace is opened does it get built from its saved
/// snapshot.
fn switch_workspace(
    manager: &Rc<RefCell<WorkspaceManager>>,
    trees: &LiveTrees,
    active_name: &Rc<RefCell<Option<String>>>,
    content_bin: &adw::Bin,
    name: &str,
    manifests: &Rc<Vec<CompiledManifest>>,
) {
    // Get or build the target's tree.
    let tree_widget = {
        let mut map = trees.borrow_mut();
        let entry = map.entry(name.to_string()).or_insert_with(|| {
            let layout = manager
                .borrow()
                .get(name)
                .and_then(|e| e.config.layout.clone())
                .unwrap_or_else(LayoutNode::single_leaf);
            let session = build_network_session(name);
            PaneTree::from_snapshot(&layout, manifests, session)
        });
        entry.widget().clone()
    };

    content_bin.set_child(Some(&tree_widget));

    if let Err(e) = manager.borrow_mut().set_active(Some(name)) {
        tracing::warn!(error = %e, "set_active failed");
    }
    *active_name.borrow_mut() = Some(name.to_string());
}

/// Build a per-workspace WebKit `NetworkSession` whose data + cache live
/// under `$XDG_DATA_HOME/omux/web/<slug>/` and `…/cache/<slug>/`. That
/// gives each workspace its own cookies / local storage / etc.
fn build_network_session(workspace_name: &str) -> webkit6::NetworkSession {
    let slug = slugify_for_dir(workspace_name);
    let data_dir = crate::workspace::paths::config_dir()
        .map(|d| d.parent().map(|p| p.to_path_buf()).unwrap_or(d))
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("omux")
        .join("web")
        .join(&slug);
    let cache_dir = data_dir.join("cache");
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(&cache_dir);
    webkit6::NetworkSession::new(
        Some(&data_dir.to_string_lossy()),
        Some(&cache_dir.to_string_lossy()),
    )
}

fn slugify_for_dir(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("workspace");
    }
    out
}

/// Snapshot every live workspace's tree and persist its current layout.
/// Called on window close so each workspace's split shape, tab kinds,
/// and browser URLs survive across app restarts.
fn persist_all_layouts(manager: &Rc<RefCell<WorkspaceManager>>, trees: &LiveTrees) {
    let trees_ref = trees.borrow();
    for (name, tree) in trees_ref.iter() {
        let snapshot = tree.snapshot();
        let Some(mut cfg) = manager.borrow().get(name).map(|e| e.config.clone()) else {
            continue;
        };
        cfg.layout = Some(snapshot);
        if let Err(e) = manager.borrow_mut().upsert(cfg) {
            tracing::warn!(error = %e, workspace = %name, "could not persist layout");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn show_new_workspace_dialog(
    parent: &adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    trees: LiveTrees,
    active_name: Rc<RefCell<Option<String>>>,
    content_bin: adw::Bin,
    sidebar: Sidebar,
    manifests: Rc<Vec<CompiledManifest>>,
) {
    let entry = gtk::Entry::builder()
        .placeholder_text("Workspace name")
        .activates_default(true)
        .build();
    let folder_label = gtk::Label::builder()
        .label("Root folder: current directory")
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    body.append(&entry);
    body.append(&folder_label);

    let dialog = adw::AlertDialog::new(
        Some("New workspace"),
        Some("Pick a name. The root folder defaults to your current working directory."),
    );
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry_clone = entry.clone();
    let parent_for_cb = parent.clone();
    dialog.connect_response(None, move |dialog, resp| {
        if resp != "create" {
            return;
        }
        let name = entry_clone.text().trim().to_string();
        if name.is_empty() {
            tracing::info!("new workspace dialog: empty name, ignoring");
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let cfg = WorkspaceConfig::new(&name, cwd);
        // Bind the upsert result to a local first — `match
        // manager.borrow_mut().upsert(...) { ... }` would keep the
        // borrow_mut alive for the whole match expression, and
        // switch_workspace inside the Ok arm tries to manager.borrow()
        // → BorrowError panic.
        let upsert_result = manager.borrow_mut().upsert(cfg);
        match upsert_result {
            Ok(()) => {
                switch_workspace(
                    &manager,
                    &trees,
                    &active_name,
                    &content_bin,
                    &name,
                    &manifests,
                );
                refresh_sidebar(&manager, &sidebar);
            }
            Err(e) => {
                tracing::warn!(error = %e, "create workspace failed");
                show_error_dialog(&parent_for_cb, &format!("Could not create workspace: {e}"));
            }
        }
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_rename_dialog(
    parent: &adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    trees: LiveTrees,
    active_name: Rc<RefCell<Option<String>>>,
    sidebar: Sidebar,
    old: String,
) {
    let entry = gtk::Entry::builder()
        .text(&old)
        .activates_default(true)
        .build();
    let dialog = adw::AlertDialog::new(Some("Rename workspace"), None);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let entry_clone = entry.clone();
    let parent_for_cb = parent.clone();
    dialog.connect_response(None, move |dialog, resp| {
        if resp != "rename" {
            return;
        }
        let new = entry_clone.text().trim().to_string();
        if new.is_empty() || new == old {
            return;
        }
        // Hoist into a local so the borrow_mut on `manager` is released
        // before the success branch (which calls manager.borrow() via
        // refresh_sidebar).
        let rename_result = manager.borrow_mut().rename(&old, &new);
        if let Err(e) = rename_result {
            tracing::warn!(error = %e, "rename failed");
            show_error_dialog(&parent_for_cb, &format!("Could not rename: {e}"));
        } else {
            // Move the live tree to the new name so it stays mounted.
            let mut trees_mut = trees.borrow_mut();
            if let Some(tree) = trees_mut.remove(&old) {
                trees_mut.insert(new.clone(), tree);
            }
            drop(trees_mut);
            if active_name.borrow().as_deref() == Some(old.as_str()) {
                *active_name.borrow_mut() = Some(new.clone());
            }
            refresh_sidebar(&manager, &sidebar);
        }
        dialog.close();
    });

    dialog.present(Some(parent));
}

#[allow(clippy::too_many_arguments)]
fn show_delete_dialog(
    parent: &adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    trees: LiveTrees,
    active_name: Rc<RefCell<Option<String>>>,
    content_bin: adw::Bin,
    sidebar: Sidebar,
    name: String,
    manifests: Rc<Vec<CompiledManifest>>,
) {
    let dialog = adw::AlertDialog::new(
        Some("Delete workspace?"),
        Some(&format!(
            "{name:?} will be removed. Any unsaved terminal state is lost.",
        )),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let parent_for_cb = parent.clone();
    dialog.connect_response(None, move |dialog, resp| {
        if resp != "delete" {
            return;
        }
        let was_active = manager.borrow().active_workspace_name() == Some(name.as_str());
        if let Err(e) = manager.borrow_mut().delete(&name) {
            tracing::warn!(error = %e, "delete failed");
            show_error_dialog(&parent_for_cb, &format!("Could not delete: {e}"));
            dialog.close();
            return;
        }
        // Drop the deleted workspace's tree (kills its shells).
        trees.borrow_mut().remove(&name);
        if was_active {
            *active_name.borrow_mut() = None;
            content_bin.set_child(gtk::Widget::NONE);
            // Pick a new active workspace if any remain.
            let next = manager
                .borrow()
                .entries()
                .first()
                .map(|e| e.config.name.clone());
            if let Some(next) = next {
                switch_workspace(
                    &manager,
                    &trees,
                    &active_name,
                    &content_bin,
                    &next,
                    &manifests,
                );
            }
        }
        refresh_sidebar(&manager, &sidebar);
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_error_dialog(parent: &adw::ApplicationWindow, message: &str) {
    show_info(parent, "Something went wrong", message);
}

fn show_info(parent: &adw::ApplicationWindow, title: &str, message: &str) {
    let dialog = adw::AlertDialog::new(Some(title), Some(message));
    dialog.add_responses(&[("ok", "OK")]);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(parent));
}
