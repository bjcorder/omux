//! Filesystem paths used by the omux ↔ omux-hook channel.
//!
//! Both binaries link to this module so the contract stays in one place.

use std::path::PathBuf;

/// Where omux listens for hook events. Honors `$OMUX_SOCKET` for tests,
/// otherwise `$XDG_RUNTIME_DIR/omux/control.sock`, otherwise
/// `/tmp/omux-<uid>/control.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("OMUX_SOCKET") {
        return PathBuf::from(p);
    }
    runtime_root().join("control.sock")
}

/// Where omux-hook drops events when omux isn't running. Drained on
/// the next omux startup.
pub fn pending_events_path() -> PathBuf {
    runtime_root().join("pending-events.jsonl")
}

fn runtime_root() -> PathBuf {
    let root = if let Ok(r) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(r)
    } else {
        let uid = unsafe { libc::geteuid() };
        PathBuf::from(format!("/tmp/omux-{uid}"))
    };
    root.join("omux")
}
