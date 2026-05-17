//! A single VTE terminal pane wrapped in a `gtk::Frame` (so CSS classes
//! can apply a notification ring around its perimeter).
//!
//! At M4 the pane:
//! * polls its PTY's foreground process group every 500ms and matches
//!   the process name against agent manifests;
//! * tracks a [`PaneStatus`] via the state machine in [`crate::agent::status`];
//! * applies the corresponding CSS class to its frame so the UI ring
//!   appears when the pane is in `NeedsAttention`.

use std::cell::{Cell, RefCell};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{PtyFlags, Terminal};

use crate::agent::detect::{self, Detection};
use crate::agent::manifest::CompiledManifest;
use crate::agent::status::{PaneStatus, StatusEvent};

use super::PaneKind;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct TerminalPane {
    container: gtk::Frame,
    terminal: Terminal,
    pane_id: Uuid,
    status: Rc<Cell<PaneStatus>>,
    detection: Rc<RefCell<Option<Detection>>>,
}

impl TerminalPane {
    pub fn new() -> Self {
        Self::new_with_manifests(&[])
    }

    pub fn new_with_manifests(manifests: &[CompiledManifest]) -> Self {
        let pane_id = Uuid::new_v4();
        let terminal = Terminal::new();
        let container = gtk::Frame::builder()
            .css_classes(["pane-frame"])
            .child(&terminal)
            .build();

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let omux_env = format!("OMUX_PANE_ID={pane_id}");

        let pane_id_for_log = pane_id;
        terminal.spawn_async(
            PtyFlags::DEFAULT,
            Some(home.as_str()),
            &[shell.as_str()],
            &[omux_env.as_str()],
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(pid) => tracing::info!(pane_id = %pane_id_for_log, ?pid, "spawned shell"),
                Err(e) => tracing::error!(pane_id = %pane_id_for_log, error = %e, "spawn failed"),
            },
        );

        let me = Self {
            container,
            terminal,
            pane_id,
            status: Rc::new(Cell::new(PaneStatus::Idle)),
            detection: Rc::new(RefCell::new(None)),
        };

        me.install_focus_clear();
        if !manifests.is_empty() {
            me.start_polling(manifests.to_vec());
        }
        me.apply_status_class();

        me
    }

    pub fn widget(&self) -> &gtk::Frame {
        &self.container
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    #[allow(dead_code)] // Wired into D-Bus / hook plumbing at M4 phase D.
    pub fn pane_id(&self) -> Uuid {
        self.pane_id
    }

    #[allow(dead_code)] // Used by workspace snapshot at M3 and by M5 browser-pane discrimination.
    pub fn kind(&self) -> PaneKind {
        PaneKind::Terminal
    }

    #[allow(dead_code)] // Used by tests and (M4 phase D) the D-Bus status service.
    pub fn status(&self) -> PaneStatus {
        self.status.get()
    }

    #[allow(dead_code)] // Used by tests and (M4 phase D) the D-Bus status service.
    pub fn detection(&self) -> Option<Detection> {
        self.detection.borrow().clone()
    }

    /// Drive a [`StatusEvent`] through the state machine and reflect the
    /// outcome in the wrapper frame's CSS classes.
    pub fn apply_status_event(&self, event: StatusEvent) {
        let old = self.status.get();
        let new = old.next(event);
        if old != new {
            self.status.set(new);
            self.apply_status_class();
            tracing::debug!(pane = %self.pane_id, ?old, ?new, "status transitioned");
        }
    }

    fn apply_status_class(&self) {
        let want = self.status.get().css_class();
        for cls in PaneStatus::all_css_classes() {
            self.container.remove_css_class(cls);
        }
        if !want.is_empty() {
            self.container.add_css_class(want);
        }
    }

    /// Clear `NeedsAttention` when the user focuses the pane.
    fn install_focus_clear(&self) {
        let me = self.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| {
            me.apply_status_event(StatusEvent::Focused);
        });
        self.terminal.add_controller(focus);
    }

    fn start_polling(&self, manifests: Vec<CompiledManifest>) {
        let terminal_weak = self.terminal.downgrade();
        let me = self.clone();
        glib::timeout_add_local(POLL_INTERVAL, move || {
            let Some(terminal) = terminal_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(fd) = vte_fd(&terminal) else {
                return glib::ControlFlow::Continue;
            };
            let current = detect::detect_from_fd(fd, &manifests);
            let was = me.detection.borrow().clone();
            if current != was {
                match (&was, &current) {
                    (None, Some(d)) => {
                        tracing::info!(pane = %me.pane_id, agent = %d.manifest_name, "agent detected");
                        me.apply_status_event(StatusEvent::AgentStarted);
                    }
                    (Some(_), None) => {
                        tracing::info!(pane = %me.pane_id, "agent gone");
                        me.apply_status_event(StatusEvent::AgentStopped);
                    }
                    (Some(prev), Some(now)) if prev.manifest_name != now.manifest_name => {
                        tracing::info!(
                            pane = %me.pane_id,
                            from = %prev.manifest_name,
                            to = %now.manifest_name,
                            "agent changed",
                        );
                        me.apply_status_event(StatusEvent::AgentStarted);
                    }
                    _ => {}
                }
                *me.detection.borrow_mut() = current;
            }
            glib::ControlFlow::Continue
        });
    }
}

impl Default for TerminalPane {
    fn default() -> Self {
        Self::new()
    }
}

/// Pull the underlying file descriptor from VTE's pty. Returns `None`
/// while the terminal is still spawning.
fn vte_fd(terminal: &Terminal) -> Option<RawFd> {
    let pty = terminal.pty()?;
    let fd: BorrowedFd<'_> = pty.fd();
    Some(fd.as_raw_fd())
}
