# omux — Design Spec & Implementation Plan

**Date:** 2026-05-17
**Status:** Draft for approval
**Target dir:** `/home/bcorder/Documents/Playground/omux` (currently empty)
**Implementation method:** `/ralph-loop` (iterative autonomous execution against the milestones in §10)

---

## Context

Build a Linux-native desktop application — `omux` — that hosts multiple terminal panes plus an embedded browser pane in a single window, with **in-app notification badges** when an AI coding agent (Claude Code, Codex CLI, or any user-declared harness) finishes a turn or requests input.

This is the gap cmux (macOS-only) and limux (Linux, no agent integration) leave: a Linux app with limux's terminal-workspace ergonomics *and* cmux's "blue ring" agent-attention signal. The headline UX: glance at the sidebar and immediately see which workspace/tab/pane has an agent waiting on you.

The user has chosen a single monolithic spec rather than a decomposed roll-out. All v1 features below are committed scope.

---

## 1. Decisions (locked)

| Area | Decision |
|---|---|
| Name | `omux` |
| Target OS | Linux (developed against Arch / Cachy OS, GTK4 + WebKitGTK 6 stack) |
| Architecture | Native GTK4 application, single Rust binary; embedded WebKitGTK browser pane |
| Language / GUI | Rust + `gtk4-rs` |
| Terminal renderer | `libghostty` (Zig) via Rust FFI; fallback `vte4-rs` documented but not built |
| Browser pane | `webkit6` crate (WebKitGTK 6) |
| IPC | D-Bus session bus via `zbus`; helper binary `omux-hook` is what Claude hooks invoke |
| Async | `glib::MainContext` for UI; `tokio` (current-thread or multi-thread) for non-UI I/O |
| Persistence — config | TOML at `$XDG_CONFIG_HOME/omux/` |
| Persistence — state | SQLite via `rusqlite` at `$XDG_STATE_HOME/omux/state.db` |
| Agent association | Auto-detect from `/proc/<pty-fg-pid>/cmdline`, matched against user-extensible TOML agent manifests |
| Turn detection — primary | Agent harness hooks → `omux-hook` helper → D-Bus → app |
| Turn detection — fallback | PTY output regex defined in agent manifest |
| Hook install (Claude Code) | One-time consented merge into `~/.claude/settings.json` on first run |
| Pane→hook correlation | `$OMUX_PANE_ID` env exported on pane spawn; shell rcfile snippet (added on first run, with consent) re-exports it so agents inherit it |
| Notification UX | In-app only: pane ring + tab badge + workspace badge in sidebar; clears on pane focus |
| Notification persistence | None: cleared on focus, no history view in v1 |
| Distribution | `cargo build --release`; no packaged artifacts in v1 |
| ralph-loop integration | Implementation methodology only; the app has no ralph-loop awareness |

---

## 2. Architecture

```
omux (single binary)                            omux-hook (small helper binary)
├── bin/omux            entry, sets up GTK app  ├── parses argv (stop/notification/sessionstart)
├── ui/                 widgets, layout, theme  ├── reads $CLAUDE_PANE_ID env (or fallback)
│   ├── window.rs        AdwApplicationWindow   └── sends D-Bus signal to omux session service
│   ├── sidebar.rs       workspaces list + drag
│   ├── pane_tree.rs     recursive split layout
│   ├── tab_bar.rs       tabs inside a pane
│   ├── status_overlay.rs ring + badge styling
│   └── css/style.css    theme + ring/badge classes
├── pane/
│   ├── mod.rs           Pane trait, PaneKind enum
│   ├── terminal.rs      libghostty FFI widget wrapper
│   ├── browser.rs       WebKitGTK widget wrapper
│   └── pty.rs           portable_pty wrapper, foreground-pid resolution
├── agent/
│   ├── manifest.rs      load + validate $XDG_CONFIG_HOME/omux/agents/*.toml
│   ├── detect.rs        polls pty.fg_pid → /proc/<pid>/cmdline → manifest match
│   ├── hook_installer.rs first-run consent + merge into ~/.claude/settings.json
│   ├── status_service.rs zbus service: receives signals from omux-hook
│   ├── output_parser.rs fallback: regex-on-pty-stream signal source
│   └── status.rs        pane status state machine (idle → running → needs-attention)
├── workspace/
│   ├── config.rs        TOML schema + load/save
│   └── state.rs         SQLite schema, migrations, repository
├── notify/
│   └── mod.rs           applies CSS classes + maintains badge counts
├── ipc/
│   └── dbus.rs          D-Bus service definition (interface name: org.omux.Status1)
└── ffi/
    └── ghostty.rs       libghostty bindings (build.rs links against libghostty.so)
```

All inter-module communication for transient events goes through `glib` channels or `tokio::sync::mpsc`. Persistent state changes go through the `workspace::state` repository.

---

## 3. Data Model

### 3.1 TOML — `$XDG_CONFIG_HOME/omux/workspaces/<slug>.toml`

```toml
name = "omux-dev"
root_folder = "/home/bcorder/Documents/Playground/omux"
pinned = true
order = 0

[[panes]]
id = "pane-1"
kind = "terminal"
working_dir = "."
command = []          # empty = user's $SHELL

[[panes]]
id = "pane-2"
kind = "browser"
url = "http://localhost:3000"

[layout]
root = { kind = "split", direction = "horizontal", ratio = 0.6, left = "pane-1", right = "pane-2" }
```

### 3.2 SQLite — `$XDG_STATE_HOME/omux/state.db`

```sql
CREATE TABLE workspaces (
  name TEXT PRIMARY KEY,
  last_opened INTEGER NOT NULL,
  display_order INTEGER NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE panes (
  pane_id TEXT PRIMARY KEY,
  workspace TEXT NOT NULL REFERENCES workspaces(name) ON DELETE CASCADE,
  kind TEXT NOT NULL,                 -- 'terminal' | 'browser'
  agent_type TEXT,                    -- nullable; 'claude-code', 'codex', or manifest name
  status TEXT NOT NULL DEFAULT 'idle',-- 'idle' | 'running' | 'needs-attention'
  last_seen_ts INTEGER NOT NULL
);

CREATE TABLE agent_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  pane_id TEXT NOT NULL REFERENCES panes(pane_id) ON DELETE CASCADE,
  ts INTEGER NOT NULL,
  kind TEXT NOT NULL,                 -- 'stop' | 'notification' | 'session-start' | 'regex-fallback'
  payload TEXT                        -- JSON blob, agent-specific
);

CREATE INDEX idx_agent_events_pane_ts ON agent_events(pane_id, ts);
```

Migrations live in `workspace/state.rs` and run on app start.

### 3.3 TOML — `$XDG_CONFIG_HOME/omux/agents/<name>.toml` (extensibility)

```toml
name = "claude-code"
display_name = "Claude Code"
process_patterns = ["^claude$", "^claude-code$"]   # matched against /proc/<pid>/comm and cmdline[0]

# Optional: if the harness supports hooks, declare what omux-hook should install.
[hooks.claude_code]
settings_path = "~/.claude/settings.json"
events = ["Stop", "Notification"]                  # which hook events to register

# Always: regex fallback patterns to match on PTY output stream
[fallback]
needs_attention_patterns = [
  "Press Enter to continue",
]
idle_timeout_secs = 0                              # 0 disables idle-only triggering
```

Shipping omux includes built-in manifests for `claude-code.toml` and `codex.toml`. Users drop additional `.toml` files in the dir to support new harnesses without code changes.

---

## 4. Critical Data Flows

### 4.1 Claude Code turn-end (hook path)

```
1. Pane spawned: omux exports OMUX_PANE_ID=<uuid> into the pty
   (and CLAUDE_PANE_ID=$OMUX_PANE_ID via the rcfile snippet installed on first run)
2. User types `claude` in the pane shell.
3. agent/detect polls /proc; sees `claude` is the pty foreground; tags pane.agent_type='claude-code'.
4. claude reads ~/.claude/settings.json → finds the omux-installed Stop hook:
      command = "omux-hook stop --pane $CLAUDE_PANE_ID"
5. On turn end, Claude runs the hook. omux-hook calls D-Bus method:
      org.omux.Status1.MarkNeedsAttention(pane_id="<uuid>", kind="stop", payload="{}")
6. agent/status_service receives signal → status state machine → 'needs-attention'.
7. notify/ applies pane-ring CSS class and increments tab/workspace badges.
8. User focuses the pane → focus handler clears the class + decrements badges.
```

### 4.2 Fallback path (Codex CLI or unknown harness without hooks)

```
1–3. Same as above; manifest matched, pane tagged.
4.  agent/output_parser is attached to the pane's PTY output stream.
5.  Each output chunk is fed through the manifest's regex patterns.
6.  On match → same D-Bus method as the hook path (called locally).
7.  Notification rendered identically — caller-agnostic past step 6.
```

### 4.3 First-run consent flow for hook installation

On startup, if `$XDG_STATE_HOME/omux/install.toml` does not record `claude_settings_merged = true`:

1. Show GTK dialog: "omux can install Stop/Notification hooks into your Claude Code settings (`~/.claude/settings.json`) so terminals light up when Claude finishes a turn. omux will add two hook entries and back up the file to `~/.claude/settings.json.omux-backup`. Proceed?"
2. On accept: read existing JSON, merge in hooks (idempotent — keyed by a sentinel `"_omux_managed": true` field), write atomically. Same for the user shell rcfile snippet (`$ZDOTDIR/.zshrc` or `~/.bashrc`).
3. Record `claude_settings_merged = true` and timestamp.
4. On decline: degrade to fallback-only mode; show a status-bar warning that Claude detection will be regex-based and less accurate.

Uninstall path: `omux --uninstall-hooks` restores backup files.

---

## 5. Component Detail

### 5.1 `pane/terminal.rs` (libghostty)

- Wraps a `gtk::Widget` exposed by libghostty's GTK4 embed API.
- Owns the PTY master fd; spawns the user's `$SHELL` with `OMUX_PANE_ID` injected.
- Exposes a `tokio` channel for the raw PTY output stream (for `output_parser`).
- Exposes `fg_pid()` via `tcgetpgrp` for `agent/detect`.

If libghostty FFI proves unstable during M1, fall back to `vte4-rs`. This is the only documented architecture escape hatch; everything else is committed.

### 5.2 `pane/browser.rs` (WebKitGTK)

- Wraps `webkit6::WebView` plus a small URL bar + back/forward buttons.
- Per-workspace cookie jar (`WebsiteDataManager` with a workspace-scoped data dir).
- No notification involvement — browser panes never raise `needs-attention`.

### 5.3 `ui/pane_tree.rs`

- Layout primitive: `PaneNode = Leaf(PaneId) | Split { direction, ratio, a, b }`.
- Split via `gtk::Paned`; per-pane tabs via `gtk::Notebook` inside a leaf.
- Keyboard shortcuts: `Ctrl+Shift+D` vertical split, `Ctrl+Shift+E` horizontal split, `Alt+Arrows` navigate.

### 5.4 `ui/sidebar.rs`

- `gtk::ListBox` of workspaces.
- `gtk::DragSource` + `gtk::DropTarget` for reorder; pinned items grouped at top, drag locked across the pin/unpin boundary.
- Right-click context menu: rename, pin/unpin, close, delete.
- Badge: small `Adw::Bin` overlay showing count of `needs-attention` panes inside that workspace.

### 5.5 `ipc/dbus.rs`

- Service name: `org.omux.Status1`.
- Interface methods:
  - `MarkNeedsAttention(pane_id: s, kind: s, payload: s)`
  - `MarkRunning(pane_id: s)`
  - `MarkIdle(pane_id: s)`
  - `Ping() -> s` (used by `omux-hook` to verify omux is running; if absent, hook logs to a fallback file the app reads on next start)

### 5.6 `omux-hook` (separate binary)

- Tiny. Argv parsing only, no async runtime.
- `omux-hook stop --pane $CLAUDE_PANE_ID` → opens session D-Bus, calls `MarkNeedsAttention`.
- Exits silently on success; on failure (omux not running), writes a JSON line to `$XDG_STATE_HOME/omux/pending-events.jsonl`.
- On omux startup, drains `pending-events.jsonl` so missed signals are not lost.

---

## 6. Notification State Machine

```
                   detect: agent process appears
   ┌──────┐ ─────────────────────────────────────► ┌──────────┐
   │ idle │                                        │ running  │
   └──────┘ ◄───────────────────────────────────── └──────────┘
                  detect: agent process gone               │
                                                           │ hook/regex: stop/notification
                                                           ▼
                                                  ┌─────────────────┐
                          pane focus / typing ◄── │ needs-attention │
                                                  └─────────────────┘
```

- Transitions logged to `agent_events`.
- Pane focus clearing is debounced 200ms (avoid clearing on transient focus).
- Badge counts on tab + workspace are derived from `panes.status` aggregations.

---

## 7. Error Handling

| Failure mode | Behavior |
|---|---|
| libghostty crash in one pane | Pane shows red overlay "terminal died"; other panes unaffected; user can close pane |
| D-Bus name acquisition fails | App still runs; hooks fall back to `pending-events.jsonl` mechanism |
| SQLite write failure | Logged; app continues with in-memory state; user-visible warning in status bar |
| TOML parse failure in workspace file | Skip that workspace, log, surface in a "broken workspaces" sidebar section |
| Hook merge failure (read-only `~/.claude/settings.json` etc.) | Roll back from backup, disable hook mode, fall back to regex |
| `omux-hook` invoked but app not running | Append event to `pending-events.jsonl`; replayed on next start |
| Agent manifest regex catastrophic backtrack | Compile with `regex` crate (no backtracking); reject manifests with unbounded patterns at load |

All long-lived tasks logged via `tracing` to stderr + a rolling file at `$XDG_STATE_HOME/omux/logs/omux.log`.

---

## 8. Testing Strategy

| Layer | Approach |
|---|---|
| `agent/manifest` | Unit tests on TOML parse + regex compile + match cases |
| `agent/status` state machine | Property tests with `proptest`: invariant that state transitions are valid |
| `agent/detect` | Inject a fake `/proc` reader; verify cmdline → manifest mapping |
| `workspace/state` | SQLite migrations tested against fresh DB + each prior schema version |
| `output_parser` | Fixture-based: feed canned PTY captures, assert events |
| Hook integration end-to-end | `tests/e2e/` — spawn a `fake-claude` binary that calls real `omux-hook`, assert app sees signal via D-Bus |
| GTK UI | Manual smoke checklist (CI cannot run GTK4 reliably headless without xvfb + working WebKit) |

CI runs `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. GTK manual smoke is a documented release checklist, not CI.

---

## 9. Verification (end-to-end)

After implementing all milestones the following must pass manually:

1. `cargo build --release` produces `target/release/omux` and `target/release/omux-hook`.
2. Launching `omux` shows an empty window with a "Create workspace" button.
3. Create workspace tied to `~/Documents/Playground/omux`; sidebar shows it pinned-able.
4. Split workspace horizontally: terminal + terminal. Vertical split inside one of them. Tabs work.
5. Replace one pane with a browser pane pointed at `http://localhost:3000`.
6. In a terminal pane, run `claude`. Within 2 seconds the pane is tagged "Claude Code" (visible in pane header).
7. Trigger a Claude turn that ends. Within 1 second of turn end, pane ring lights up, tab badge appears, workspace badge in sidebar increments.
8. Focus the pane → ring + badges clear.
9. Quit and relaunch: workspace + layout + pinned state restored; pane statuses reset to `idle`.
10. `omux --uninstall-hooks` restores `~/.claude/settings.json` and shell rcfile from backups.

---

## 10. Implementation Milestones (for /ralph-loop)

Each milestone is the granularity of one ralph-loop iteration target. The loop should run tests + manual smoke at each boundary before advancing.

| # | Milestone | Done when |
|---|---|---|
| **M0** | Scaffold | `cargo new`, workspace with `omux` + `omux-hook` crates, GTK4 dep, empty window opens |
| **M1** | Single terminal pane via libghostty | Window with one libghostty pane running `$SHELL`; if libghostty integration blocks > 1 ralph cycle, swap to `vte4-rs` and document |
| **M2** | Split panes + per-pane tabs | h/v splits, tab bar in each leaf, keyboard shortcuts, drag-resize splits |
| **M3** | Workspaces with TOML + SQLite | Sidebar, create/rename/delete/pin/reorder workspaces; layout persists across restart |
| **M4** | Agent detection + hook + notification | Manifests load; auto-detect tags panes; first-run hook install dialog; `omux-hook` works; pane ring + badges render and clear correctly |
| **M5** | Embedded browser pane | A pane can be a `webkit6::WebView` with URL bar; per-workspace data isolation |
| **M6** | Polish | Right-click context menus (copy/paste/split/clear), animated sidebar collapse, drag-drop pin boundary handling, theming via CSS file |

Stretch (not v1): tray icon, desktop libnotify integration, scrollback persistence, sound, multi-window, search.

---

## 11. Files to Create

```
Cargo.toml                           workspace manifest
omux/Cargo.toml                      app crate
omux/build.rs                        link libghostty
omux/src/main.rs                     bin/omux entry
omux/src/ui/                         see §2
omux/src/pane/
omux/src/agent/
omux/src/workspace/
omux/src/notify/
omux/src/ipc/
omux/src/ffi/
omux/resources/agents/               default manifests (claude-code.toml, codex.toml)
omux/resources/style.css             theme + ring/badge classes
omux-hook/Cargo.toml
omux-hook/src/main.rs                bin/omux-hook
tests/e2e/fake_claude.rs             fake harness binary for end-to-end test
README.md                            build + install + uninstall instructions
```

No existing repository files to reuse — `/home/bcorder/Documents/Playground/omux` is empty.

---

## 12. Third-Party Crates (anchor list)

| Crate | Purpose |
|---|---|
| `gtk4`, `libadwaita`, `glib`, `gio`, `pango` | GTK4 / Adwaita UI |
| `webkit6` | embedded browser pane |
| `tokio` | non-UI async I/O |
| `zbus` | D-Bus session bus |
| `rusqlite` (`bundled` feature) | SQLite |
| `serde`, `toml` | config + manifests |
| `regex` | output-parser fallbacks (no backtracking, safe by default) |
| `tracing`, `tracing-subscriber`, `tracing-appender` | structured logging |
| `portable-pty` | PTY abstraction if libghostty's PTY isn't sufficient |
| `directories` | XDG paths |
| `uuid` | pane IDs |
| `notify` | inotify watch on fallback `pending-events.jsonl` |
| `anyhow`, `thiserror` | error types |
| `proptest` | property tests for state machine |

libghostty itself is a system library linked via `build.rs`. Build prerequisites documented in README.

---

## 13. Open Risks

1. **libghostty FFI maturity.** Documented escape hatch: switch to `vte4-rs` at M1 if blocked. No GPU rendering in that case.
2. **WebKitGTK 6 packaging on Arch / Cachy.** Likely fine (system packages present) but worth a M0 smoke step that imports `webkit6` and instantiates a `WebView` in a throwaway test before committing to M5.
3. **First-run hook merge into `~/.claude/settings.json`.** The merge is idempotent and reversible, but the user's settings might already be heavily customized. Backup-before-write + sentinel field is the mitigation; if the user declines consent, omux silently degrades to regex fallback for Claude Code.
4. **Auto-detect race window.** If `claude` runs and finishes a turn before omux's detect-poll catches up (poll interval default 500ms), the first hook signal might arrive against an un-tagged pane. `omux-hook` always carries the pane_id explicitly from env, so the signal lands correctly regardless; the only consequence is the pane header may briefly show "Shell" instead of "Claude Code." Acceptable.
