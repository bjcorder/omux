//! Top-level GTK4/Adwaita widgets for omux.
//!
//! At M3 this module hosts the sidebar + content split. M4 will add
//! pane-ring + tab/workspace badge styling; M6 will add right-click
//! context menus and animated collapse.

pub mod shell;
pub mod sidebar;

pub use shell::AppShell;
