<h1 align="center">omux</h1>

<p align="center">
  A Linux-native, multi-terminal workspace with
  <strong>in-app notification rings when your AI coding agent finishes
  a turn or needs input</strong>.
</p>

<p align="center">
  <a href="https://github.com/bjcorder/omux/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/bjcorder/omux/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/bjcorder/omux/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/bjcorder/omux?display_name=tag&sort=semver"></a>
  <a href="https://slsa.dev"><img alt="SLSA Build L3" src="https://slsa.dev/images/gh-badge-level3.svg"></a>
  <a href="./LICENSE"><img alt="license: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
</p>

<p align="center">
  Inspired by
  <a href="https://github.com/manaflow-ai/cmux">cmux</a> (macOS) and
  <a href="https://github.com/am-will/limux">limux</a> (Linux), but
  with first-class notification plumbing for Claude Code, Codex CLI,
  and any other agent harness you can describe with a TOML manifest.
</p>

```
┌─ default  ──┐ ┌──────────────────────────────────────────┐
│ ● testing 1 │ │ ! shell ✕  shell ✕                +   │
│   default   │ ├──────────────────────────────────────────┤
└─────────────┘ │ user@host:~/proj $ claude                │
                │ ✱ Thinking…                              │
                │ ▓▓▓▓▓▓▓▓▓▓▓▓▓ ← needs-attention ring   │
                └──────────────────────────────────────────┘
```

When an agent fires its `Stop` hook (or matches a configured
PTY-output regex), the pane gains an accent-colored ring, the tab
label gets a notification badge, and the sidebar workspace row picks
up an unread count. Focusing the pane clears all three.

## Highlights

- **Recursive split layout** — `Ctrl+Shift+D` / `Ctrl+Shift+E` for
  horizontal / vertical splits, each leaf can hold multiple tabs,
  drag-resize splits, directional `Alt+Arrow` navigation.
- **Embedded browser pane** (`Ctrl+Shift+B`) with per-workspace
  cookie isolation, URL bar, back/forward/reload. Useful for the
  localhost dev-server-next-to-the-agent flow.
- **Per-leaf `+` menu** for adding a new terminal tab / browser tab,
  or splitting the leaf, without learning the keyboard shortcuts.
- **Per-tab `×` close button** that also collapses the leaf out of
  the tree if it was the last tab (only the root leaf refuses, to
  avoid empty workspaces).
- **Workspaces with persistent state**. Each workspace's split
  layout, terminals, scrollback, agent state, and browser pages
  survive switches *and* full app restarts. (Layout structure is
  persisted to TOML; running shells live in memory while omux runs.)
- **Resizable sidebar** with width persisted across restarts.
- **Notification routing** for Claude Code, Codex CLI, and
  user-supplied harnesses described by a TOML manifest. Hook path
  + PTY-output regex fallback. Hooks are reversibly installed into
  `~/.claude/settings.json` with a backup file; surgical uninstall
  preserves other entries.

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

All milestones landed plus a substantial round of post-M6
stabilization (multi-live-tree workspaces, per-leaf `+` menu, per-tab
`×` close, leaf-collapse-on-empty, resizable sidebar, clean
shutdown, install scripts + .desktop). 52 automated tests cover the
persistence layer, state machine, manifest matching, hook installer,
and the omux-hook ↔ socket wire contract; the GTK widgets are
verified via the manual smoke checklist in [`PROGRESS.md`](./PROGRESS.md).

See [`CHANGELOG.md`](./CHANGELOG.md) for the per-milestone breakdown.

## Releases

Prebuilt binaries + provenance attestations are published on the
[GitHub releases page](https://github.com/bjcorder/omux/releases).
Each release contains:

- `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` — release binaries +
  `.desktop` + icon + `install.sh` + LICENSE + README. Dynamically
  linked; install the system packages listed under *Build from
  source* below.
- `omux-X.Y.Z-source.tar.gz` — clean source tree of the tag.
- `*.tar.gz.sha256` — SHA256 sidecar for each tarball.
- `omux-X.Y.Z.intoto.jsonl` — **SLSA Build Level 3** in-toto
  attestation, Sigstore-signed and logged in
  [Rekor](https://search.sigstore.dev/).

### Verify before you run

```sh
TAG=v0.1.0   # the release you want
gh release download "$TAG" -R bjcorder/omux \
    -p '*.tar.gz' -p '*.intoto.jsonl'
slsa-verifier verify-artifact \
    --provenance-path "omux-${TAG#v}.intoto.jsonl" \
    --source-uri      github.com/bjcorder/omux \
    --source-tag      "$TAG" \
    "omux-${TAG#v}-x86_64-unknown-linux-gnu.tar.gz"
```

See [`docs/VERIFYING.md`](./docs/VERIFYING.md) for the full walk-through
(what this proves, what it doesn't, what to do on failure).

## Quick install

```sh
git clone https://github.com/bjcorder/omux.git
cd omux
./scripts/install.sh
```

This runs `cargo build --release` and drops `omux` + `omux-hook`
into `~/.local/bin/`, a `.desktop` entry into
`~/.local/share/applications/`, and an icon into
`~/.local/share/icons/hicolor/scalable/apps/`. After it finishes,
launch omux from your application menu (or `omux` if `~/.local/bin`
is on your PATH).

Pass `--system` for a `/usr/local` + `/usr/share` install (needs
sudo). To remove: `./scripts/uninstall.sh` (preserves config + state).

The same `install.sh` ships inside the release tarball. If you
extract the tarball and run `./install.sh` from inside it, the
script skips the cargo build and just copies the pre-built binaries
into place.

## Build from source

Build prerequisites: **GTK 4.18+**, **libadwaita 1.7+**,
**WebKitGTK 6.0**, **VTE 4 (gtk4 build)**, **pkg-config**,
**Rust 1.85+**.

### Arch / Cachy OS

```sh
sudo pacman -S --needed gtk4 libadwaita webkitgtk-6.0 vte4 pkgconf
cargo build --release
```

### Debian / Ubuntu

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev \
                 libvte-2.91-gtk4-dev pkg-config
cargo build --release
```

Binaries:

```
target/release/omux         the GUI app
target/release/omux-hook    the helper that agent hooks invoke
```

If you skipped the install script, drop both somewhere on `$PATH`
(e.g. `~/.local/bin/`) so Claude Code's hook commands can find
`omux-hook`.

## First launch

```sh
omux
```

On first launch omux:

1. Seeds a workspace called `default` rooted at `$HOME`.
2. Spawns your `$SHELL` in a single terminal pane (with
   `OMUX_PANE_ID=<uuid>` exported into the env).
3. Pops up a one-time **Enable Claude Code notifications?** dialog.
   Accept to merge omux's `Stop` + `Notification` hooks into
   `~/.claude/settings.json` (a `.omux-backup` of the original is
   saved alongside). Decline to use omux without hook integration —
   agent detection still works via process matching, and the
   regex-output fallback still catches things like `Press Enter to
   continue`.

To reverse the install at any time:

```sh
omux --uninstall-hooks
```

This surgically removes only omux's entries (the ones carrying the
`_omux_managed: true` sentinel) so anything else you or Claude Code
have added since survives untouched.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl + Shift + D` | Split focused leaf side-by-side (h-split) |
| `Ctrl + Shift + E` | Split focused leaf top/bottom (v-split) |
| `Ctrl + T` | New terminal tab in focused leaf |
| `Ctrl + Shift + B` | New browser tab in focused leaf |
| `Ctrl + W` | Close current tab. If it was the last tab in a non-root leaf, the leaf collapses out of the tree. |
| `Ctrl + Tab` / `Ctrl + Shift + Tab` | Cycle focus through leaves |
| `Alt + Arrow` | Focus adjacent leaf in that direction |
| `Ctrl + Shift + C` | Copy selection from active terminal |
| `Ctrl + Shift + V` | Paste into active terminal |

## Mouse interactions

- **Per-leaf `+` button** (right end of each tab bar) — opens a
  popover with **New tab → Terminal / Browser** and **Split this
  pane → Side-by-side / Top-bottom**. Clicking from leaf A targets
  leaf A regardless of which leaf currently has focus.
- **Per-tab `×` button** — closes that tab; collapses the leaf if it
  was the last tab in a non-root leaf.
- **Right-click on a terminal pane** — Copy / Paste / Clear /
  Split-h / Split-v / New tab / Close tab.
- **Right-click on a sidebar workspace** — Rename / Pin / Delete.
- **Drag sidebar rows** to reorder workspaces. Drag the **sidebar
  divider** to resize the sidebar (width persists across restarts).

## Workspaces

Each workspace is a folder-bound named layout. Click the
`+ New workspace` button at the bottom of the sidebar; the workspace
inherits the current working directory as its root.

Workspaces persist their state in two layers:

1. **In-memory across switches** — switching to another workspace
   and back leaves the first workspace's terminals, scrollback,
   agent state, and browser pages untouched. omux keeps one
   `PaneTree` alive per opened workspace for the app's lifetime.
2. **On disk across restarts** — each workspace's split shape, tab
   kinds, and browser URLs are saved to
   `$XDG_CONFIG_HOME/omux/workspaces/<slug>.toml` on window close.
   Running shells aren't persistable (PTYs are kernel state), so
   they get re-spawned in the same shape on next launch.

Cookies / local storage / cache for browser panes are scoped
per workspace (one `webkit6::NetworkSession` per workspace) so
logging into the same site from two workspaces gives you two
independent sessions.

## Agents

omux ships built-in manifests for Claude Code and Codex CLI. To
support another harness, drop a TOML file at
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
# omux-hook reads $OMUX_PANE_ID from its inherited env (omux injects
# it when spawning each pane's shell, so it propagates shell →
# harness → hook automatically).

# Output-regex fallback (optional). For harnesses without hooks,
# omux scans the last few rows of terminal output each poll cycle
# and fires a needs-attention event when any pattern matches.
# Matches are debounced 2s and deduplicated by exact matched text.
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
| `$XDG_STATE_HOME/omux/state.db` | SQLite: workspace order, last-opened timestamps, active workspace, sidebar width |
| `$XDG_RUNTIME_DIR/omux/control.sock` | Unix socket omux-hook writes to (cleaned up on shutdown) |
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

* `--pane <uuid>` — explicit pane id (otherwise reads
  `$CLAUDE_PANE_ID`, then `$OMUX_PANE_ID` from env).
* `--payload <json>` — arbitrary JSON passed through to omux for
  debugging.

If omux isn't running, the event is appended to
`$XDG_RUNTIME_DIR/omux/pending-events.jsonl`; the file is drained
the next time omux starts.

Smoke test the IPC without a real Claude:

```sh
echo '{"kind":"stop","pane_id":"<uuid-from-the-pane>"}' \
  | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/omux/control.sock
```

## Architecture

See [`docs/design.md`](./docs/design.md) for the full design spec
(modules, data flow, persistence schema, error handling, testing
strategy). [`PROGRESS.md`](./PROGRESS.md) tracks per-milestone status
and the manual smoke checklist. [`CHANGELOG.md`](./CHANGELOG.md) has
the per-milestone breakdown.

High level:

```
omux (single binary)                          omux-hook (helper)
├── ui/         Adwaita sidebar + content     ├── argv: stop|notification|…
├── pane/       Recursive split tree of       └── unix socket → omux
│               TerminalPane / BrowserPane
├── agent/      Manifests, process detection,
│               status state machine, hook
│               installer
├── workspace/  TOML config + SQLite state +
│               WorkspaceManager + live
│               PaneTree map
├── ipc/        Unix socket service + event
│               types
└── main.rs     CSS load, signal handlers,
                CLI flags, AppShell wiring
```

## Tests + checks

```sh
cargo test --workspace                  # 52 tests (44 omux + 8 omux-hook integration)
cargo clippy --workspace -- -D warnings # lint
cargo fmt --check                       # format
```

The omux-hook integration tests (`omux-hook/tests/ipc_end_to_end.rs`)
stand up a Unix-socket listener in-process, run omux-hook as a
subprocess pointed at it, and verify the JSON wire contract. They
catch any schema drift between the helper and the omux service
without needing a GTK display.

GTK widgets aren't covered by automated tests (CI can't easily run a
real display); the manual smoke checklist in `PROGRESS.md` covers
the UI interactions.

## Contributing

PRs welcome. Conventional commits are required (`cargo-release` and
the release notes pipeline parse the subject lines). See
[`CONTRIBUTING.md`](./CONTRIBUTING.md) for the format. Maintainers
cutting a release should follow [`RELEASING.md`](./RELEASING.md).

## Security

See [`SECURITY.md`](./SECURITY.md) for the threat model and how to
privately report a vulnerability. Release artifacts carry SLSA L3
provenance; see [`docs/VERIFYING.md`](./docs/VERIFYING.md) for how to
verify them before running.

## License

MIT. See [`LICENSE`](./LICENSE).
