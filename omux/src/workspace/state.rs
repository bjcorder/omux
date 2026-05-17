//! SQLite-backed runtime state (design §3.2).
//!
//! At M3 we only track the workspace-level rows + a tiny `app_state`
//! key/value table for things like "last active workspace". The pane /
//! agent-event tables land at M4.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

pub struct StateRepo {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRow {
    pub name: String,
    pub last_opened: i64,
    pub display_order: i64,
    pub pinned: bool,
}

impl StateRepo {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> anyhow::Result<()> {
        // user_version-based migrations so we can add tables at M4 without
        // a destructive reset.
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS workspaces (
                    name          TEXT PRIMARY KEY,
                    last_opened   INTEGER NOT NULL DEFAULT 0,
                    display_order INTEGER NOT NULL DEFAULT 0,
                    pinned        INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS app_state (
                    key   TEXT PRIMARY KEY,
                    value TEXT
                );
                PRAGMA user_version = 1;
                ",
            )?;
        }
        Ok(())
    }

    pub fn list_workspaces(&self) -> anyhow::Result<Vec<WorkspaceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, last_opened, display_order, pinned
             FROM workspaces
             ORDER BY pinned DESC, display_order ASC, name ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WorkspaceRow {
                    name: r.get(0)?,
                    last_opened: r.get(1)?,
                    display_order: r.get(2)?,
                    pinned: r.get::<_, i64>(3)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn upsert_workspace(&self, row: &WorkspaceRow) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO workspaces (name, last_opened, display_order, pinned)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET
                last_opened   = excluded.last_opened,
                display_order = excluded.display_order,
                pinned        = excluded.pinned",
            params![
                row.name,
                row.last_opened,
                row.display_order,
                row.pinned as i64
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)] // Used by WorkspaceManager::delete (phase C UI).
    pub fn delete_workspace(&self, name: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM workspaces WHERE name = ?1", params![name])?;
        Ok(())
    }

    #[allow(dead_code)] // Used by WorkspaceManager::rename (phase C UI).
    pub fn rename_workspace(&self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE workspaces SET name = ?1 WHERE name = ?2",
            params![new_name, old_name],
        )?;
        Ok(())
    }

    pub fn get_active_workspace(&self) -> anyhow::Result<Option<String>> {
        let v = self
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE key = 'active_workspace'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(v)
    }

    pub fn set_active_workspace(&self, name: Option<&str>) -> anyhow::Result<()> {
        match name {
            Some(n) => {
                self.conn.execute(
                    "INSERT INTO app_state (key, value) VALUES ('active_workspace', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![n],
                )?;
            }
            None => {
                self.conn
                    .execute("DELETE FROM app_state WHERE key = 'active_workspace'", [])?;
            }
        }
        Ok(())
    }

    /// Generic key/value store on the `app_state` table. Used for UI
    /// state that doesn't deserve its own column (sidebar width, last
    /// selected pane, etc).
    pub fn app_state_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let v = self
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(v)
    }

    pub fn app_state_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, order: i64, pinned: bool) -> WorkspaceRow {
        WorkspaceRow {
            name: name.into(),
            last_opened: 0,
            display_order: order,
            pinned,
        }
    }

    #[test]
    fn upsert_and_list_sorts_pinned_first() {
        let repo = StateRepo::open_in_memory().unwrap();
        repo.upsert_workspace(&sample("alpha", 1, false)).unwrap();
        repo.upsert_workspace(&sample("beta", 2, true)).unwrap();
        repo.upsert_workspace(&sample("gamma", 0, true)).unwrap();
        let rows = repo.list_workspaces().unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["gamma", "beta", "alpha"]);
    }

    #[test]
    fn upsert_updates_existing_row() {
        let repo = StateRepo::open_in_memory().unwrap();
        repo.upsert_workspace(&sample("alpha", 1, false)).unwrap();
        repo.upsert_workspace(&sample("alpha", 5, true)).unwrap();
        let rows = repo.list_workspaces().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_order, 5);
        assert!(rows[0].pinned);
    }

    #[test]
    fn rename_and_delete() {
        let repo = StateRepo::open_in_memory().unwrap();
        repo.upsert_workspace(&sample("alpha", 1, false)).unwrap();
        repo.rename_workspace("alpha", "alphax").unwrap();
        let rows = repo.list_workspaces().unwrap();
        assert_eq!(rows[0].name, "alphax");
        repo.delete_workspace("alphax").unwrap();
        assert!(repo.list_workspaces().unwrap().is_empty());
    }

    #[test]
    fn active_workspace_round_trip() {
        let repo = StateRepo::open_in_memory().unwrap();
        assert!(repo.get_active_workspace().unwrap().is_none());
        repo.set_active_workspace(Some("alpha")).unwrap();
        assert_eq!(
            repo.get_active_workspace().unwrap().as_deref(),
            Some("alpha")
        );
        repo.set_active_workspace(None).unwrap();
        assert!(repo.get_active_workspace().unwrap().is_none());
    }
}
