//! Pane abstractions.
//!
//! At M1 the only kind is a terminal pane backed by VTE4. Browser panes
//! arrive at M5; split layouts at M2.

pub mod terminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    // Browser arrives at M5.
}
