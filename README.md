# omux

A Linux-native, multi-terminal workspace with **in-app notification rings
when your AI coding agent finishes a turn or needs input** — inspired by
[cmux](https://github.com/manaflow-ai/cmux) (macOS) and
[limux](https://github.com/am-will/limux) (Linux), but with first-class
notification plumbing for Claude Code, Codex CLI, and any other agent
harness you can describe with a TOML manifest.

```
┌─ default ──┐ ┌─────────────────────────────────────────┐
│ ● default  │ │ ! shell                                │
│   work     │ ├─────────────────────────────────────────┤
│   web      │ │ user@host:~/proj $ claude                │
└────────────┘ │ Thinking…                                │
               │ ▓▓▓▓▓▓▓▓▓ ← needs-attention ring        │
               └─────────────────────────────────────────┘
```

When an agent fires its `Stop` hook (or matches a configured PTY-output
regex), the pane gains an accent-colored ring, the tab label gets a
notification badge, and the sidebar workspace row picks up an unread
count. Focusing the pane clears all three.

## Status

| Milestone | What it lands |
|---|---|
| M0 | Cargo workspace + empty Adwaita window |
| M1 | VTE terminal pane spawning `$SHELL` with `OMUX_PANE_ID` env |
| M2 | Recursive pane tree (h/v splits, per-leaf tabs, keyboard nav) |
| M3 | Workspace persistence (TOML + SQLite) + sidebar UI |
| M4 | Agent detection (process + regex), Unix-socket IPC, hook installer |
| M5 | WebKitGTK browser pane with per-workspace cookie isolation |
| M6 | Right-click menus, badges, drag-reorder, directional nav, polish |

All milestones are landed. 44 unit tests cover the persistence layer,
state machine, manifest matching, hook installer, and other pure-Rust
modules. The GUI is verified via the manual smoke checklist in
[`PROGRESS.md`](./PROGRESS.md).

## Build

### Arch / Cachy OS

```sh
sudo pacman -S --needed gtk4 libadwaita webkitgtk-6.0 vte4 pkgconf
cargo build --release
```

### Other distros

You need development packages for **GTK 4.18+**, **libadwaita 1.7+**,
**WebKitGTK 6.0**, **VTE 4 (gtk4 build)**, and **pkg-config**. On
Debian/Ubuntu these are roughly:

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev \
                 libvte-2.91-gtk4-dev pkg-config
```

The binaries are produced at:

```
target/release/omux         # the GUI app
target/release/omux-hook    # the tiny helper that agent hooks invoke
```

Install them somewhere on `$PATH` (e.g. `~/.local/bin/`) so Claude Code's
hook config can find `omux-hook`.

## First launch

```sh
./target/release/omux
```

On first launch omux:

1. Seeds a workspace called `default` rooted at `$HOME`.
2. Spawns your `$SHELL` in a single terminal pane (with
   `OMUX_PANE_ID=<uuid>` exported into the env).
3. Pops up a one-time **Enable Claude Code notifications?** dialog. Accept
   to merge omux's `Stop` + `Notification` hooks into
   `~/.claude/settings.json`. Decline to use omux without hook
   integration — agent detection still works via process matching, and
   the regex-output fallback still catches things like `Press Enter to
   continue`.

A backup of your original `settings.json` is saved alongside as
`settings.json.omux-backup`. You can reverse the install any time with:

```sh
omux --uninstall-hooks
```

The uninstaller surgically removes only omux's entries (entries carrying
the `_omux_managed: true` sentinel) so anything else you or Claude Code
added since install survives untouched.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl + Shift + D` | Split focused leaf side-by-side (h-split) |
| `Ctrl + Shift + E` | Split focused leaf top/bottom (v-split) |
| `Ctrl + T` | New terminal tab in focused leaf |
| `Ctrl + Shift + B` | New browser tab in focused leaf |
| `Ctrl + W` | Close current tab (refuses if it's the last in the leaf) |
| `Ctrl + Tab` / `Ctrl + Shift + Tab` | Cycle focus through leaves |
| `Alt + Arrow` | Focus adjacent leaf in that direction |
| `Ctrl + Shift + C` | Copy selection from active terminal |
| `Ctrl + Shift + V` | Paste into active terminal |

Right-click a terminal pane for **Copy / Paste / Clear / Split-h / Split-v
/ New tab / Close tab**. Right-click a sidebar workspace for **Rename /
Pin / Delete**. Drag sidebar rows to reorder them.

## Workspaces

Each workspace is a folder-bound named layout. Click `+ New workspace` in
the sidebar to create one; pick a name and the workspace inherits the
current working directory as its root. Workspaces persist their layout
across restarts — switch between two workspaces and they each remember
their split shape and tab kinds independently.

Cookies / local storage / cache for browser panes are scoped per
workspace (one `webkit6::NetworkSession` per workspace) so logging into
the same site from two workspaces gives you two independent sessions.

## Agents

omux ships built-in manifests for Claude Code and Codex CLI. To support
another harness, drop a TOML file at
`$XDG_CONFIG_HOME/omux/agents/<name>.toml`:

```toml
name = "my-agent"
display_name = "My Agent"

# Regexes matched against /proc/<pid>/comm for the PTY's foreground
# process group. First match wins.
process_patterns = ["^my-agent$", "^myagentctl$"]

# Hook integration (optional). If your harness can run a command at
# turn-end, configure it to run:
#   omux-hook stop
# omux-hook reads $OMUX_PANE_ID from its inherited env (omux injects it
# when spawning each pane's shell, so it propagates shell → harness →
# hook automatically).

# Output-regex fallback (optional). For harnesses without hooks, omux
# scans the last few rows of terminal output each poll cycle and fires
# a needs-attention event when any pattern matches. Matches are
# debounced 2s and deduplicated by exact matched text.
[fallback]
needs_attention_patterns = [
  "Press Enter to continue",
  "Waiting for your response",
]
idle_timeout_secs = 0
```

Built-in manifests are shadowed by user files of the same `name`.

## File locations

| Path | Contents |
|---|---|
| `$XDG_CONFIG_HOME/omux/workspaces/*.toml` | One file per workspace (name, root, pinned, layout snapshot). User-readable. |
| `$XDG_CONFIG_HOME/omux/agents/*.toml` | User-supplied agent manifests (override built-ins) |
| `$XDG_CONFIG_HOME/omux/web/<slug>/` | Per-workspace WebKit data dir (cookies, local storage) |
| `$XDG_STATE_HOME/omux/state.db` | SQLite: workspace order, last-opened timestamps, active workspace |
| `$XDG_RUNTIME_DIR/omux/control.sock` | Unix socket omux-hook writes to |
| `$XDG_RUNTIME_DIR/omux/pending-events.jsonl` | Hook events buffered while omux is offline; drained on next start |
| `~/.claude/settings.json` | Claude Code config; omux merges Stop + Notification entries (sentinel-tagged, reversible) |

## omux-hook

The helper is invoked by agent hooks (or by you, for testing) like:

```sh
omux-hook stop
omux-hook notification
omux-hook session-start
omux-hook regex-fallback
```

Optional flags:

* `--pane <uuid>` — explicit pane id (otherwise reads `$CLAUDE_PANE_ID`,
  then `$OMUX_PANE_ID` from env).
* `--payload <json>` — arbitrary JSON passed through to omux for
  debugging.

If omux isn't running, the event is appended to
`$XDG_RUNTIME_DIR/omux/pending-events.jsonl`; the file is drained the
next time omux starts.

Smoke test the IPC without a real Claude:

```sh
echo '{"kind":"stop","pane_id":"<uuid-from-the-pane>"}' \
  | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/omux/control.sock
```

## Architecture

See [`docs/design.md`](./docs/design.md) for the full design spec
(modules, data flow, persistence schema, error handling, testing
strategy). [`PROGRESS.md`](./PROGRESS.md) tracks per-milestone status
and the manual smoke checklist.

High level:

```
omux (single binary)                            omux-hook (helper)
├── ui/         Adwaita sidebar + content       ├── argv: stop|notification|…
├── pane/       Recursive split tree of         └── unix socket → omux
│               TerminalPane / BrowserPane
├── agent/      Manifests, process detection,
│               status state machine, hook
│               installer
├── workspace/  TOML config + SQLite state +
│               WorkspaceManager
├── ipc/        Unix socket service + event
│               types
└── main.rs     CSS load, signal handlers,
                CLI flags, AppShell wiring
```

## Tests + checks

```sh
cargo test --workspace                  # 44 unit tests
cargo clippy --workspace -- -D warnings # lint
cargo fmt --check                       # format
```

GTK widgets aren't covered by automated tests (CI can't run a real
display); the manual smoke checklist in `PROGRESS.md` exists for that.

## License

MIT.
