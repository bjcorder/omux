//! WebKitGTK browser pane (design §5.2).
//!
//! Layout:
//!
//! ```text
//!  ┌───────────────────────────────────────────┐
//!  │ ←  →  ⟳  [ url entry ─────────────── ]    │   nav row
//!  ├───────────────────────────────────────────┤
//!  │                                           │
//!  │           webkit6::WebView                │   web view
//!  │                                           │
//!  └───────────────────────────────────────────┘
//! ```
//!
//! Browser panes carry an isolated `webkit6::NetworkSession` per
//! workspace so cookies and local storage stay scoped. The session is
//! provided by [`crate::pane::tree::PaneTree`] which is itself created
//! per workspace.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use uuid::Uuid;
use webkit6::prelude::*;
use webkit6::{NetworkSession, WebView};

use super::PaneKind;

const DEFAULT_URL: &str = "about:blank";

#[derive(Clone)]
pub struct BrowserPane {
    container: gtk::Frame,
    web_view: WebView,
    url_entry: gtk::Entry,
    pane_id: Uuid,
}

impl BrowserPane {
    /// Create a new browser pane backed by the provided NetworkSession.
    /// Initial URL is `about:blank` unless `start_url` is supplied.
    pub fn new(network_session: &NetworkSession, start_url: Option<&str>) -> Self {
        let pane_id = Uuid::new_v4();

        let web_view = WebView::builder()
            .network_session(network_session)
            .vexpand(true)
            .hexpand(true)
            .build();
        let initial = start_url.unwrap_or(DEFAULT_URL);
        web_view.load_uri(initial);

        let back_btn = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Back")
            .css_classes(["flat"])
            .build();
        let fwd_btn = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Forward")
            .css_classes(["flat"])
            .build();
        let reload_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Reload")
            .css_classes(["flat"])
            .build();
        let url_entry = gtk::Entry::builder()
            .text(initial)
            .placeholder_text("URL")
            .hexpand(true)
            .activates_default(false)
            .css_classes(["browser-url"])
            .build();

        // Connect signals.
        let view_back = web_view.clone();
        back_btn.connect_clicked(move |_| view_back.go_back());
        let view_fwd = web_view.clone();
        fwd_btn.connect_clicked(move |_| view_fwd.go_forward());
        let view_reload = web_view.clone();
        reload_btn.connect_clicked(move |_| view_reload.reload());

        let view_for_entry = web_view.clone();
        url_entry.connect_activate(move |entry| {
            let text = entry.text();
            let url = normalize_url(&text);
            view_for_entry.load_uri(&url);
        });

        // Track navigation so the entry mirrors the current page.
        let entry_for_uri = url_entry.clone();
        let updating: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let updating_clone = updating.clone();
        web_view.connect_uri_notify(move |view| {
            if let Some(uri) = view.uri() {
                *updating_clone.borrow_mut() = true;
                entry_for_uri.set_text(uri.as_str());
                *updating_clone.borrow_mut() = false;
            }
        });

        // Enable/disable back/fwd as history changes.
        let back_btn_for_changed = back_btn.clone();
        let fwd_btn_for_changed = fwd_btn.clone();
        let view_for_changed = web_view.clone();
        let sync_history = move || {
            back_btn_for_changed.set_sensitive(view_for_changed.can_go_back());
            fwd_btn_for_changed.set_sensitive(view_for_changed.can_go_forward());
        };
        sync_history();
        let sync_for_signal = sync_history.clone();
        web_view.connect_load_changed(move |_, _| sync_for_signal());

        let nav_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        nav_row.append(&back_btn);
        nav_row.append(&fwd_btn);
        nav_row.append(&reload_btn);
        nav_row.append(&url_entry);

        let stack = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        stack.append(&nav_row);
        stack.append(&web_view);

        let container = gtk::Frame::builder()
            .css_classes(["pane-frame", "browser-pane"])
            .child(&stack)
            .build();

        Self {
            container,
            web_view,
            url_entry,
            pane_id,
        }
    }

    pub fn widget(&self) -> &gtk::Frame {
        &self.container
    }

    #[allow(dead_code)] // Used by snapshot to record current URL.
    pub fn current_url(&self) -> Option<String> {
        self.web_view.uri().map(|s| s.to_string())
    }

    #[allow(dead_code)]
    pub fn pane_id(&self) -> Uuid {
        self.pane_id
    }

    #[allow(dead_code)]
    pub fn kind(&self) -> PaneKind {
        PaneKind::Browser
    }

    /// Make the URL entry the focused widget (handy for "open browser
    /// then start typing" UX).
    pub fn focus_url_bar(&self) {
        self.url_entry.grab_focus();
    }
}

fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DEFAULT_URL.to_string();
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("about:")
    {
        return trimmed.to_string();
    }
    // No scheme — treat as either localhost-style or web search. For v1
    // we default to https://; users wanting a web search can type the
    // full URL into a search engine.
    if trimmed.contains(' ') || !trimmed.contains('.') {
        // Heuristic: spaces or no dot → not a URL; route to DuckDuckGo.
        let escaped = url_encode(trimmed);
        return format!("https://duckduckgo.com/?q={escaped}");
    }
    format!("https://{trimmed}")
}

fn url_encode(s: &str) -> String {
    // Minimal RFC 3986 escaping for the subset we care about.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keeps_full_urls() {
        assert_eq!(normalize_url("https://x.com"), "https://x.com");
        assert_eq!(
            normalize_url("http://localhost:3000"),
            "http://localhost:3000"
        );
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }

    #[test]
    fn normalize_adds_https_to_bare_domain() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(
            normalize_url("docs.gtk.org/gtk4/"),
            "https://docs.gtk.org/gtk4/"
        );
    }

    #[test]
    fn normalize_routes_searches_to_duckduckgo() {
        let q = normalize_url("rust async tokio");
        assert!(q.starts_with("https://duckduckgo.com/?q="));
        assert!(q.contains("rust%20async%20tokio"));
    }

    #[test]
    fn normalize_empty_becomes_default() {
        assert_eq!(normalize_url(""), DEFAULT_URL);
        assert_eq!(normalize_url("   "), DEFAULT_URL);
    }
}
