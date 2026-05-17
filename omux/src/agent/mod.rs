//! Agent detection, status, and (later milestones) notification plumbing.
//!
//! M4 lands in three phases:
//!
//! * **Phase A** (this iteration): manifests + status state machine.
//! * **Phase B** (this iteration): process detection — read the VTE pty's
//!   foreground process group and match against manifest patterns.
//! * **Phase C** (this iteration): apply CSS classes to the pane wrapper
//!   so a pane in `NeedsAttention` shows a ring.
//! * **Phase D** (next iteration): D-Bus service in omux + the `omux-hook`
//!   helper binary + first-run install into `~/.claude/settings.json`.
//! * **Phase E** (after that): PTY output-regex fallback for harnesses
//!   without hooks (per design §3.3 manifest `fallback` block).

pub mod detect;
pub mod manifest;
pub mod status;
