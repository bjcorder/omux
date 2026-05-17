# omux

Multi-terminal Linux workspace with agent-attention notifications. Inspired
by [cmux](https://github.com/manaflow-ai/cmux) (macOS) and
[limux](https://github.com/am-will/limux) (Linux), with added in-app
notification badges that light up panes/tabs/workspaces when an AI coding
agent (Claude Code, Codex CLI, or any user-declared harness) finishes a turn
or asks for input.

Design and milestones: `docs/design.md` (canonical, evolves with the
project; `~/.claude/plans/i-want-to-use-calm-llama.md` is the original
approved-plan snapshot from 2026-05-17).

## Status

See `PROGRESS.md` for current milestone state.

## Build

System dependencies (Arch / Cachy OS):

```
sudo pacman -S --needed gtk4 libadwaita webkitgtk-6.0 vte4 pkgconf
```

Then:

```
cargo build --release
./target/release/omux
```

## Layout

```
omux/        the app binary (GTK4 + libadwaita UI)
omux-hook/   small helper invoked by agent hooks; talks to omux via D-Bus
tests/e2e/   end-to-end integration tests (fake-claude harness, etc.)
docs/        design & implementation plan
```

## License

MIT.
