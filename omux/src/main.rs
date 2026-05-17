//! omux — main entry point.
//!
//! Loads agent manifests, opens the [`WorkspaceManager`], installs the
//! shared CSS stylesheet, and hands off to [`ui::AppShell`].

mod agent;
mod ipc;
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

    // Handle one-shot CLI commands before opening the GUI.
    let args: Vec<String> = std::env::args().collect();
    for arg in &args[1..] {
        match arg.as_str() {
            "--uninstall-hooks" => return run_uninstall_hooks(),
            "--help" | "-h" => return print_help(),
            "--version" | "-V" => {
                println!("omux {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            _ => {}
        }
    }

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| install_css());
    app.connect_activate(build_ui);
    // Don't pass argv through; we've already consumed it for our own flags.
    app.run_with_args::<&str>(&[])
}

fn run_uninstall_hooks() -> glib::ExitCode {
    match agent::hook_installer::uninstall() {
        Ok(true) => {
            eprintln!("omux: restored ~/.claude/settings.json from backup");
            glib::ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("omux: no backup found; nothing to uninstall");
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("omux: uninstall failed: {e}");
            glib::ExitCode::FAILURE
        }
    }
}

fn print_help() -> glib::ExitCode {
    println!(
        "omux {} — multi-terminal workspace with agent-attention notifications\n\
         \n\
         USAGE:\n    \
             omux                       launch the GUI\n    \
             omux --uninstall-hooks     remove omux hooks from ~/.claude/settings.json\n    \
             omux --version             print version\n    \
             omux --help                show this message\n\
         \n\
         RUNTIME ENV:\n    \
             OMUX_PANE_ID    injected by omux into each pane shell; read by omux-hook\n    \
             OMUX_SOCKET     override the control socket path (default: \
             $XDG_RUNTIME_DIR/omux/control.sock)",
        env!("CARGO_PKG_VERSION"),
    );
    glib::ExitCode::SUCCESS
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
    install_window_actions(shell);

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
    add_shortcut(&controller, "<Control><Shift>b", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.new_browser_tab_in_focused(None))
    });
    add_shortcut(&controller, "<Control>Tab", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.focus_next_leaf())
    });
    add_shortcut(&controller, "<Control><Shift>Tab", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.focus_prev_leaf())
    });
    add_shortcut(&controller, "<Control><Shift>c", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.copy_active_selection())
    });
    add_shortcut(&controller, "<Control><Shift>v", {
        let shell = shell.handle();
        move || shell.with_active_tree(|tree| tree.paste_to_active())
    });
    add_shortcut(&controller, "<Control>w", {
        let shell = shell.handle();
        move || {
            shell.with_active_tree(|tree| {
                let _ = tree.close_focused_tab();
            });
        }
    });

    shell.window_for_shortcuts().add_controller(controller);
}

/// Install window-level GAction handlers for the entries the pane
/// context menu binds to (`win.split-h`, `win.split-v`, `win.new-tab`,
/// `win.close-tab`).
fn install_window_actions(shell: &AppShell) {
    use gtk4::gio;
    let window = shell.window_for_shortcuts();

    let split_h = gio::SimpleAction::new("split-h", None);
    let shell_h = shell.handle();
    split_h.connect_activate(move |_, _| {
        shell_h.with_active_tree(|tree| tree.split(Orientation::Horizontal));
    });
    window.add_action(&split_h);

    let split_v = gio::SimpleAction::new("split-v", None);
    let shell_v = shell.handle();
    split_v.connect_activate(move |_, _| {
        shell_v.with_active_tree(|tree| tree.split(Orientation::Vertical));
    });
    window.add_action(&split_v);

    let new_tab = gio::SimpleAction::new("new-tab", None);
    let shell_t = shell.handle();
    new_tab.connect_activate(move |_, _| {
        shell_t.with_active_tree(|tree| tree.new_tab_in_focused());
    });
    window.add_action(&new_tab);

    let close_tab = gio::SimpleAction::new("close-tab", None);
    let shell_c = shell.handle();
    close_tab.connect_activate(move |_, _| {
        shell_c.with_active_tree(|tree| {
            let _ = tree.close_focused_tab();
        });
    });
    window.add_action(&close_tab);
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
