//! XDG paths for omux.

use std::path::PathBuf;

use directories::ProjectDirs;

const QUALIFIER: &str = "org";
const ORG: &str = "omux";
const APP: &str = "omux";

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORG, APP)
}

/// `$XDG_CONFIG_HOME/omux/` (or platform equivalent). Created if missing.
pub fn config_dir() -> anyhow::Result<PathBuf> {
    let d = dirs()
        .ok_or_else(|| anyhow::anyhow!("could not resolve XDG project dirs"))?
        .config_dir()
        .to_path_buf();
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// `$XDG_STATE_HOME/omux/` (or platform equivalent). Created if missing.
pub fn state_dir() -> anyhow::Result<PathBuf> {
    let d = dirs()
        .ok_or_else(|| anyhow::anyhow!("could not resolve XDG project dirs"))?
        .state_dir()
        .ok_or_else(|| anyhow::anyhow!("platform has no XDG state dir"))?
        .to_path_buf();
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// `$XDG_CONFIG_HOME/omux/workspaces/`. Created if missing.
pub fn workspaces_dir() -> anyhow::Result<PathBuf> {
    let d = config_dir()?.join("workspaces");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// `$XDG_STATE_HOME/omux/state.db`.
pub fn state_db_path() -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("state.db"))
}
