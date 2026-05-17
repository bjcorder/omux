//! omux — M1: window with a single VTE terminal pane running `$SHELL`.

mod pane;

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use pane::terminal::TerminalPane;

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

    let pane = TerminalPane::new();
    tracing::info!(
        pane_id = %pane.pane_id(),
        kind = ?pane.kind(),
        "mounted initial pane",
    );

    view.set_content(Some(pane.widget()));

    window.set_content(Some(&view));
    window.present();
}
