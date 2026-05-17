# omux — Milestone Progress

Reference: `docs/design.md` §10.

| # | Milestone | Status | Notes |
|---|---|---|---|
| **M0** | Scaffold | ✅ done | Cargo workspace, empty Adwaita window, fmt/clippy/test clean |
| **M1** | Single terminal pane (vte4-rs) | ✅ done | `TerminalPane` wraps `vte4::Terminal`; spawns `$SHELL` with `OMUX_PANE_ID` injected. vte4 0.9 + uuid 1 deps added. |
| **M2** | Split panes + per-pane tabs | ✅ done | Recursive `PaneTree` (leaves = `gtk::Notebook` of `TerminalPane` tabs; splits = `gtk::Paned`). Shortcuts: `Ctrl+Shift+D` h-split, `Ctrl+Shift+E` v-split, `Ctrl+T` new tab, `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle focus. Drag-resize free from `gtk::Paned`. Click-to-focus deferred to M6. |
| **M3** | Workspaces (TOML + SQLite) | ✅ done | Persistence layer (config TOML + SQLite state) + WorkspaceManager + PaneTree snapshot/restore + Adwaita OverlaySplitView sidebar with workspace list, "New workspace" button, right-click context menu (Rename / Pin / Delete), and click-to-switch. Drag-drop reorder UI deferred to M6 polish; the reorder *backend* (`WorkspaceManager::reorder`) is implemented and unit-tested. 16 unit tests cover the persistence layer. |
| **M4** | Agent detection + hook + notification | ✅ done | Manifests + process detection + status state machine + notification ring + Unix-socket IPC + `omux-hook` helper + first-run hook installer. Switched IPC from D-Bus to Unix domain socket at phase D (lighter dependency, integrates natively with the glib main loop via `gio::SocketService`); decision recorded in `omux/src/ipc/mod.rs`. 35 unit tests. Phase E (PTY output-regex fallback for harnesses without hooks) folded into M6 polish since Claude Code's hook path is the headline use case. |
| **M5** | Embedded browser pane | ✅ done | `webkit6` 0.5 dep, `BrowserPane` (WebView + URL bar + back/fwd/reload), `Pane` enum unifying Terminal + Browser in leaves, `TabSnapshot` persists per-tab kind + URL (with legacy-integer fallback), per-workspace `NetworkSession` isolating cookies/storage. `Ctrl+Shift+B` adds a browser tab to the focused leaf. 41 unit tests. |
| **M6** | Polish | ✅ done | Right-click pane context menu (copy/paste/clear/splits/new+close tab), Ctrl+Shift+C/V/Ctrl+W keyboard shortcuts, tab needs-attention badges, sidebar workspace badges, click-to-focus, Alt+Arrow directional pane nav, sidebar drag-reorder, PTY output regex fallback, `--uninstall-hooks` CLI flag, CSS theme polish. Animated sidebar collapse comes free from `AdwOverlaySplitView`. Leaf-collapse-on-last-tab-close deferred — single-tab leaves can't close their tab (low-impact). |

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

**M5:**
22. `Ctrl+Shift+B` in any leaf → a browser tab appears with URL bar at `about:blank`.
23. Type a URL (or a search query) into the bar + Enter → page loads. Bare domains autocomplete with `https://`; queries route to DuckDuckGo.
24. Back / forward / reload buttons work, sensitive state tracks the navigation history.
25. Close and reopen the workspace → browser tabs come back at the URLs they were on.
26. Two workspaces each with a browser pane logged into the same site → sessions are separate (cookies live under per-workspace data dirs).

**M4:**
17. First launch shows a "Enable Claude Code notifications?" dialog. Click "Install hooks" → omux merges its Stop / Notification entries into `~/.claude/settings.json` (with a `.omux-backup` of the original). Skip and the visual ring still works for agent-running detection, just not for turn-end notifications.
18. In a terminal pane, run `claude`. Within ~500ms the pane gains a thin accent border (`.agent-running`).
19. Trigger a Claude turn that ends. Within ~1s the pane gains the bright accent ring + glow (`.needs-attention`). Focus the pane → ring clears.
20. End-to-end test without Claude: `echo '{"kind":"stop","pane_id":"<uuid-from-the-pane>"}' | nc -U "$XDG_RUNTIME_DIR/omux/control.sock"` → that pane lights up.
21. Quit omux while a hook fires: `omux-hook stop` falls back to `$XDG_RUNTIME_DIR/omux/pending-events.jsonl`. Relaunching omux drains the file and applies the events.

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
