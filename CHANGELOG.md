# Changelog

All notable changes to omux.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres loosely to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] — Unreleased

First implementation pass. All M0–M6 milestones from
[`docs/design.md`](./docs/design.md) landed. Targets Linux with GTK 4.18+,
libadwaita 1.7+, WebKitGTK 6.0, VTE 4.

### M0 — Scaffold

- Cargo workspace with two binary crates: `omux` (GUI) and `omux-hook`
  (small hook helper).
- Rust 2024 edition, build/fmt/clippy(-D warnings)/test gates established.

### M1 — Single terminal pane

- `pane::terminal::TerminalPane` wraps `vte4::Terminal` inside a
  `gtk::Frame` so CSS classes can ring the pane.
- Spawns the user's `$SHELL` with `OMUX_PANE_ID=<uuid>` injected into the
  child env.
- libghostty was the original spec; swapped to vte4 at M0 (documented as
  a design escape hatch — see `docs/design.md` §13.1) because libghostty
  has no standalone Linux library packaging yet.

### M2 — Splits + tabs + keyboard nav

- Recursive `PaneTree`: leaves are `gtk::Notebook` containers of tabbed
  panes; splits are `gtk::Paned` (h or v).
- Shortcuts: `Ctrl+Shift+D` / `Ctrl+Shift+E` split, `Ctrl+T` new tab,
  `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle focus, drag-resize free from
  `Paned`.

### M3 — Workspace persistence + sidebar

- `workspace::config` — serde TOML schema for `WorkspaceConfig` + safe,
  slug-validated gio I/O at `$XDG_CONFIG_HOME/omux/workspaces/*.toml`.
- `workspace::state` — rusqlite-backed `state.db` with `user_version`
  migrations, tracking workspace order / last-opened timestamp / active
  workspace.
- `workspace::manager` — `WorkspaceManager` joins the two stores and
  exposes upsert / delete / rename / pin / reorder / set-active.
- `pane::tree::PaneTree::snapshot` / `from_snapshot` round-trip layouts
  through `workspace::snapshot::LayoutNode` (per-tab kind + URL).
- `ui::AppShell` glues a `WorkspaceManager` + `ui::Sidebar` + content
  bin together. `adw::OverlaySplitView` for the sidebar layout.
- Sidebar: workspace rows, "New workspace" dialog, right-click context
  menu (Rename / Pin / Delete) backed by per-row `gio::SimpleActionGroup`.

### M4 — Agent detection + notification

- `agent::manifest` — built-in TOML manifests for Claude Code and Codex,
  user overrides at `$XDG_CONFIG_HOME/omux/agents/*.toml`.
- `agent::detect` — polls `tcgetpgrp(pty_fd)` → `/proc/<pid>/comm` →
  manifest regex match every 500 ms.
- `agent::status` — `PaneStatus` state machine (`Idle` → `Running` →
  `NeedsAttention`, cleared on focus).
- `ipc::SocketService` — Unix domain socket at
  `$XDG_RUNTIME_DIR/omux/control.sock`, line-delimited JSON events;
  drains `pending-events.jsonl` on startup so events buffered while omux
  was offline are not lost.
- `omux-hook` helper binary: argv-driven, writes a `HookEvent` line to
  the socket or buffers to `pending-events.jsonl`.
- `agent::hook_installer` — idempotent merge of `Stop` + `Notification`
  hooks into `~/.claude/settings.json` (sentinel-tagged); first-run
  consent dialog from the shell.
- D-Bus → Unix socket: design.md §5.5 originally specified D-Bus.
  Switched to Unix socket at phase D for lighter deps + clean glib
  main-loop integration. Decision recorded in `omux/src/ipc/mod.rs`.
- PTY output regex fallback for harnesses without hooks; debounced 2 s
  and deduplicated by matched text.
- Visual: pane ring (`.needs-attention` CSS), per-tab badge,
  per-workspace badge in the sidebar (all driven by a 250 ms refresh
  timer).

### M5 — Embedded browser pane

- `pane::browser::BrowserPane` — `webkit6::WebView` + URL bar +
  back / forward / reload, wrapped in a `pane-frame` Frame.
- `Pane` enum unifies `Terminal` + `Browser` in a leaf's tab list.
- Per-workspace `webkit6::NetworkSession` at
  `$XDG_CONFIG_HOME/omux/web/<slug>/` so cookies / local storage are
  scoped to the workspace.
- `LeafSnapshot.tabs` is now `Vec<TabSnapshot>` (kind + URL); a custom
  serde deserializer still accepts the M3 legacy integer form so
  pre-M5 workspace files load.
- Shortcut: `Ctrl+Shift+B` adds a browser tab to the focused leaf.

### M6 — Polish

- CLI: `--uninstall-hooks` (surgical removal of `_omux_managed` entries,
  preserving everything else in `settings.json`), `--version`, `--help`.
- PTY output regex fallback (M4 phase E carried over).
- Click-to-focus: every pane gets an `EventControllerFocus` that updates
  `TreeState.focused` when focus enters its frame subtree.
- Right-click terminal context menu: Copy / Paste / Clear (per-pane)
  and Split / New tab / Close tab (window actions).
- Keyboard: `Ctrl+Shift+C` copy, `Ctrl+Shift+V` paste, `Ctrl+W` close
  tab, `Alt+Arrow` directional pane navigation.
- Tab needs-attention badges + sidebar workspace badges driven by a
  250 ms refresh timer.
- Sidebar drag-drop reorder via `gtk::DragSource` + `gtk::DropTarget`;
  fires `WorkspaceManager::reorder`.
- CSS theme polish: `.dragging`, `.browser-url`, `.tab-needs-attention`,
  `.sidebar-badge`.

### Post-M6 stabilization

- SIGINT / SIGTERM handlers explicitly remove the control socket
  before quitting (the `Drop` path doesn't run reliably through
  `app.quit()`).
- `--uninstall-hooks` switched from full-backup-restore to surgical
  removal so settings.json edits made between install and uninstall
  (e.g. plugin toggles done by Claude Code) survive.
- README rewritten as a complete user guide.
- `omux-hook/tests/ipc_end_to_end.rs` — eight end-to-end tests for the
  helper ↔ socket wire contract (round-trip, env vs flag pane id,
  kebab-case kind, payload pass-through, all error paths, the offline
  pending-jsonl fallback). Closes the design.md §8 testing gap that
  was originally written against D-Bus.

### Known gaps (deferred out of v1)

- Closing the last tab in a split-child leaf would require widget
  re-parenting the sibling up one level; currently refused.
- Drag-drop pin-boundary enforcement during a sidebar drag (basic
  reorder works; pinned items aren't locked to the top mid-drag).
- The §9 GUI verification checklist (`docs/design.md` §9, mirrored to
  `PROGRESS.md`) requires a human at the keyboard for steps 2–6 and 8;
  CI-driven verification of these would need xvfb + a working WebKit
  in headless mode and is out of scope for v1.
- No `git` remote is wired; the design.md §1 distribution decision was
  `cargo build` from source. Push artifacts and CI live wherever the
  user eventually pushes the repository.

### Post-v1 stabilization (after live smoke testing)

A round of bug fixes + UX additions found while driving the live
app with kdotool + ydotool on a KDE Plasma Wayland session.

- **Pane registry reconcile** — new panes from splits / `Ctrl+T` /
  `Ctrl+Shift+B` weren't registered for hook routing until the next
  workspace switch. The 250 ms badge timer now also reconciles the
  registry, so any pane gets routable within one tick of creation.
- **Capture-phase keyboard shortcuts** — the window-level
  `ShortcutController` defaulted to bubble phase, which let VTE eat
  every `Ctrl+Shift+*` / `Ctrl+T` / `Ctrl+W` keystroke before it
  reached the shortcut handler. Switched to capture phase.
- **Multi-live PaneTree per workspace** — switching workspaces used
  to rebuild the target tree from snapshot every time, killing
  every running shell and resetting the cwd. `AppShell.trees` is
  now a `HashMap<String, PaneTree>`; each opened workspace stays
  alive for the lifetime of the app.
- **Per-leaf `+` button** — `gtk::Notebook` action widget at the
  right end of each tab bar opens a popover with **New tab →
  Terminal / Browser** and **Split this pane → Side-by-side /
  Top-bottom**. Per-leaf `gio::SimpleActionGroup` so each leaf's
  button targets itself.
- **Per-tab `×` close button** — clicking removes the tab. If it
  was the last tab in a non-root leaf, `collapse_leaf` re-parents
  the sibling subtree into the grandparent (or the root bin) so
  the empty leaf goes away. The workspace-root leaf refuses to
  empty.
- **Resizable sidebar** — `adw::OverlaySplitView` → `gtk::Paned`
  so the divider is user-draggable. Width persists across restarts
  via a new generic `app_state_get`/`app_state_set` k/v store on
  the SQLite state DB.
- **Clean shutdown** — `cleanup_runtime_state` is now wired to
  `Application::connect_shutdown` (in addition to SIGINT/SIGTERM
  handlers) so the control socket is removed on normal window
  close, not just on signals.
- **Install scripts** — `scripts/install.sh` / `scripts/uninstall.sh`
  drop binaries into `~/.local/bin` (or `/usr/local/bin` with
  `--system`), `.desktop` entry into the XDG applications dir, and
  the SVG icon into the hicolor icon theme.
- **Two `RefCell` panics caught in dialogs** — `match
  manager.borrow_mut().upsert(cfg) { Ok(()) => switch_workspace
  (...) }` and the `if let Err … else` rename path both kept the
  scrutinee RefMut alive across the whole expression by Rust's
  temporary-lifetime rule, conflicting with later `manager
  .borrow()` calls. Hoisted both into `let`-bindings.

### Stats (current)

- 23 commits on `main`.
- 52 automated tests pass (44 in `omux`, 8 in `omux-hook`).
- `cargo build --release` produces a 5.4 MB `omux` binary and a 452 KB
  `omux-hook` helper.
