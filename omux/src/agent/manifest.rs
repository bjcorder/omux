//! Agent manifests (design §3.3).
//!
//! Each agent harness is described by a TOML file at
//! `$XDG_CONFIG_HOME/omux/agents/<name>.toml`. omux ships with built-in
//! manifests for Claude Code and Codex; users can drop additional `.toml`
//! files into the directory to add new harnesses without code changes.

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentManifest {
    pub name: String,
    pub display_name: String,
    /// Regex patterns matched against `/proc/<pid>/comm` (the kernel-truncated
    /// short process name). Any match marks the pane as running this agent.
    pub process_patterns: Vec<String>,
    #[serde(default)]
    pub fallback: FallbackConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FallbackConfig {
    /// PTY-output regexes that, when matched, raise a needs-attention event
    /// for harnesses without hook integration.
    #[serde(default)]
    pub needs_attention_patterns: Vec<String>,
    /// Idle-after-last-output threshold in seconds (0 disables).
    #[serde(default)]
    pub idle_timeout_secs: u64,
}

/// Same shape as [`AgentManifest`] but with the regexes compiled. Cheap
/// to share across pane-detect tasks; created once at app startup.
#[derive(Clone)]
pub struct CompiledManifest {
    pub name: String,
    pub display_name: String,
    pub process_patterns: Vec<Regex>,
    #[allow(dead_code)] // Consumed by the PTY output-parser fallback (M4 phase E).
    pub needs_attention_patterns: Vec<Regex>,
    #[allow(dead_code)] // Consumed by the idle-timeout watchdog (M4 phase E).
    pub idle_timeout_secs: u64,
}

impl AgentManifest {
    pub fn compile(&self) -> anyhow::Result<CompiledManifest> {
        let process_patterns = self
            .process_patterns
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;
        let needs_attention_patterns = self
            .fallback
            .needs_attention_patterns
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompiledManifest {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            process_patterns,
            needs_attention_patterns,
            idle_timeout_secs: self.fallback.idle_timeout_secs,
        })
    }
}

/// Built-in manifests inlined into the binary. Users can override these
/// by placing a same-named TOML at `$XDG_CONFIG_HOME/omux/agents/`.
pub fn builtin_manifests() -> Vec<AgentManifest> {
    let mut out = Vec::new();
    for src in [CLAUDE_CODE_MANIFEST, CODEX_MANIFEST] {
        match toml::from_str::<AgentManifest>(src) {
            Ok(m) => out.push(m),
            Err(e) => tracing::error!(error = %e, "broken built-in manifest"),
        }
    }
    out
}

/// Load all manifests: built-ins first, then any user overrides from
/// `<config_dir>/agents/*.toml` which replace built-ins of the same name.
pub fn load_all(user_dir: Option<&std::path::Path>) -> Vec<AgentManifest> {
    let mut by_name: std::collections::HashMap<String, AgentManifest> = builtin_manifests()
        .into_iter()
        .map(|m| (m.name.clone(), m))
        .collect();

    if let Some(dir) = user_dir {
        for m in load_user_manifests(dir) {
            tracing::info!(name = %m.name, "loaded user agent manifest");
            by_name.insert(m.name.clone(), m);
        }
    }

    let mut list: Vec<_> = by_name.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

fn load_user_manifests(dir: &std::path::Path) -> Vec<AgentManifest> {
    use gtk4::gio;
    use gtk4::prelude::*;

    let mut out = Vec::new();
    let file = gio::File::for_path(dir);
    if !file.query_exists(gio::Cancellable::NONE) {
        return out;
    }
    let enumerator = match file.enumerate_children(
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "could not enumerate agents dir");
            return out;
        }
    };
    loop {
        match enumerator.next_file(gio::Cancellable::NONE) {
            Ok(Some(info)) => {
                if info.file_type() != gio::FileType::Regular {
                    continue;
                }
                let name = info.name();
                let name_str = name.to_string_lossy();
                if !name_str.ends_with(".toml") {
                    continue;
                }
                let child = file.child(&*name_str);
                let bytes = match child.load_contents(gio::Cancellable::NONE) {
                    Ok((b, _)) => b,
                    Err(e) => {
                        tracing::warn!(file = %name_str, error = %e, "agent manifest load failed");
                        continue;
                    }
                };
                let text = match std::str::from_utf8(&bytes) {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        tracing::warn!(file = %name_str, "agent manifest is not UTF-8");
                        continue;
                    }
                };
                match toml::from_str::<AgentManifest>(&text) {
                    Ok(m) => out.push(m),
                    Err(e) => {
                        tracing::warn!(file = %name_str, error = %e, "agent manifest parse failed")
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "agents dir enumeration error");
                break;
            }
        }
    }
    out
}

const CLAUDE_CODE_MANIFEST: &str = r#"
name = "claude-code"
display_name = "Claude Code"
process_patterns = ["^claude$", "^claude-code$"]

[fallback]
needs_attention_patterns = []
idle_timeout_secs = 0
"#;

const CODEX_MANIFEST: &str = r#"
name = "codex"
display_name = "Codex CLI"
process_patterns = ["^codex$"]

[fallback]
needs_attention_patterns = ["Press Enter to continue"]
idle_timeout_secs = 0
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse_and_compile() {
        let manifests = builtin_manifests();
        assert!(manifests.len() >= 2);
        for m in manifests {
            let _compiled = m.compile().expect("builtin compile");
        }
    }

    #[test]
    fn claude_pattern_matches_expected_names() {
        let manifests = builtin_manifests();
        let claude = manifests
            .iter()
            .find(|m| m.name == "claude-code")
            .unwrap()
            .compile()
            .unwrap();
        assert!(claude.process_patterns.iter().any(|r| r.is_match("claude")));
        assert!(
            claude
                .process_patterns
                .iter()
                .any(|r| r.is_match("claude-code"))
        );
        assert!(
            !claude
                .process_patterns
                .iter()
                .any(|r| r.is_match("clauded"))
        );
    }

    #[test]
    fn user_manifest_overrides_builtin_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude-code.toml");
        std::fs::write(
            &path,
            r#"
                name = "claude-code"
                display_name = "Claude Code (custom)"
                process_patterns = ["^claude$"]
                [fallback]
                needs_attention_patterns = []
                idle_timeout_secs = 0
            "#,
        )
        .unwrap();

        let manifests = load_all(Some(dir.path()));
        let claude = manifests.iter().find(|m| m.name == "claude-code").unwrap();
        assert_eq!(claude.display_name, "Claude Code (custom)");
    }

    #[test]
    fn invalid_manifest_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.toml"), "this = is not valid").unwrap();
        // Should not panic; built-ins still come through.
        let manifests = load_all(Some(dir.path()));
        assert!(manifests.iter().any(|m| m.name == "claude-code"));
    }
}
