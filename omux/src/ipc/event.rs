//! Event types shared by omux and omux-hook.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookEventKind {
    /// Agent's Stop hook fired (the agent finished a turn or exited).
    Stop,
    /// Agent's Notification hook fired (input required, error, etc.).
    Notification,
    /// SessionStart hook fired (a new turn / new session beginning).
    SessionStart,
    /// Output-regex fallback matched.
    RegexFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEvent {
    pub kind: HookEventKind,
    pub pane_id: Uuid,
    /// Free-form payload passed through from the hook. Currently unused
    /// by the renderer but persisted to `agent_events` at M4 phase E so
    /// we can debug what each hook is reporting.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

impl HookEvent {
    #[allow(dead_code)] // Used in tests; main producer is omux-hook (separate binary).
    pub fn new(kind: HookEventKind, pane_id: Uuid) -> Self {
        Self {
            kind,
            pane_id,
            payload: None,
        }
    }

    #[allow(dead_code)] // Used in tests; main producer is omux-hook (separate binary).
    pub fn to_json_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn parse_json_line(line: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(line.trim())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_json_line() {
        let ev = HookEvent::new(HookEventKind::Stop, Uuid::from_u128(1));
        let line = ev.to_json_line();
        assert!(line.ends_with('\n'));
        let back = HookEvent::parse_json_line(&line).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn parses_kebab_case_kind() {
        let raw = r#"{"kind":"session-start","pane_id":"00000000-0000-0000-0000-000000000001"}"#;
        let ev = HookEvent::parse_json_line(raw).unwrap();
        assert_eq!(ev.kind, HookEventKind::SessionStart);
    }
}
