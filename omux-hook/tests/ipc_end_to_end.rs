//! End-to-end test for the omux-hook → omux IPC contract.
//!
//! Doesn't depend on a real omux GUI or a GTK display: we stand up a
//! tiny Unix-socket listener in the test process, point omux-hook at it
//! via `$OMUX_SOCKET`, and verify the JSON event lands on the wire with
//! the shape omux expects.
//!
//! This locks down the wire contract: if either side changes the
//! JSON schema (event kind names, pane id format, required fields)
//! without keeping the other in sync, this test fails.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

const HOOK_BIN: &str = env!("CARGO_BIN_EXE_omux-hook");

/// Schema the test asserts against. Mirrors `omux::ipc::event::HookEvent`
/// (kept duplicated here so a future schema change has to be made
/// deliberately in two places, surfacing the break).
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    kind: String,
    pane_id: String,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

struct TestServer {
    socket_path: PathBuf,
    listener: UnixListener,
}

impl TestServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("control.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind unix socket");
        // Hold the dir alive for the lifetime of this server by leaking the handle.
        std::mem::forget(dir);
        Self {
            socket_path,
            listener,
        }
    }

    /// Block (with a 5s deadline) on a single connection, read one line,
    /// return it.
    fn accept_one_event(&self) -> String {
        self.listener.set_nonblocking(false).expect("set blocking");
        // Bound the wait so a broken test doesn't hang forever.
        let listener = self.listener.try_clone().expect("clone listener");
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            if let Ok((stream, _addr)) = listener.accept() {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                let _ = tx.send(line);
            }
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("hook event did not arrive within 5s")
    }
}

fn run_hook<I, S>(env: &[(&str, &str)], args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new(HOOK_BIN);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn omux-hook")
}

#[test]
fn stop_event_with_explicit_pane_flag_round_trips() {
    let server = TestServer::start();
    let pane_id = "11111111-2222-3333-4444-555555555555";

    let socket = server.socket_path.to_string_lossy().to_string();
    let env = [("OMUX_SOCKET", socket.as_str())];

    // Spawn the helper. Don't wait synchronously — it'll race the accept.
    let server_for_thread = server.socket_path.clone();
    let event_line = thread::spawn(move || {
        // The listener already exists at this point so the connect will succeed.
        let listener = UnixListener::bind(&server_for_thread).err();
        // We can't re-bind (already bound), so reuse the existing server.
        // Just block on the listener up the stack.
        listener
    });
    drop(event_line);

    let out = run_hook(&env, ["stop", "--pane", pane_id]);
    assert!(
        out.status.success(),
        "omux-hook exited non-zero: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let line = server.accept_one_event();
    let event: WireEvent = serde_json::from_str(line.trim()).expect("parse JSON line");
    assert_eq!(event.kind, "stop");
    assert_eq!(event.pane_id, pane_id);
    assert!(event.payload.is_none());
}

#[test]
fn notification_event_via_env_var_round_trips() {
    let server = TestServer::start();
    let pane_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    let socket = server.socket_path.to_string_lossy().to_string();
    let out = run_hook(
        &[("OMUX_SOCKET", socket.as_str()), ("OMUX_PANE_ID", pane_id)],
        ["notification"],
    );
    assert!(out.status.success());

    let line = server.accept_one_event();
    let event: WireEvent = serde_json::from_str(line.trim()).expect("parse JSON");
    assert_eq!(event.kind, "notification");
    assert_eq!(event.pane_id, pane_id);
}

#[test]
fn session_start_kind_serializes_as_kebab_case() {
    let server = TestServer::start();
    let pane_id = "00000000-0000-0000-0000-000000000001";

    let socket = server.socket_path.to_string_lossy().to_string();
    let out = run_hook(
        &[("OMUX_SOCKET", socket.as_str()), ("OMUX_PANE_ID", pane_id)],
        ["session-start"],
    );
    assert!(out.status.success());

    let line = server.accept_one_event();
    let event: WireEvent = serde_json::from_str(line.trim()).expect("parse JSON");
    assert_eq!(
        event.kind, "session-start",
        "kind must serialize as kebab-case to match omux's HookEventKind"
    );
}

#[test]
fn payload_is_passed_through_when_valid_json() {
    let server = TestServer::start();
    let pane_id = "12345678-90ab-cdef-1234-567890abcdef";

    let socket = server.socket_path.to_string_lossy().to_string();
    let payload = r#"{"reason":"turn-end","count":3}"#;
    let out = run_hook(
        &[("OMUX_SOCKET", socket.as_str()), ("OMUX_PANE_ID", pane_id)],
        ["stop", "--payload", payload],
    );
    assert!(
        out.status.success(),
        "stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    let line = server.accept_one_event();
    let event: WireEvent = serde_json::from_str(line.trim()).expect("parse JSON");
    let p = event.payload.expect("payload should be present");
    assert_eq!(p["reason"], "turn-end");
    assert_eq!(p["count"], 3);
}

#[test]
fn missing_kind_argument_is_rejected_with_nonzero_exit() {
    // The helper should refuse to write *anything* to the socket without a kind.
    // No server here — we just check the exit code + stderr.
    let out = run_hook(&[("OMUX_PANE_ID", "1")], Vec::<&str>::new());
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("missing event kind"),
        "expected helpful stderr message, got: {:?}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn missing_pane_id_is_rejected_with_nonzero_exit() {
    // No env, no --pane → should error out clearly without writing to the socket.
    let out = run_hook(&[("OMUX_SOCKET", "/tmp/omux-test-nowhere.sock")], ["stop"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no pane id"),
        "expected pane-id error, got: {:?}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn unknown_kind_is_rejected() {
    let out = run_hook(
        &[
            ("OMUX_SOCKET", "/tmp/omux-test-nowhere.sock"),
            ("OMUX_PANE_ID", "1"),
        ],
        ["definitely-not-a-kind"],
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown event kind"),
        "expected unknown-kind error, got: {:?}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn offline_omux_writes_to_pending_jsonl() {
    // Point at a socket path that does NOT exist. The helper should
    // fall back to appending to pending-events.jsonl in the same
    // runtime root we provide via XDG_RUNTIME_DIR.
    let dir = tempfile::tempdir().expect("tempdir");
    let xdg = dir.path().to_string_lossy().to_string();
    let nonexistent = dir.path().join("omux").join("control.sock");
    let nonexistent_str = nonexistent.to_string_lossy().to_string();

    let out = run_hook(
        &[
            ("XDG_RUNTIME_DIR", xdg.as_str()),
            ("OMUX_SOCKET", nonexistent_str.as_str()),
            ("OMUX_PANE_ID", "ffffffff-ffff-ffff-ffff-ffffffffffff"),
        ],
        ["stop"],
    );
    // The helper exits success after buffering.
    assert!(
        out.status.success(),
        "helper should not fail when omux is offline; stderr={:?}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("buffering"),
        "expected a hint that the event was buffered, got: {:?}",
        String::from_utf8_lossy(&out.stderr),
    );

    let pending = dir.path().join("omux").join("pending-events.jsonl");
    assert!(pending.exists(), "pending-events.jsonl should exist");
    let body = std::fs::read_to_string(&pending).expect("read pending");
    let line = body.lines().next().expect("at least one event line");
    let event: WireEvent = serde_json::from_str(line).expect("parse JSON");
    assert_eq!(event.kind, "stop");
    assert_eq!(event.pane_id, "ffffffff-ffff-ffff-ffff-ffffffffffff");
}
