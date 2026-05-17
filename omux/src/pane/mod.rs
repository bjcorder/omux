//! Pane abstractions.
//!
//! At M1 the only kind is a terminal pane backed by VTE4. Browser panes
//! arrive at M5; split layouts at M2.

pub mod terminal;
pub mod tree;

#[allow(dead_code)] // Variants land as features arrive (browser pane at M5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    // Browser arrives at M5.
}
