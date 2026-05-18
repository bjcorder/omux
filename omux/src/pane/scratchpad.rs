//! Ephemeral text scratchpad pane.
//!
//! Scratchpad tabs persist as layout entries, but their text buffer is
//! deliberately runtime-only: restored scratchpads always start blank.

use gtk4 as gtk;
use gtk4::prelude::*;
use uuid::Uuid;

use super::PaneKind;

#[derive(Clone)]
pub struct ScratchpadPane {
    container: gtk::Frame,
    text_view: gtk::TextView,
    pane_id: Uuid,
}

impl ScratchpadPane {
    pub fn new() -> Self {
        let pane_id = Uuid::new_v4();
        let text_view = gtk::TextView::builder()
            .accepts_tab(true)
            .hexpand(true)
            .monospace(true)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .vexpand(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .child(&text_view)
            .hexpand(true)
            .vexpand(true)
            .build();

        let container = gtk::Frame::builder()
            .css_classes(["pane-frame", "scratchpad-pane"])
            .child(&scroller)
            .build();

        Self {
            container,
            text_view,
            pane_id,
        }
    }

    pub fn widget(&self) -> &gtk::Frame {
        &self.container
    }

    pub fn focus_editor(&self) {
        self.text_view.grab_focus();
    }

    #[allow(dead_code)]
    pub fn pane_id(&self) -> Uuid {
        self.pane_id
    }

    #[allow(dead_code)]
    pub fn kind(&self) -> PaneKind {
        PaneKind::Scratchpad
    }
}

impl Default for ScratchpadPane {
    fn default() -> Self {
        Self::new()
    }
}
