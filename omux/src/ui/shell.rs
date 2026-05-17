//! Application shell.
//!
//! `AppShell` glues together:
//! * the [`WorkspaceManager`] (persistence),
//! * the [`Sidebar`] widget (UI mirror of the workspace list),
//! * a single live [`PaneTree`] mounted in an `adw::Bin` (the content area),
//! * the dialogs for create / rename / delete.
//!
//! Only one workspace's PaneTree is alive at a time. Switching snapshots
//! the current tree, persists it, and rebuilds the target from its saved
//! layout. PTY state is not preserved across switches (this is the cost of
//! the "one live tree" model; see design.md §1).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::pane::tree::PaneTree;
use crate::workspace::WorkspaceConfig;
use crate::workspace::WorkspaceManager;
use crate::workspace::snapshot::LayoutNode;

use super::sidebar::{Sidebar, WorkspaceRowData};

const DEFAULT_WORKSPACE_NAME: &str = "default";

#[derive(Clone)]
pub struct AppShell {
    window: adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    sidebar: Sidebar,
    content_bin: adw::Bin,
    active: Rc<RefCell<Option<(String, PaneTree)>>>,
}

impl AppShell {
    pub fn build(app: &adw::Application, manager: WorkspaceManager) -> Self {
        let manager = Rc::new(RefCell::new(manager));

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

        // Sidebar + content split.
        let sidebar = Sidebar::new();
        let content_bin = adw::Bin::new();
        let split = adw::OverlaySplitView::builder()
            .sidebar(sidebar.widget())
            .content(&content_bin)
            .min_sidebar_width(180.0)
            .max_sidebar_width(280.0)
            .show_sidebar(true)
            .build();

        toolbar.set_content(Some(&split));
        window.set_content(Some(&toolbar));

        let active: Rc<RefCell<Option<(String, PaneTree)>>> = Rc::new(RefCell::new(None));

        let shell = Self {
            window,
            manager,
            sidebar,
            content_bin,
            active,
        };

        ensure_default_workspace(&shell.manager);
        shell.refresh_sidebar();
        shell.wire_callbacks();
        shell.restore_initial_workspace();

        shell
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
        if let Some((_, tree)) = self.active.borrow().as_ref() {
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
        let active = self.active.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        self.sidebar.on_select(move |name| {
            switch_workspace(&manager, &active, &content_bin, name);
            sidebar.set_active(Some(name));
        });

        // on_new → "new workspace" dialog
        let manager = self.manager.clone();
        let active = self.active.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        self.sidebar.on_new(move || {
            show_new_workspace_dialog(
                &window,
                manager.clone(),
                active.clone(),
                content_bin.clone(),
                sidebar.clone(),
            );
        });

        // on_rename → rename dialog
        let manager = self.manager.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        self.sidebar.on_rename(move |old, _placeholder| {
            show_rename_dialog(&window, manager.clone(), sidebar.clone(), old.to_string());
        });

        // on_delete → confirm + delete
        let manager = self.manager.clone();
        let active = self.active.clone();
        let content_bin = self.content_bin.clone();
        let sidebar = self.sidebar.clone();
        let window = self.window.clone();
        self.sidebar.on_delete(move |name| {
            show_delete_dialog(
                &window,
                manager.clone(),
                active.clone(),
                content_bin.clone(),
                sidebar.clone(),
                name.to_string(),
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

        // Window close → snapshot active.
        let manager = self.manager.clone();
        let active = self.active.clone();
        self.window.connect_close_request(move |_| {
            save_active_layout(&manager, &active);
            glib::Propagation::Proceed
        });
    }

    fn switch_to(&self, name: &str) {
        switch_workspace(&self.manager, &self.active, &self.content_bin, name);
        self.sidebar.set_active(Some(name));
    }
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

fn switch_workspace(
    manager: &Rc<RefCell<WorkspaceManager>>,
    active: &Rc<RefCell<Option<(String, PaneTree)>>>,
    content_bin: &adw::Bin,
    name: &str,
) {
    // 1. Snapshot the currently active tree (if any) and persist its layout.
    save_active_layout(manager, active);

    // 2. Build the target's tree from its saved layout (or a fresh leaf).
    let layout = manager
        .borrow()
        .get(name)
        .and_then(|e| e.config.layout.clone())
        .unwrap_or_else(LayoutNode::single_leaf);
    let tree = PaneTree::from_snapshot(&layout);
    content_bin.set_child(Some(tree.widget()));

    // 3. Update manager + active record.
    if let Err(e) = manager.borrow_mut().set_active(Some(name)) {
        tracing::warn!(error = %e, "set_active failed");
    }
    *active.borrow_mut() = Some((name.to_string(), tree));
}

fn save_active_layout(
    manager: &Rc<RefCell<WorkspaceManager>>,
    active: &Rc<RefCell<Option<(String, PaneTree)>>>,
) {
    let Some((name, tree)) = active.borrow().clone() else {
        return;
    };
    let snapshot = tree.snapshot();
    let Some(mut cfg) = manager.borrow().get(&name).map(|e| e.config.clone()) else {
        return;
    };
    cfg.layout = Some(snapshot);
    if let Err(e) = manager.borrow_mut().upsert(cfg) {
        tracing::warn!(error = %e, workspace = %name, "could not persist layout");
    }
}

fn show_new_workspace_dialog(
    parent: &adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    active: Rc<RefCell<Option<(String, PaneTree)>>>,
    content_bin: adw::Bin,
    sidebar: Sidebar,
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
        match manager.borrow_mut().upsert(cfg) {
            Ok(()) => {
                switch_workspace(&manager, &active, &content_bin, &name);
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
        if let Err(e) = manager.borrow_mut().rename(&old, &new) {
            tracing::warn!(error = %e, "rename failed");
            show_error_dialog(&parent_for_cb, &format!("Could not rename: {e}"));
        } else {
            refresh_sidebar(&manager, &sidebar);
        }
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_delete_dialog(
    parent: &adw::ApplicationWindow,
    manager: Rc<RefCell<WorkspaceManager>>,
    active: Rc<RefCell<Option<(String, PaneTree)>>>,
    content_bin: adw::Bin,
    sidebar: Sidebar,
    name: String,
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
        if was_active {
            *active.borrow_mut() = None;
            content_bin.set_child(gtk::Widget::NONE);
            // Pick a new active workspace if any remain.
            let next = manager
                .borrow()
                .entries()
                .first()
                .map(|e| e.config.name.clone());
            if let Some(next) = next {
                switch_workspace(&manager, &active, &content_bin, &next);
            }
        }
        refresh_sidebar(&manager, &sidebar);
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_error_dialog(parent: &adw::ApplicationWindow, message: &str) {
    let dialog = adw::AlertDialog::new(Some("Something went wrong"), Some(message));
    dialog.add_responses(&[("ok", "OK")]);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(parent));
}
