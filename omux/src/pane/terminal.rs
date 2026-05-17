//! A single VTE terminal pane.
//!
//! Spawns the user's `$SHELL` and exports `OMUX_PANE_ID` into the child
//! environment so future hook plumbing (M4) can correlate D-Bus signals
//! back to the pane that produced them.

use gtk4::gio;
use gtk4::glib;
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{PtyFlags, Terminal};

use super::PaneKind;

pub struct TerminalPane {
    widget: Terminal,
    pane_id: Uuid,
}

impl TerminalPane {
    /// Create a new terminal pane, spawn the user's shell in it, and return
    /// the wrapper. The VTE widget is owned by the pane; access via
    /// [`Self::widget`] to attach it to a parent container.
    pub fn new() -> Self {
        let pane_id = Uuid::new_v4();
        let widget = Terminal::new();

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let omux_env = format!("OMUX_PANE_ID={pane_id}");

        let pane_id_for_log = pane_id;
        widget.spawn_async(
            PtyFlags::DEFAULT,
            Some(home.as_str()),
            &[shell.as_str()],
            &[omux_env.as_str()],
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(pid) => {
                    tracing::info!(pane_id = %pane_id_for_log, ?pid, "spawned shell");
                }
                Err(e) => {
                    tracing::error!(pane_id = %pane_id_for_log, error = %e, "failed to spawn shell");
                }
            },
        );

        Self { widget, pane_id }
    }

    pub fn widget(&self) -> &Terminal {
        &self.widget
    }

    pub fn pane_id(&self) -> Uuid {
        self.pane_id
    }

    pub fn kind(&self) -> PaneKind {
        PaneKind::Terminal
    }
}

impl Default for TerminalPane {
    fn default() -> Self {
        Self::new()
    }
}
