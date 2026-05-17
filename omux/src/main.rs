//! omux — main entry point.
//!
//! Loads agent manifests, opens the [`WorkspaceManager`], installs the
//! shared CSS stylesheet, and hands off to [`ui::AppShell`].

mod agent;
mod pane;
mod ui;
mod workspace;

use gtk4 as gtk;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{CallbackAction, Orientation, Shortcut, ShortcutController, ShortcutTrigger};
use libadwaita as adw;

use agent::manifest::{self, CompiledManifest};
use ui::AppShell;
use workspace::WorkspaceManager;

const APP_ID: &str = "org.omux.Omux";
const STYLE_CSS: &str = include_str!("../resources/style.css");

fn main() -> glib::ExitCode {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("omux=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| install_css());
    app.connect_activate(build_ui);
    app.run()
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE_CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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

    let manifests = load_compiled_manifests();
    tracing::info!(
        manifest_count = manifests.len(),
        manifests = ?manifests.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        "loaded agent manifests",
    );

    let shell = AppShell::build(app, manager, manifests);
    install_window_shortcuts(&shell);
    shell.present();
}

fn load_compiled_manifests() -> Vec<CompiledManifest> {
    // User overrides at $XDG_CONFIG_HOME/omux/agents/*.toml (optional).
    let user_dir = workspace::paths::config_dir()
        .ok()
        .map(|d| d.join("agents"));
    let raw = manifest::load_all(user_dir.as_deref());
    raw.into_iter()
        .filter_map(|m| match m.compile() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(name = %m.name, error = %e, "agent manifest regex compile failed");
                None
            }
        })
        .collect()
}

fn install_window_shortcuts(shell: &AppShell) {
    let controller = ShortcutController::new();
    controller.set_scope(gtk4::ShortcutScope::Global);

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
