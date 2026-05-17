//! omux — M0 scaffold.
//!
//! At M0 this binary opens an empty Adwaita window. Real pane/agent
//! plumbing arrives in later milestones (see plan §10).

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

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

    let placeholder = gtk4::Label::builder()
        .label("omux — M0 scaffold. Panes arrive at M1.")
        .build();
    view.set_content(Some(&placeholder));

    window.set_content(Some(&view));
    window.present();
}
