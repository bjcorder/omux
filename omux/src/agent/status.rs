//! Per-pane status state machine (design §6).
//!
//! ```text
//!                  detect: agent process appears
//!   ┌──────┐ ────────────────────────────────────► ┌──────────┐
//!   │ idle │                                       │ running  │
//!   └──────┘ ◄─────────────────────────────────── └──────────┘
//!                  detect: agent process gone               │
//!                                                          │ hook/regex
//!                                                          ▼
//!                                                 ┌─────────────────┐
//!                         pane focus / typing ◄── │ needs-attention │
//!                                                 └─────────────────┘
//! ```

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneStatus {
    Idle,
    Running,
    NeedsAttention,
}

impl PaneStatus {
    /// CSS class name to apply to the pane wrapper for this status. Empty
    /// string means "no class for this state".
    pub fn css_class(self) -> &'static str {
        match self {
            PaneStatus::Idle => "",
            PaneStatus::Running => "agent-running",
            PaneStatus::NeedsAttention => "needs-attention",
        }
    }

    pub fn all_css_classes() -> &'static [&'static str] {
        &["agent-running", "needs-attention"]
    }
}

#[derive(Debug, Clone, Copy)]
pub enum StatusEvent {
    /// detect.rs observed a matching agent process appear in the PTY's
    /// foreground process group.
    AgentStarted,
    /// detect.rs observed the agent process disappear (foreground returned
    /// to the shell or another binary).
    AgentStopped,
    /// A hook callback fired (or the output-regex fallback matched).
    #[allow(dead_code)] // Raised by the D-Bus status service at M4 phase D.
    AttentionRequested,
    /// User focused the pane (debounced; clears NeedsAttention).
    Focused,
}

impl PaneStatus {
    pub fn next(self, event: StatusEvent) -> Self {
        match (self, event) {
            (_, StatusEvent::AgentStarted) => PaneStatus::Running,
            (_, StatusEvent::AgentStopped) => PaneStatus::Idle,
            (_, StatusEvent::AttentionRequested) => PaneStatus::NeedsAttention,
            (PaneStatus::NeedsAttention, StatusEvent::Focused) => PaneStatus::Running,
            (other, StatusEvent::Focused) => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_to_running_on_agent_started() {
        assert_eq!(
            PaneStatus::Idle.next(StatusEvent::AgentStarted),
            PaneStatus::Running
        );
    }

    #[test]
    fn running_to_needs_attention_on_request() {
        assert_eq!(
            PaneStatus::Running.next(StatusEvent::AttentionRequested),
            PaneStatus::NeedsAttention
        );
    }

    #[test]
    fn needs_attention_cleared_by_focus() {
        assert_eq!(
            PaneStatus::NeedsAttention.next(StatusEvent::Focused),
            PaneStatus::Running
        );
    }

    #[test]
    fn focus_does_not_change_idle_or_running() {
        assert_eq!(
            PaneStatus::Idle.next(StatusEvent::Focused),
            PaneStatus::Idle
        );
        assert_eq!(
            PaneStatus::Running.next(StatusEvent::Focused),
            PaneStatus::Running
        );
    }

    #[test]
    fn agent_stopped_returns_to_idle_even_when_pending_attention() {
        assert_eq!(
            PaneStatus::NeedsAttention.next(StatusEvent::AgentStopped),
            PaneStatus::Idle
        );
    }

    #[test]
    fn attention_can_fire_even_when_idle() {
        // Defensive: a hook event might arrive after the agent appears to
        // have exited (race window). We still surface attention.
        assert_eq!(
            PaneStatus::Idle.next(StatusEvent::AttentionRequested),
            PaneStatus::NeedsAttention
        );
    }
}
