//! omux — M3 (phase A + scaffold): workspaces persist across restarts.
//!
//! The full sidebar UI with create/rename/delete/pin/reorder lands in a
//! follow-up iteration; this binary opens the [`WorkspaceManager`],
//! restores the last-active workspace's layout, and snapshots the layout
//! on window close so the next launch picks up where we left off.

mod pane;
mod workspace;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{CallbackAction, Orientation, Shortcut, ShortcutController, ShortcutTrigger};
use libadwaita as adw;
use libadwaita::prelude::*;

use pane::tree::PaneTree;
use workspace::WorkspaceConfig;
use workspace::WorkspaceManager;
use workspace::snapshot::LayoutNode;

const APP_ID: &str = "org.omux.Omux";
const DEFAULT_WORKSPACE_NAME: &str = "default";

fn main() -> glib::ExitCode {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("omux=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let manager = match WorkspaceManager::open() {
        Ok(m) => Rc::new(RefCell::new(m)),
        Err(e) => {
            tracing::error!(error = %e, "could not open workspace manager; aborting");
            app.quit();
            return;
        }
    };

    // Ensure there's a workspace to open. On first launch this seeds a
    // single "default" workspace tied to the user's home dir.
    ensure_default_workspace(&manager);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("omux")
        .default_width(1280)
        .default_height(800)
        .build();

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);

    let tree = restore_active_tree(&manager);
    tracing::info!(
        workspace_count = manager.borrow().entries().len(),
        active = ?manager.borrow().active_workspace_name(),
        "shell ready",
    );

    view.set_content(Some(tree.widget()));
    install_shortcuts(&window, &tree);

    // Snapshot the active workspace's layout when the window closes so
    // the next launch picks up the same shape.
    let manager_close = manager.clone();
    let tree_close = tree.clone();
    window.connect_close_request(move |_| {
        persist_active_layout(&manager_close, &tree_close);
        glib::Propagation::Proceed
    });

    window.set_content(Some(&view));
    window.present();
}

fn ensure_default_workspace(manager: &Rc<RefCell<WorkspaceManager>>) {
    let mut mgr = manager.borrow_mut();
    if !mgr.entries().is_empty() {
        return;
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));
    let cfg = WorkspaceConfig::new(DEFAULT_WORKSPACE_NAME, home);
    if let Err(e) = mgr.upsert(cfg) {
        tracing::warn!(error = %e, "could not seed default workspace");
        return;
    }
    if let Err(e) = mgr.set_active(Some(DEFAULT_WORKSPACE_NAME)) {
        tracing::warn!(error = %e, "could not mark default workspace active");
    }
}

fn restore_active_tree(manager: &Rc<RefCell<WorkspaceManager>>) -> PaneTree {
    let mgr = manager.borrow();
    let layout = mgr
        .active_workspace_name()
        .and_then(|name| mgr.get(name))
        .or_else(|| mgr.entries().first())
        .and_then(|e| e.config.layout.clone())
        .unwrap_or_else(LayoutNode::single_leaf);
    PaneTree::from_snapshot(&layout)
}

fn persist_active_layout(manager: &Rc<RefCell<WorkspaceManager>>, tree: &PaneTree) {
    let snapshot = tree.snapshot();
    let mut mgr = manager.borrow_mut();
    let Some(name) = mgr.active_workspace_name().map(str::to_string) else {
        return;
    };
    let Some(entry) = mgr.get(&name) else { return };
    let mut cfg = entry.config.clone();
    cfg.layout = Some(snapshot);
    if let Err(e) = mgr.upsert(cfg) {
        tracing::warn!(error = %e, workspace = %name, "could not persist layout");
    }
}

fn install_shortcuts(window: &adw::ApplicationWindow, tree: &PaneTree) {
    let controller = ShortcutController::new();
    controller.set_scope(gtk4::ShortcutScope::Global);

    add_shortcut(&controller, "<Control><Shift>d", {
        let tree = tree.clone();
        move || tree.split(Orientation::Horizontal)
    });
    add_shortcut(&controller, "<Control><Shift>e", {
        let tree = tree.clone();
        move || tree.split(Orientation::Vertical)
    });
    add_shortcut(&controller, "<Control>t", {
        let tree = tree.clone();
        move || tree.new_tab_in_focused()
    });
    add_shortcut(&controller, "<Control>Tab", {
        let tree = tree.clone();
        move || tree.focus_next_leaf()
    });
    add_shortcut(&controller, "<Control><Shift>Tab", {
        let tree = tree.clone();
        move || tree.focus_prev_leaf()
    });

    window.add_controller(controller);
}

fn add_shortcut<F>(controller: &ShortcutController, accel: &str, f: F)
where
    F: Fn() + 'static,
{
    let Some(trigger) = ShortcutTrigger::parse_string(accel) else {
        tracing::warn!(accel, "could not parse shortcut trigger");
        return;
    };
    let action = CallbackAction::new(move |_widget, _args| {
        f();
        glib::Propagation::Stop
    });
    controller.add_shortcut(Shortcut::new(Some(trigger), Some(action)));
}
