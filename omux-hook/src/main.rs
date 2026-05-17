//! omux-hook — tiny CLI helper invoked by agent harness hooks.
//!
//! Argv shape:
//!
//!     omux-hook <kind> --pane <uuid> [--payload <json>]
//!
//! where `<kind>` is one of `stop`, `notification`, `session-start`,
//! `regex-fallback`. The `--pane` UUID is read from the hook's
//! environment (`$CLAUDE_PANE_ID` is set by the rcfile snippet that
//! omux installs on first run).
//!
//! On success the helper exits 0 silently. If omux isn't running (no
//! one listening on the control socket), the event is appended to
//! `$XDG_RUNTIME_DIR/omux/pending-events.jsonl`. omux drains that file
//! on its next startup so no events are lost across app restarts.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum HookEventKind {
    Stop,
    Notification,
    SessionStart,
    RegexFallback,
}

impl HookEventKind {
    fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "stop" => Self::Stop,
            "notification" => Self::Notification,
            "session-start" | "sessionstart" => Self::SessionStart,
            "regex-fallback" | "regex" => Self::RegexFallback,
            other => anyhow::bail!("unknown event kind {other:?}"),
        })
    }
}

#[derive(Debug, Serialize)]
struct HookEvent {
    kind: HookEventKind,
    pane_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // First positional argument is the event kind.
    let kind_arg = args.first().ok_or_else(|| {
        anyhow::anyhow!(
            "missing event kind (expected: stop, notification, session-start, regex-fallback)"
        )
    })?;
    let kind = HookEventKind::parse(kind_arg)?;

    let mut pane_arg: Option<String> = None;
    let mut payload_arg: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--pane" => {
                pane_arg = args.get(i + 1).cloned();
                i += 2;
            }
            "--payload" => {
                payload_arg = args.get(i + 1).cloned();
                i += 2;
            }
            other => {
                eprintln!("omux-hook: unknown argument {other:?}");
                i += 1;
            }
        }
    }

    let pane_id = pane_arg
        .or_else(|| std::env::var("CLAUDE_PANE_ID").ok())
        .or_else(|| std::env::var("OMUX_PANE_ID").ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("no pane id (pass --pane <uuid> or set CLAUDE_PANE_ID)"))?;

    let payload = payload_arg.and_then(|s| serde_json::from_str(&s).ok());

    let event = HookEvent {
        kind,
        pane_id,
        payload,
    };

    let serialized = serde_json::to_string(&event).context("serialize event")?;
    let line = format!("{serialized}\n");

    let socket = socket_path();
    match try_send_to_socket(&socket, line.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Fall back: append to pending file. omux drains it on startup.
            eprintln!("omux-hook: socket send failed ({e}); buffering to pending events");
            append_pending(line.as_bytes())
        }
    }
}

fn try_send_to_socket(path: &PathBuf, bytes: &[u8]) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(path)
        .with_context(|| format!("connect to omux control socket at {}", path.display()))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

fn append_pending(bytes: &[u8]) -> anyhow::Result<()> {
    use std::fs::OpenOptions;
    let path = pending_events_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("OMUX_SOCKET") {
        return PathBuf::from(p);
    }
    runtime_root().join("control.sock")
}

fn pending_events_path() -> PathBuf {
    runtime_root().join("pending-events.jsonl")
}

fn runtime_root() -> PathBuf {
    let root = if let Ok(r) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(r)
    } else {
        // Fall back to /tmp/omux-<uid> for systems without XDG_RUNTIME_DIR.
        let uid = unsafe { libc::geteuid() };
        PathBuf::from(format!("/tmp/omux-{uid}"))
    };
    root.join("omux")
}
