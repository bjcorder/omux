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
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{Format, PtyFlags, Terminal};

use crate::agent::detect::{self, Detection};
use crate::agent::manifest::CompiledManifest;
use crate::agent::status::{PaneStatus, StatusEvent};

use super::PaneKind;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Minimum delay between two consecutive output-regex AttentionRequested
/// events. Without this, a steady prompt-tail would fire on every poll.
const REGEX_DEBOUNCE: Duration = Duration::from_secs(2);
/// How many bottom rows to scan for the needs-attention regex fallback.
const REGEX_SCAN_ROWS: i64 = 6;

#[derive(Clone)]
pub struct TerminalPane {
    container: gtk::Frame,
    terminal: Terminal,
    pane_id: Uuid,
    status: Rc<Cell<PaneStatus>>,
    detection: Rc<RefCell<Option<Detection>>>,
    /// Last text that matched a needs-attention regex (used to dedupe
    /// repeated firings on the same prompt).
    last_match_text: Rc<RefCell<Option<String>>>,
    /// Last time we fired an AttentionRequested event from the regex
    /// fallback (used for [`REGEX_DEBOUNCE`]).
    last_match_at: Rc<Cell<Option<Instant>>>,
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
            last_match_text: Rc::new(RefCell::new(None)),
            last_match_at: Rc::new(Cell::new(None)),
        };

        me.install_focus_clear();
        me.install_context_menu();
        if !manifests.is_empty() {
            me.start_polling(manifests.to_vec());
        }
        me.apply_status_class();

        me
    }

    /// Build a `gio::Menu` of pane actions and bind it to the terminal
    /// widget as its context menu. Action handlers are installed on a
    /// `term.*` action group scoped to this pane's terminal widget; the
    /// shell layer separately installs `term.split-h` / `term.split-v` /
    /// `term.new-tab` / `term.close-tab` on the window so the shortcut
    /// path stays unified.
    fn install_context_menu(&self) {
        let menu = gio::Menu::new();
        let edit_section = gio::Menu::new();
        edit_section.append(Some("Copy"), Some("term.copy"));
        edit_section.append(Some("Paste"), Some("term.paste"));
        edit_section.append(Some("Clear"), Some("term.clear"));
        menu.append_section(None, &edit_section);

        let pane_section = gio::Menu::new();
        pane_section.append(Some("Split horizontally"), Some("win.split-h"));
        pane_section.append(Some("Split vertically"), Some("win.split-v"));
        pane_section.append(Some("New tab"), Some("win.new-tab"));
        pane_section.append(Some("Close tab"), Some("win.close-tab"));
        menu.append_section(None, &pane_section);

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        self.terminal.set_context_menu(Some(&popover));

        // Terminal-local actions (copy/paste/clear).
        let actions = gio::SimpleActionGroup::new();

        let term = self.terminal.clone();
        let copy = gio::SimpleAction::new("copy", None);
        copy.connect_activate(move |_, _| {
            term.copy_clipboard_format(Format::Text);
        });
        actions.add_action(&copy);

        let term = self.terminal.clone();
        let paste = gio::SimpleAction::new("paste", None);
        paste.connect_activate(move |_, _| {
            term.paste_clipboard();
        });
        actions.add_action(&paste);

        let term = self.terminal.clone();
        let clear = gio::SimpleAction::new("clear", None);
        clear.connect_activate(move |_, _| {
            term.reset(true, true);
        });
        actions.add_action(&clear);

        self.terminal.insert_action_group("term", Some(&actions));
    }

    /// Copy the current terminal selection (or visible buffer) to the
    /// system clipboard. Bound to `Ctrl+Shift+C` from the shell layer.
    pub fn copy_selection(&self) {
        self.terminal.copy_clipboard_format(Format::Text);
    }

    /// Paste from the system clipboard into the terminal. Bound to
    /// `Ctrl+Shift+V` from the shell layer.
    pub fn paste_clipboard(&self) {
        self.terminal.paste_clipboard();
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
                        me.last_match_text.borrow_mut().take();
                        me.last_match_at.set(None);
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
                *me.detection.borrow_mut() = current.clone();
            }

            // PTY output regex fallback (M4 phase E). For agents without
            // hook integration (and as a safety net for those with hooks),
            // scan the last few rows for any needs-attention pattern.
            if let Some(d) = current.as_ref()
                && me.status.get() != PaneStatus::NeedsAttention
                && let Some(manifest) = manifests.iter().find(|m| m.name == d.manifest_name)
                && !manifest.needs_attention_patterns.is_empty()
                && let Some(text) = read_recent_text(&terminal, REGEX_SCAN_ROWS)
                && manifest
                    .needs_attention_patterns
                    .iter()
                    .any(|r| r.is_match(&text))
                && me.should_fire_regex_attention(&text)
            {
                tracing::info!(
                    pane = %me.pane_id,
                    agent = %d.manifest_name,
                    "needs-attention regex matched",
                );
                me.apply_status_event(StatusEvent::AttentionRequested);
                *me.last_match_text.borrow_mut() = Some(text);
                me.last_match_at.set(Some(Instant::now()));
            }

            glib::ControlFlow::Continue
        });
    }

    fn should_fire_regex_attention(&self, text: &str) -> bool {
        if let Some(last_text) = self.last_match_text.borrow().as_deref()
            && last_text == text
        {
            return false;
        }
        if let Some(when) = self.last_match_at.get()
            && when.elapsed() < REGEX_DEBOUNCE
        {
            return false;
        }
        true
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

/// Grab the last `rows` rows of visible text from the terminal as a
/// plain string. Used by the regex-fallback agent attention detector.
fn read_recent_text(terminal: &Terminal, rows: i64) -> Option<String> {
    let (_, cursor_row) = terminal.cursor_position();
    let start_row = (cursor_row - rows).max(0);
    let (maybe, _len) = terminal.text_range_format(Format::Text, start_row, 0, cursor_row, -1);
    let s = maybe?.to_string();
    if s.is_empty() { None } else { Some(s) }
}
