//! omux — M3 (full): workspaces with sidebar + persistent layouts.

mod pane;
mod ui;
mod workspace;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{CallbackAction, Orientation, Shortcut, ShortcutController, ShortcutTrigger};
use libadwaita as adw;

use ui::AppShell;
use workspace::WorkspaceManager;

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
    let manager = match WorkspaceManager::open() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "could not open workspace manager; aborting");
            app.quit();
            return;
        }
    };

    let shell = AppShell::build(app, manager);
    install_window_shortcuts(&shell);
    shell.present();
}

fn install_window_shortcuts(shell: &AppShell) {
    let controller = ShortcutController::new();
    controller.set_scope(gtk4::ShortcutScope::Global);

    // Pane shortcuts route through AppShell::active_tree so they target
    // whichever workspace is currently mounted.
    add_shortcut(&controller, "<Control><Shift>d", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.split(Orientation::Horizontal))
    });
    add_shortcut(&controller, "<Control><Shift>e", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.split(Orientation::Vertical))
    });
    add_shortcut(&controller, "<Control>t", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.new_tab_in_focused())
    });
    add_shortcut(&controller, "<Control>Tab", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.focus_next_leaf())
    });
    add_shortcut(&controller, "<Control><Shift>Tab", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.focus_prev_leaf())
    });

    shell.window_for_shortcuts().add_controller(controller);
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
