//! omux — M2: split panes + per-pane tabs + keyboard shortcuts.

mod pane;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{CallbackAction, Orientation, Shortcut, ShortcutController, ShortcutTrigger};
use libadwaita as adw;
use libadwaita::prelude::*;

use pane::tree::PaneTree;

const APP_ID: &str = "org.omux.Omux";

fn main() -> glib::ExitCode {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("omux=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("omux")
        .default_width(1280)
        .default_height(800)
        .build();

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);

    let tree = PaneTree::new();
    view.set_content(Some(tree.widget()));

    install_shortcuts(&window, &tree);

    window.set_content(Some(&view));
    window.present();
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
