//! Server side of the omux control socket.
//!
//! Listens on `$XDG_RUNTIME_DIR/omux/control.sock`, parses each
//! line-delimited [`HookEvent`], and dispatches it via a callback into
//! the gtk main thread.

use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use super::event::HookEvent;
use super::paths;

pub type EventHandler = Rc<dyn Fn(HookEvent) + 'static>;

pub struct SocketService {
    #[allow(dead_code)] // Held to keep the service alive; stopped via Drop / stop().
    inner: gio::SocketService,
    socket_path: PathBuf,
}

impl SocketService {
    /// Start listening. The provided handler is invoked on the glib main
    /// thread for every parsed event.
    pub fn start(handler: EventHandler) -> anyhow::Result<Self> {
        let socket_path = paths::socket_path();
        ensure_parent_dir(&socket_path)?;
        // Remove any stale socket file from a previous run.
        let _ = std::fs::remove_file(&socket_path);

        let service = gio::SocketService::new();
        let address = gio::UnixSocketAddress::new(&socket_path);
        service.add_address(
            &address,
            gio::SocketType::Stream,
            gio::SocketProtocol::Default,
            glib::Object::NONE,
        )?;

        let handler_for_signal = handler.clone();
        service.connect_incoming(move |_service, connection, _source| {
            handle_connection(connection.clone(), handler_for_signal.clone());
            false // false ⇒ allow other handlers to also run
        });
        service.start();
        tracing::info!(socket = %socket_path.display(), "control socket listening");

        Ok(Self {
            inner: service,
            socket_path,
        })
    }

    #[allow(dead_code)] // Wired up at M6 polish (clean shutdown ordering).
    pub fn stop(&self) {
        self.inner.stop();
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Drain `$XDG_RUNTIME_DIR/omux/pending-events.jsonl` into the
    /// handler. Called once at startup to replay events that the hook
    /// helper wrote while omux wasn't running.
    pub fn drain_pending(handler: &EventHandler) {
        let path = paths::pending_events_path();
        if !path.exists() {
            return;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not read pending events");
                return;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match HookEvent::parse_json_line(line) {
                Ok(ev) => handler(ev),
                Err(e) => tracing::warn!(error = %e, line, "drop malformed pending event"),
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

fn handle_connection(connection: gio::SocketConnection, handler: EventHandler) {
    let input = connection.input_stream();
    let reader = gio::DataInputStream::new(&input);
    read_next_line(reader, connection, handler);
}

fn read_next_line(
    reader: gio::DataInputStream,
    connection: gio::SocketConnection,
    handler: EventHandler,
) {
    let reader_for_cb = reader.clone();
    let connection_for_cb = connection.clone();
    let handler_for_cb = handler.clone();
    reader.read_line_async(
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(Some(bytes)) => {
                let line = String::from_utf8_lossy(&bytes).to_string();
                match HookEvent::parse_json_line(&line) {
                    Ok(ev) => handler_for_cb(ev),
                    Err(e) => tracing::warn!(error = %e, line, "ignored malformed event"),
                }
                // Keep reading; a single hook may write multiple events.
                read_next_line(reader_for_cb, connection_for_cb, handler_for_cb);
            }
            Ok(None) => {
                // EOF — client closed. Done with this connection.
                let _ = connection_for_cb.close(gio::Cancellable::NONE);
            }
            Err(e) => {
                tracing::debug!(error = %e, "socket read ended");
                let _ = connection_for_cb.close(gio::Cancellable::NONE);
            }
        },
    );
}

fn ensure_parent_dir(socket_path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
