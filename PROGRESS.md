# omux — Milestone Progress

Reference: `docs/design.md` §10.

| # | Milestone | Status | Notes |
|---|---|---|---|
| **M0** | Scaffold | ✅ done | Cargo workspace, empty Adwaita window, fmt/clippy/test clean |
| **M1** | Single terminal pane (vte4-rs) | ✅ done | `TerminalPane` wraps `vte4::Terminal`; spawns `$SHELL` with `OMUX_PANE_ID` injected. vte4 0.9 + uuid 1 deps added. |
| **M2** | Split panes + per-pane tabs | ✅ done | Recursive `PaneTree` (leaves = `gtk::Notebook` of `TerminalPane` tabs; splits = `gtk::Paned`). Shortcuts: `Ctrl+Shift+D` h-split, `Ctrl+Shift+E` v-split, `Ctrl+T` new tab, `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle focus. Drag-resize free from `gtk::Paned`. Click-to-focus deferred to M6. |
| **M3** | Workspaces (TOML + SQLite) | ✅ done | Persistence layer (config TOML + SQLite state) + WorkspaceManager + PaneTree snapshot/restore + Adwaita OverlaySplitView sidebar with workspace list, "New workspace" button, right-click context menu (Rename / Pin / Delete), and click-to-switch. Drag-drop reorder UI deferred to M6 polish; the reorder *backend* (`WorkspaceManager::reorder`) is implemented and unit-tested. 16 unit tests cover the persistence layer. |
| **M4** | Agent detection + hook + notification | 🚧 partial | Phases A+B+C done: agent manifests (built-in Claude Code + Codex, user overrides at `agents/*.toml`), process detection via `tcgetpgrp` + `/proc/<pid>/comm`, status state machine (`Idle` / `Running` / `NeedsAttention`), CSS ring on the pane Frame. Phase D (D-Bus + `omux-hook` + first-run `~/.claude/settings.json` merge) and Phase E (PTY output-regex fallback) land in the next iterations. 29 unit tests (13 new for agent module). |
| **M5** | Embedded browser pane | ⏳ pending | Adds `webkit6` dep; second variant on `PaneKind` |
| **M6** | Polish | ⏳ pending | Right-click menus, animated sidebar, click-to-focus, directional Alt+arrow nav |

## Action items for user

None right now — loop has bounded iterations and proceeds autonomously.

## Manual smoke checklist

After each milestone, the user should verify the GTK UI manually (no headless test exists).

**M2:**
1. `cargo run` → window opens with a single VTE terminal running your `$SHELL`.
2. `Ctrl+Shift+D` → split side-by-side (h-split). New shell appears to the right.
3. `Ctrl+Shift+E` → split top-bottom (v-split) inside the focused leaf.
4. `Ctrl+T` → new tab in the focused leaf.
5. `Ctrl+Tab` / `Ctrl+Shift+Tab` → cycle focus through leaves.
6. Drag the divider between split panes → resize works.
7. Run `printenv OMUX_PANE_ID` in any shell → should print a UUID unique to that pane.

**M4 (partial):**
17. Run `cargo run`. In any pane: run `bash` then `claude` (or just `claude` if your shell's `claude` is a wrapper). Within ~500ms the pane should gain a thin accent border (the `.agent-running` class).
18. Exit Claude → border returns to idle (no class).
19. The full notification ring fires when an agent's stop/notification hook calls `omux-hook`. That helper isn't wired yet; the visual is testable manually by editing `pane/terminal.rs::apply_status_event` to fire `AttentionRequested` on a keyboard shortcut.

**M3:**
8. First run seeds a `default` workspace tied to `$HOME`. Check `$XDG_CONFIG_HOME/omux/workspaces/default.toml` exists.
9. Split a few times, then close the window. `default.toml` should be updated with the saved layout (look for a `[layout]` table inside).
10. Relaunch `omux` → the window comes back with the same split layout (fresh shells inside).
11. Click "New workspace", enter a name → workspace created, sidebar updates, content switches to a single fresh shell.
12. Right-click a sidebar row → context menu with Rename / Pin / Delete.
13. Pin one → moves to the top of the sidebar with a star icon.
14. Rename one → file under `workspaces/` is renamed; SQLite row updated.
15. Delete the active one → confirmation dialog; on confirm, switches to the next remaining workspace.
16. Switch between two non-trivial workspaces → each retains its own split layout independently.

## Decisions log

- **2026-05-17 (M0)** — Swapped terminal renderer from libghostty to vte4-rs. libghostty is not packaged as a standalone shared library on Arch / Cachy OS, and building Ghostty from source to extract it is out of scope. Per design.md §13.1 escape hatch.
