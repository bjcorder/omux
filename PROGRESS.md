# omux — Milestone Progress

Reference: `docs/design.md` §10.

| # | Milestone | Status | Notes |
|---|---|---|---|
| **M0** | Scaffold | ✅ done | Cargo workspace, empty Adwaita window, fmt/clippy/test clean |
| **M1** | Single terminal pane (vte4-rs) | ✅ done | `TerminalPane` wraps `vte4::Terminal`; spawns `$SHELL` with `OMUX_PANE_ID` injected. vte4 0.9 + uuid 1 deps added. |
| **M2** | Split panes + per-pane tabs | ✅ done | Recursive `PaneTree` (leaves = `gtk::Notebook` of `TerminalPane` tabs; splits = `gtk::Paned`). Shortcuts: `Ctrl+Shift+D` h-split, `Ctrl+Shift+E` v-split, `Ctrl+T` new tab, `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle focus. Drag-resize free from `gtk::Paned`. Click-to-focus deferred to M6. |
| **M3** | Workspaces (TOML + SQLite) | 🚧 partial | Phase A done (TOML config, SQLite state, workspace manager, 16 unit tests). Phase B done (PaneTree snapshot/restore round-trips through TOML). Phase D partial: default workspace is auto-seeded, layout persists on window close. Phase C (sidebar UI with create/rename/delete/pin/reorder) and full multi-workspace switching land in the next iteration. |
| **M4** | Agent detection + hook + notification | ⏳ pending | Depends on PTY-output stream from `pane/terminal.rs` |
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

**M3 partial:**
8. First run after this milestone seeds a `default` workspace tied to `$HOME`. Check `$XDG_CONFIG_HOME/omux/workspaces/default.toml` exists.
9. Split a few times, then close the window. `default.toml` should be updated with the saved layout (look for a `[layout]` table inside).
10. Relaunch `omux` → the window comes back with the same split layout (fresh shells inside).

## Decisions log

- **2026-05-17 (M0)** — Swapped terminal renderer from libghostty to vte4-rs. libghostty is not packaged as a standalone shared library on Arch / Cachy OS, and building Ghostty from source to extract it is out of scope. Per design.md §13.1 escape hatch.
