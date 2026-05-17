//! Workspaces: durable named layouts.
//!
//! Two persistence stores per design §3:
//!
//! * **TOML configs** at `$XDG_CONFIG_HOME/omux/workspaces/<slug>.toml` —
//!   the user-readable definition of a workspace (name, root folder,
//!   pinned flag, layout snapshot).
//! * **SQLite state** at `$XDG_STATE_HOME/omux/state.db` — runtime
//!   metadata: display order, last-opened timestamp, the current active
//!   workspace.
//!
//! M3 ships the data layer + manager + sidebar. Per-pane agent status
//! (also in design §3.2) lands at M4.

pub mod config;
pub mod manager;
pub mod paths;
pub mod snapshot;
pub mod state;

#[allow(unused_imports)]
pub use config::WorkspaceConfig;
#[allow(unused_imports)]
pub use manager::WorkspaceManager;
#[allow(unused_imports)]
pub use snapshot::{LayoutNode, Orientation as SnapshotOrientation};
