//! Pane abstractions: a leaf in the layout tree holds a tab list of `Pane`s.
//!
//! At M5 two pane kinds exist:
//!
//! * [`terminal::TerminalPane`] — VTE-backed shell with agent detection.
//! * [`browser::BrowserPane`] — WebKitGTK web view with URL bar.
//! * [`scratchpad::ScratchpadPane`] — ephemeral text notes.

pub mod browser;
pub mod scratchpad;
pub mod terminal;
pub mod tree;

use gtk4 as gtk;

use browser::BrowserPane;
use scratchpad::ScratchpadPane;
use terminal::TerminalPane;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Browser,
    Scratchpad,
}

/// A tab inside a leaf. Stored heterogeneously so the same `Vec<Pane>`
/// can hold a mix of terminal + browser tabs.
#[derive(Clone)]
pub enum Pane {
    Terminal(TerminalPane),
    Browser(BrowserPane),
    Scratchpad(ScratchpadPane),
}

impl Pane {
    pub fn widget(&self) -> &gtk::Frame {
        match self {
            Pane::Terminal(t) => t.widget(),
            Pane::Browser(b) => b.widget(),
            Pane::Scratchpad(s) => s.widget(),
        }
    }

    #[allow(dead_code)] // Read by persistence/snapshot code in tree.rs.
    pub fn kind(&self) -> PaneKind {
        match self {
            Pane::Terminal(_) => PaneKind::Terminal,
            Pane::Browser(_) => PaneKind::Browser,
            Pane::Scratchpad(_) => PaneKind::Scratchpad,
        }
    }

    /// The stable id used to address this pane in the IPC registry and
    /// in close-by-id paths. For terminal panes this is what the agent
    /// hook environment exports as `OMUX_PANE_ID`.
    pub fn pane_id(&self) -> uuid::Uuid {
        match self {
            Pane::Terminal(t) => t.pane_id(),
            Pane::Browser(b) => b.pane_id(),
            Pane::Scratchpad(s) => s.pane_id(),
        }
    }

    pub fn as_terminal(&self) -> Option<&TerminalPane> {
        match self {
            Pane::Terminal(t) => Some(t),
            Pane::Browser(_) | Pane::Scratchpad(_) => None,
        }
    }

    /// Label shown on the tab.
    pub fn tab_label(&self) -> &'static str {
        match self {
            Pane::Terminal(_) => "shell",
            Pane::Browser(_) => "web",
            Pane::Scratchpad(_) => "note",
        }
    }

    /// Move focus to the most natural inner widget for typing.
    pub fn grab_inner_focus(&self) {
        use gtk4::prelude::*;
        match self {
            Pane::Terminal(t) => {
                t.terminal().grab_focus();
            }
            Pane::Browser(b) => {
                b.focus_url_bar();
            }
            Pane::Scratchpad(s) => {
                s.focus_editor();
            }
        }
    }
}
