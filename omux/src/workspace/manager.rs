//! Workspace lifecycle orchestrator.
//!
//! Owns the in-memory list of workspaces and coordinates the TOML config
//! store + SQLite state store. The sidebar UI talks to a `WorkspaceManager`;
//! the manager owns the persistence layer.
//!
//! This module is M3-phase-B; the UI wiring lands at phase C in the same
//! milestone but in a separate file (`ui::sidebar`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;

use super::config::WorkspaceConfig;
use super::paths;
use super::state::{StateRepo, WorkspaceRow};

pub struct WorkspaceManager {
    repo: StateRepo,
    workspaces_dir: PathBuf,
    workspaces: Vec<WorkspaceEntry>,
    active: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceEntry {
    pub config: WorkspaceConfig,
    pub last_opened: i64,
    pub display_order: i64,
}

impl WorkspaceManager {
    /// Open the production manager backed by XDG dirs.
    pub fn open() -> anyhow::Result<Self> {
        let workspaces_dir = paths::workspaces_dir()?;
        let db_path = paths::state_db_path()?;
        let repo = StateRepo::open(&db_path)?;
        Self::from_parts(repo, workspaces_dir)
    }

    /// Test/internal constructor: caller provides a repo and a directory.
    pub fn from_parts(repo: StateRepo, workspaces_dir: PathBuf) -> anyhow::Result<Self> {
        let configs = WorkspaceConfig::load_all_from_dir(&workspaces_dir)?;
        let rows = repo.list_workspaces()?;
        let active = repo.get_active_workspace()?;

        // Join TOML configs with SQLite rows by workspace name.
        let mut workspaces: Vec<WorkspaceEntry> = configs
            .into_iter()
            .map(|cfg| {
                let row = rows.iter().find(|r| r.name == cfg.name);
                WorkspaceEntry {
                    last_opened: row.map(|r| r.last_opened).unwrap_or(0),
                    display_order: row.map(|r| r.display_order).unwrap_or(cfg.order as i64),
                    config: cfg,
                }
            })
            .collect();
        workspaces.sort_by(|a, b| {
            b.config
                .pinned
                .cmp(&a.config.pinned)
                .then(a.display_order.cmp(&b.display_order))
                .then(a.config.name.cmp(&b.config.name))
        });

        Ok(Self {
            repo,
            workspaces_dir,
            workspaces,
            active,
        })
    }

    pub fn entries(&self) -> &[WorkspaceEntry] {
        &self.workspaces
    }

    pub fn active_workspace_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    #[allow(dead_code)] // Used by phase C sidebar UI.
    pub fn workspaces_dir(&self) -> &std::path::Path {
        &self.workspaces_dir
    }

    pub fn get(&self, name: &str) -> Option<&WorkspaceEntry> {
        self.workspaces.iter().find(|e| e.config.name == name)
    }

    /// Add or update a workspace and persist immediately.
    pub fn upsert(&mut self, cfg: WorkspaceConfig) -> anyhow::Result<()> {
        cfg.save_to_dir(&self.workspaces_dir)
            .with_context(|| format!("save workspace {:?}", cfg.name))?;

        let entry = WorkspaceEntry {
            last_opened: now_unix(),
            display_order: cfg.order as i64,
            config: cfg.clone(),
        };

        let row = WorkspaceRow {
            name: cfg.name.clone(),
            last_opened: entry.last_opened,
            display_order: entry.display_order,
            pinned: cfg.pinned,
        };
        self.repo.upsert_workspace(&row)?;

        if let Some(existing) = self
            .workspaces
            .iter_mut()
            .find(|e| e.config.name == cfg.name)
        {
            *existing = entry;
        } else {
            self.workspaces.push(entry);
        }
        Ok(())
    }

    /// Remove a workspace (TOML file + SQLite row).
    #[allow(dead_code)] // Used by phase C sidebar UI (delete action).
    pub fn delete(&mut self, name: &str) -> anyhow::Result<()> {
        let Some(entry) = self.workspaces.iter().find(|e| e.config.name == name) else {
            return Ok(());
        };
        WorkspaceConfig::delete_from_dir(&self.workspaces_dir, &entry.config.slug())?;
        self.repo.delete_workspace(name)?;
        self.workspaces.retain(|e| e.config.name != name);
        if self.active.as_deref() == Some(name) {
            self.active = None;
            self.repo.set_active_workspace(None)?;
        }
        Ok(())
    }

    /// Rename a workspace: writes the new TOML, removes the old, updates state.
    #[allow(dead_code)] // Used by phase C sidebar UI (rename action).
    pub fn rename(&mut self, old: &str, new: &str) -> anyhow::Result<()> {
        if old == new {
            return Ok(());
        }
        let mut cfg = self
            .workspaces
            .iter()
            .find(|e| e.config.name == old)
            .map(|e| e.config.clone())
            .ok_or_else(|| anyhow::anyhow!("no workspace named {old:?}"))?;
        let old_slug = cfg.slug();
        cfg.name = new.to_string();
        cfg.save_to_dir(&self.workspaces_dir)?;
        if cfg.slug() != old_slug {
            WorkspaceConfig::delete_from_dir(&self.workspaces_dir, &old_slug)?;
        }
        self.repo.rename_workspace(old, new)?;
        if let Some(e) = self.workspaces.iter_mut().find(|e| e.config.name == old) {
            e.config = cfg;
        }
        if self.active.as_deref() == Some(old) {
            self.active = Some(new.to_string());
            self.repo.set_active_workspace(Some(new))?;
        }
        Ok(())
    }

    #[allow(dead_code)] // Used by phase C sidebar UI (pin toggle).
    pub fn set_pinned(&mut self, name: &str, pinned: bool) -> anyhow::Result<()> {
        let Some(entry) = self.workspaces.iter_mut().find(|e| e.config.name == name) else {
            return Ok(());
        };
        entry.config.pinned = pinned;
        entry.config.save_to_dir(&self.workspaces_dir)?;
        let row = WorkspaceRow {
            name: name.to_string(),
            last_opened: entry.last_opened,
            display_order: entry.display_order,
            pinned,
        };
        self.repo.upsert_workspace(&row)?;
        Ok(())
    }

    #[allow(dead_code)] // Used by phase C sidebar UI (drag-drop reorder).
    pub fn reorder(&mut self, names_in_order: &[String]) -> anyhow::Result<()> {
        for (i, name) in names_in_order.iter().enumerate() {
            if let Some(entry) = self.workspaces.iter_mut().find(|e| &e.config.name == name) {
                entry.display_order = i as i64;
                entry.config.order = i as i32;
                let row = WorkspaceRow {
                    name: name.clone(),
                    last_opened: entry.last_opened,
                    display_order: entry.display_order,
                    pinned: entry.config.pinned,
                };
                self.repo.upsert_workspace(&row)?;
                entry.config.save_to_dir(&self.workspaces_dir)?;
            }
        }
        // Re-sort the in-memory list to reflect the new order.
        self.workspaces.sort_by(|a, b| {
            b.config
                .pinned
                .cmp(&a.config.pinned)
                .then(a.display_order.cmp(&b.display_order))
                .then(a.config.name.cmp(&b.config.name))
        });
        Ok(())
    }

    /// Read a generic UI-state key from the SQLite store.
    pub fn app_state_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        self.repo.app_state_get(key)
    }

    /// Write a generic UI-state key to the SQLite store.
    pub fn app_state_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.repo.app_state_set(key, value)
    }

    pub fn set_active(&mut self, name: Option<&str>) -> anyhow::Result<()> {
        self.active = name.map(|s| s.to_string());
        self.repo.set_active_workspace(name)?;
        if let Some(name) = name
            && let Some(entry) = self.workspaces.iter_mut().find(|e| e.config.name == name)
        {
            entry.last_opened = now_unix();
            let row = WorkspaceRow {
                name: name.to_string(),
                last_opened: entry.last_opened,
                display_order: entry.display_order,
                pinned: entry.config.pinned,
            };
            self.repo.upsert_workspace(&row)?;
        }
        Ok(())
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn manager_in(dir: &std::path::Path) -> WorkspaceManager {
        let repo = StateRepo::open_in_memory().unwrap();
        WorkspaceManager::from_parts(repo, dir.to_path_buf()).unwrap()
    }

    #[test]
    fn empty_manager() {
        let dir = tempdir().unwrap();
        let mgr = manager_in(dir.path());
        assert!(mgr.entries().is_empty());
        assert!(mgr.active_workspace_name().is_none());
    }

    #[test]
    fn upsert_delete_round_trip() {
        let dir = tempdir().unwrap();
        let mut mgr = manager_in(dir.path());
        mgr.upsert(WorkspaceConfig::new("alpha", "/a")).unwrap();
        mgr.upsert(WorkspaceConfig::new("beta", "/b")).unwrap();
        assert_eq!(mgr.entries().len(), 2);
        mgr.delete("alpha").unwrap();
        assert_eq!(mgr.entries().len(), 1);
        assert_eq!(mgr.entries()[0].config.name, "beta");
    }

    #[test]
    fn rename_moves_file() {
        let dir = tempdir().unwrap();
        let mut mgr = manager_in(dir.path());
        mgr.upsert(WorkspaceConfig::new("alpha", "/a")).unwrap();
        mgr.rename("alpha", "alphax").unwrap();
        assert_eq!(mgr.entries()[0].config.name, "alphax");
        // The old slug's TOML must be gone.
        assert!(WorkspaceConfig::load_from_dir(dir.path(), "alpha").is_err());
    }

    #[test]
    fn pin_brings_workspace_to_top() {
        let dir = tempdir().unwrap();
        let mut mgr = manager_in(dir.path());
        mgr.upsert(WorkspaceConfig::new("alpha", "/a")).unwrap();
        mgr.upsert(WorkspaceConfig::new("beta", "/b")).unwrap();
        mgr.set_pinned("beta", true).unwrap();
        // Re-sort happens lazily; force it through reorder of current order.
        let names: Vec<_> = mgr
            .entries()
            .iter()
            .map(|e| e.config.name.clone())
            .collect();
        mgr.reorder(&names).unwrap();
        assert_eq!(mgr.entries()[0].config.name, "beta");
    }

    #[test]
    fn active_workspace_persists_across_reopen() {
        let dir = tempdir().unwrap();
        let mut mgr = manager_in(dir.path());
        mgr.upsert(WorkspaceConfig::new("alpha", "/a")).unwrap();
        mgr.set_active(Some("alpha")).unwrap();
        // Build a new manager against the same dir / fresh repo to simulate restart.
        // Note: in-memory SQLite won't persist, so test with a file-backed db too.
        let db_path = dir.path().join("state.db");
        let repo = StateRepo::open(&db_path).unwrap();
        let mut mgr2 = WorkspaceManager::from_parts(repo, dir.path().to_path_buf()).unwrap();
        mgr2.set_active(Some("alpha")).unwrap();
        let repo2 = StateRepo::open(&db_path).unwrap();
        let mgr3 = WorkspaceManager::from_parts(repo2, dir.path().to_path_buf()).unwrap();
        assert_eq!(mgr3.active_workspace_name(), Some("alpha"));
    }
}
