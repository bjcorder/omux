//! Idempotent install/uninstall of Claude Code hook entries into
//! `~/.claude/settings.json` (design §4.3).
//!
//! On first launch, omux shows a consent dialog. On accept, this module
//! merges two hook entries (`Stop` + `Notification`) marked with the
//! sentinel `"_omux_managed": true` field. A backup of the original
//! file is saved to `~/.claude/settings.json.omux-backup` so the merge
//! is reversible via `omux --uninstall-hooks` (M4 phase D / M6 polish
//! depending on when CLI flags land).

use std::path::PathBuf;

use serde_json::{Value, json};

const SENTINEL: &str = "_omux_managed";

pub struct InstallResult {
    pub installed_now: bool,
    pub backup_path: PathBuf,
}

pub fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".claude").join("settings.json"))
}

pub fn backup_path() -> Option<PathBuf> {
    settings_path().map(|p| p.with_extension("json.omux-backup"))
}

/// True if `~/.claude/settings.json` already has an omux-managed hook
/// block (or if the path can't be resolved, in which case the caller
/// should not prompt).
pub fn already_installed() -> bool {
    let Some(path) = settings_path() else {
        return true;
    };
    let Ok(text) = read_file(&path) else {
        return false;
    };
    let Ok(value): Result<Value, _> = serde_json::from_str(&text) else {
        return false;
    };
    has_omux_managed_hook(&value)
}

/// Insert the omux hook entries into the settings file, creating the
/// file (and parent directory) if necessary. Atomically writes via
/// `<path>.tmp` → rename. A backup of the original is saved to
/// `<path>.omux-backup`.
pub fn install() -> anyhow::Result<InstallResult> {
    let path = settings_path()
        .ok_or_else(|| anyhow::anyhow!("HOME unset; cannot resolve settings path"))?;
    let backup = backup_path().unwrap();

    let mut value: Value = match read_file(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };

    // Save backup before mutating (only if the file existed and we haven't
    // already backed up).
    if path.exists() && !backup.exists() {
        if let Some(parent) = backup.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let original = read_file(&path).unwrap_or_default();
        atomic_write(&backup, original.as_bytes())?;
    }

    let installed_now = ensure_omux_hooks(&mut value);

    if installed_now {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_string_pretty(&value)?;
        atomic_write(&path, serialized.as_bytes())?;
    }

    Ok(InstallResult {
        installed_now,
        backup_path: backup,
    })
}

/// Surgically remove omux-managed hook entries from the current
/// `~/.claude/settings.json`, leaving anything else (user-installed
/// hooks, plugin lists, settings the user or Claude Code added since
/// install time) untouched.
///
/// Returns `true` if at least one omux-managed entry was removed. The
/// stale backup file is deleted after a successful uninstall.
pub fn uninstall() -> anyhow::Result<bool> {
    let path = settings_path()
        .ok_or_else(|| anyhow::anyhow!("HOME unset; cannot resolve settings path"))?;
    let backup = backup_path().unwrap();
    if !path.exists() {
        return Ok(false);
    }
    let text = read_file(&path)?;
    let mut value: Value = serde_json::from_str(&text)?;
    let removed = strip_omux_hooks(&mut value);
    if removed {
        let serialized = serde_json::to_string_pretty(&value)?;
        atomic_write(&path, serialized.as_bytes())?;
        let _ = std::fs::remove_file(&backup);
    }
    Ok(removed)
}

/// Walk `value.hooks.<Event>[*].hooks[*]` and remove any entries
/// carrying `_omux_managed: true`. Clean up newly empty arrays so the
/// JSON stays tidy.
fn strip_omux_hooks(value: &mut Value) -> bool {
    let Some(hooks) = value.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };
    let mut changed = false;
    let event_names: Vec<String> = hooks.keys().cloned().collect();
    for event in event_names {
        let Some(arr) = hooks.get_mut(&event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        let before = arr.len();
        arr.retain(|entry| {
            // Keep entries that do NOT contain any omux-managed inner hook.
            let Some(inner) = entry.get("hooks").and_then(|h| h.as_array()) else {
                return true;
            };
            !inner.iter().any(is_omux_managed)
        });
        if arr.len() != before {
            changed = true;
        }
        if arr.is_empty() {
            hooks.remove(&event);
        }
    }
    // If the top-level `hooks` object is now empty, drop it too so the
    // file doesn't end up with a dangling `"hooks": {}` block.
    if hooks.is_empty()
        && let Some(obj) = value.as_object_mut()
    {
        obj.remove("hooks");
    }
    changed
}

fn read_file(path: &std::path::Path) -> anyhow::Result<String> {
    use gtk4::gio;
    use gtk4::prelude::*;
    let file = gio::File::for_path(path);
    let (bytes, _etag) = file
        .load_contents(gio::Cancellable::NONE)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use gtk4::gio;
    use gtk4::prelude::*;
    let file = gio::File::for_path(path);
    file.replace_contents(
        bytes,
        None,
        false,
        gio::FileCreateFlags::NONE,
        gio::Cancellable::NONE,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// Merge `Stop` + `Notification` hook entries into the settings value.
/// Returns `true` if anything changed.
fn ensure_omux_hooks(value: &mut Value) -> bool {
    let stop_hook = build_hook_entry("stop");
    let notif_hook = build_hook_entry("notification");

    let mut changed = false;
    let hooks = value
        .as_object_mut()
        .map(|obj| {
            obj.entry("hooks".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
        })
        .and_then(|h| h.as_object_mut());

    let Some(hooks) = hooks else {
        return false;
    };

    changed |= ensure_event_hook(hooks, "Stop", stop_hook);
    changed |= ensure_event_hook(hooks, "Notification", notif_hook);
    changed
}

fn ensure_event_hook(
    hooks: &mut serde_json::Map<String, Value>,
    event_name: &str,
    new_entry: Value,
) -> bool {
    let arr = hooks
        .entry(event_name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(arr) = arr.as_array_mut() else {
        return false;
    };
    // Look for an existing omux-managed entry (matcher == "" carrying our sentinel).
    let already = arr.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|nested| nested.iter().any(is_omux_managed))
            .unwrap_or(false)
    });
    if already {
        return false;
    }
    arr.push(new_entry);
    true
}

fn is_omux_managed(value: &Value) -> bool {
    value
        .get(SENTINEL)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn has_omux_managed_hook(value: &Value) -> bool {
    let Some(hooks) = value.get("hooks").and_then(|h| h.as_object()) else {
        return false;
    };
    for event in ["Stop", "Notification"] {
        if let Some(arr) = hooks.get(event).and_then(|v| v.as_array()) {
            for entry in arr {
                if let Some(nested) = entry.get("hooks").and_then(|n| n.as_array()) {
                    if nested.iter().any(is_omux_managed) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn build_hook_entry(event: &str) -> Value {
    // Claude Code's hook config (see Claude Code docs): a hook entry is
    //   { matcher: "", hooks: [{ type: "command", command: "..." }] }
    // We add a sentinel `_omux_managed: true` to the inner object so we
    // can detect (and later remove) our own entries.
    //
    // The command is `omux-hook <event>` with no --pane arg: omux-hook
    // reads `OMUX_PANE_ID` from the env (omux injects it when spawning
    // each pane's shell, and the env propagates through the
    // shell → claude → hook process chain).
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": format!("omux-hook {event}"),
            SENTINEL: true,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_omux_hooks_inserts_when_empty() {
        let mut v = json!({});
        assert!(ensure_omux_hooks(&mut v));
        assert!(v["hooks"]["Stop"].is_array());
        assert!(v["hooks"]["Notification"].is_array());
        assert!(has_omux_managed_hook(&v));
    }

    #[test]
    fn ensure_omux_hooks_is_idempotent() {
        let mut v = json!({});
        assert!(ensure_omux_hooks(&mut v));
        assert!(!ensure_omux_hooks(&mut v));
    }

    #[test]
    fn merges_with_existing_user_hooks() {
        let mut v = json!({
            "hooks": {
                "Stop": [
                    { "matcher": ".*", "hooks": [ { "type": "command", "command": "echo bye" } ] }
                ]
            }
        });
        assert!(ensure_omux_hooks(&mut v));
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        // First (user) entry is preserved.
        assert_eq!(stop[0]["matcher"], ".*");
        // Second is omux-managed.
        assert!(stop[1]["hooks"][0][SENTINEL].as_bool().unwrap_or(false));
    }

    #[test]
    fn detects_existing_managed_block() {
        let v = json!({
            "hooks": {
                "Stop": [
                    {
                        "matcher": "",
                        "hooks": [ { "type": "command", "command": "omux-hook stop", SENTINEL: true } ]
                    }
                ]
            }
        });
        assert!(has_omux_managed_hook(&v));
    }

    #[test]
    fn strip_removes_only_managed_entries() {
        let mut v = json!({
            "enabledPlugins": { "x": true },
            "hooks": {
                "Stop": [
                    { "matcher": "user", "hooks": [ { "type": "command", "command": "echo bye" } ] },
                    { "matcher": "", "hooks": [ { "type": "command", "command": "omux-hook stop", SENTINEL: true } ] }
                ],
                "Notification": [
                    { "matcher": "", "hooks": [ { "type": "command", "command": "omux-hook notification", SENTINEL: true } ] }
                ]
            }
        });
        assert!(strip_omux_hooks(&mut v));
        // User's enabledPlugins survives untouched.
        assert_eq!(v["enabledPlugins"]["x"], json!(true));
        // The user's Stop hook survives; the omux-managed one is gone.
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["matcher"], "user");
        // Notification only had an omux-managed entry → the whole key is dropped.
        assert!(v["hooks"].get("Notification").is_none());
    }

    #[test]
    fn strip_drops_empty_hooks_block() {
        let mut v = json!({
            "hooks": {
                "Stop": [
                    { "matcher": "", "hooks": [ { "type": "command", "command": "x", SENTINEL: true } ] }
                ]
            }
        });
        assert!(strip_omux_hooks(&mut v));
        assert!(v.get("hooks").is_none());
    }

    #[test]
    fn strip_is_idempotent_when_nothing_managed() {
        let mut v = json!({
            "hooks": { "Stop": [ { "matcher": "", "hooks": [ { "type": "command", "command": "x" } ] } ] }
        });
        assert!(!strip_omux_hooks(&mut v));
        // User hook still present.
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }
}
