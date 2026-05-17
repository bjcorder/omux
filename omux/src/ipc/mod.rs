//! Inter-process communication: the bridge that lets `omux-hook` (or any
//! external agent harness hook) deliver attention events to a running
//! omux app.
//!
//! The design.md §5.5 originally specified D-Bus. M4 ships a Unix
//! domain socket instead: lighter dependency footprint, integrates
//! cleanly with the glib main loop via `gio::SocketService`, and the
//! event surface is small (3 event kinds). Migration to D-Bus tracked
//! for post-v1 if cross-language hook authors request it.
//!
//! Event format: one JSON object per line, written to
//! `$XDG_RUNTIME_DIR/omux/control.sock`. See [`event::HookEvent`].

pub mod event;
pub mod paths;
pub mod socket_service;

pub use event::{HookEvent, HookEventKind};
pub use socket_service::SocketService;
