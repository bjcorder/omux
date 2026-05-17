//! TOML schema for a workspace definition (design §3.1).
//!
//! One file per workspace at `$XDG_CONFIG_HOME/omux/workspaces/<slug>.toml`.
//!
//! Filesystem access is constrained two ways:
//!
//! 1. The slug must match its own canonical [`slugify`] form (rejects
//!    `..`, slashes, etc.).
//! 2. After building `<dir>/<slug>.toml`, only the parent equality is
//!    accepted — the computed path's parent must be exactly `dir`.
//!
//! Reads/writes go through GLib's `gio::File` API rather than `std::fs::*`.
//! gio is already in the dependency graph (via `gtk4`) and gives us
//! `load_contents` / `replace_contents` with atomic-write semantics.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::snapshot::LayoutNode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    pub name: String,
    pub root_folder: PathBuf,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub order: i32,
    pub layout: Option<LayoutNode>,
}

impl WorkspaceConfig {
    pub fn new(name: impl Into<String>, root_folder: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root_folder: root_folder.into(),
            pinned: false,
            order: 0,
            layout: Some(LayoutNode::single_leaf()),
        }
    }

    /// Filesystem-safe slug derived from the workspace name. Used as the
    /// TOML filename so renames produce a clean file move.
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }

    /// Load `<dir>/<slug>.toml`.
    pub fn load_from_dir(dir: &Path, slug: &str) -> anyhow::Result<Self> {
        let text = safe_io::read(dir, slug)?;
        Ok(toml::from_str(&text)?)
    }

    /// Atomically write `<dir>/<self.slug()>.toml`.
    pub fn save_to_dir(&self, dir: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        safe_io::write(dir, &self.slug(), text.as_bytes())
    }

    /// Remove `<dir>/<slug>.toml`. No-op if the file is already gone.
    #[allow(dead_code)] // Used by WorkspaceManager::{delete,rename} (phase C UI).
    pub fn delete_from_dir(dir: &Path, slug: &str) -> anyhow::Result<()> {
        safe_io::delete(dir, slug)
    }

    /// Enumerate `<dir>/*.toml` and load each.
    pub fn load_all_from_dir(dir: &Path) -> anyhow::Result<Vec<Self>> {
        let slugs = safe_io::list_slugs(dir)?;
        let mut out = Vec::new();
        for slug in slugs {
            match Self::load_from_dir(dir, &slug) {
                Ok(cfg) => out.push(cfg),
                Err(e) => tracing::warn!(%slug, error = %e, "failed to load workspace"),
            }
        }
        Ok(out)
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("workspace");
    }
    out
}

/// All filesystem I/O for workspace config files. Routes through
/// `gio::File` so we don't touch `std::fs::*` directly for these paths.
mod safe_io {
    use std::path::{Path, PathBuf};

    use gtk4::gio;
    use gtk4::glib;
    use gtk4::prelude::*;

    use super::slugify;

    fn validated_path(dir: &Path, slug: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(!slug.is_empty(), "workspace slug is empty");
        let canonical = slugify(slug);
        anyhow::ensure!(
            canonical == slug,
            "rejected workspace slug {slug:?}: canonical form would be {canonical:?}",
        );
        let path = dir.join(format!("{slug}.toml"));
        anyhow::ensure!(
            path.parent() == Some(dir),
            "computed path's parent is not the base dir"
        );
        Ok(path)
    }

    pub fn read(dir: &Path, slug: &str) -> anyhow::Result<String> {
        let path = validated_path(dir, slug)?;
        let file = gio::File::for_path(&path);
        let (bytes, _etag) = file
            .load_contents(gio::Cancellable::NONE)
            .map_err(map_glib_err)?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    pub fn write(dir: &Path, slug: &str, bytes: &[u8]) -> anyhow::Result<()> {
        ensure_dir(dir)?;
        let path = validated_path(dir, slug)?;
        let file = gio::File::for_path(&path);
        file.replace_contents(
            bytes,
            None,
            false,
            gio::FileCreateFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map_err(map_glib_err)?;
        Ok(())
    }

    pub fn delete(dir: &Path, slug: &str) -> anyhow::Result<()> {
        let path = validated_path(dir, slug)?;
        let file = gio::File::for_path(&path);
        match file.delete(gio::Cancellable::NONE) {
            Ok(()) => Ok(()),
            Err(e) if e.matches(gio::IOErrorEnum::NotFound) => Ok(()),
            Err(e) => Err(map_glib_err(e)),
        }
    }

    pub fn list_slugs(dir: &Path) -> anyhow::Result<Vec<String>> {
        let file = gio::File::for_path(dir);
        if !file.query_exists(gio::Cancellable::NONE) {
            return Ok(Vec::new());
        }
        let enumerator = file
            .enumerate_children(
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .map_err(map_glib_err)?;

        let mut out = Vec::new();
        loop {
            match enumerator.next_file(gio::Cancellable::NONE) {
                Ok(Some(info)) => {
                    if info.file_type() != gio::FileType::Regular {
                        continue;
                    }
                    let name = info.name();
                    let name_str = name.to_string_lossy();
                    let Some(stem) = name_str.strip_suffix(".toml") else {
                        continue;
                    };
                    if slugify(stem) != stem {
                        tracing::warn!(file = %name_str, "skipping non-canonical workspace filename");
                        continue;
                    }
                    out.push(stem.to_string());
                }
                Ok(None) => break,
                Err(e) => return Err(map_glib_err(e)),
            }
        }
        Ok(out)
    }

    fn ensure_dir(dir: &Path) -> anyhow::Result<()> {
        let file = gio::File::for_path(dir);
        match file.make_directory_with_parents(gio::Cancellable::NONE) {
            Ok(()) => Ok(()),
            Err(e) if e.matches(gio::IOErrorEnum::Exists) => Ok(()),
            Err(e) => Err(map_glib_err(e)),
        }
    }

    fn map_glib_err(e: glib::Error) -> anyhow::Error {
        anyhow::anyhow!("{e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gio;
    use gtk4::prelude::*;
    use tempfile::tempdir;

    fn ensure_gio_init() {
        // gio basics work without gtk_init, but in some setups GLib may
        // need to be touched once. This is a cheap no-op in normal runs.
        let _ = gtk4::glib::MainContext::default();
    }

    #[test]
    fn slug_normalizes_whitespace_and_punctuation() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("__a__b__"), "a-b");
        assert_eq!(slugify("   "), "workspace");
    }

    #[test]
    fn rejects_non_canonical_slugs() {
        ensure_gio_init();
        let dir = tempdir().unwrap();
        let bad_slugs = ["../etc/passwd", "has space", "", "x/y", "Capital"];
        for slug in bad_slugs {
            assert!(
                WorkspaceConfig::load_from_dir(dir.path(), slug).is_err(),
                "expected slug {slug:?} to be rejected",
            );
        }
    }

    #[test]
    fn round_trip_through_dir() {
        ensure_gio_init();
        let dir = tempdir().unwrap();
        let cfg = WorkspaceConfig::new("omux dev", "/tmp/omux");
        cfg.save_to_dir(dir.path()).unwrap();
        let loaded = WorkspaceConfig::load_from_dir(dir.path(), &cfg.slug()).unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn load_all_skips_non_canonical_files() {
        ensure_gio_init();
        let dir = tempdir().unwrap();
        let cfg = WorkspaceConfig::new("alpha", "/a");
        cfg.save_to_dir(dir.path()).unwrap();
        // Drop a non-TOML and a non-canonical-named TOML next to it. We
        // write these via gio too so we don't introduce stray std::fs
        // file-creation calls.
        let readme = gio::File::for_path(dir.path().join("readme.md"));
        readme
            .replace_contents(
                b"hi",
                None,
                false,
                gio::FileCreateFlags::NONE,
                gio::Cancellable::NONE,
            )
            .unwrap();
        let weird = gio::File::for_path(dir.path().join("Weird Name.toml"));
        weird
            .replace_contents(
                b"name = \"x\"",
                None,
                false,
                gio::FileCreateFlags::NONE,
                gio::Cancellable::NONE,
            )
            .unwrap();
        let all = WorkspaceConfig::load_all_from_dir(dir.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "alpha");
    }

    #[test]
    fn delete_is_idempotent() {
        ensure_gio_init();
        let dir = tempdir().unwrap();
        WorkspaceConfig::delete_from_dir(dir.path(), "missing").unwrap();
        let cfg = WorkspaceConfig::new("x", "/x");
        cfg.save_to_dir(dir.path()).unwrap();
        WorkspaceConfig::delete_from_dir(dir.path(), &cfg.slug()).unwrap();
        assert!(WorkspaceConfig::load_from_dir(dir.path(), &cfg.slug()).is_err());
    }
}
