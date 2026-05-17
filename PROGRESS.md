# omux — Milestone Progress

Reference: `docs/design.md` §10.

| # | Milestone | Status | Notes |
|---|---|---|---|
| **M0** | Scaffold | ✅ done | Cargo workspace, empty Adwaita window, fmt/clippy/test clean |
| **M1** | Single terminal pane (vte4-rs) | ⏸️ blocked | Needs `vte4` system package installed (`sudo pacman -S vte4`). libghostty → vte4 swap recorded in design.md §1, §5.1, §13.1 |
| **M2** | Split panes + per-pane tabs | ⏳ pending | Depends on M1 |
| **M3** | Workspaces (TOML + SQLite) | ⏳ pending | Mostly independent of M1 but easier once panes exist |
| **M4** | Agent detection + hook + notification | ⏳ pending | Depends on M1 (PTY hookup) |
| **M5** | Embedded browser pane | ⏳ pending | Independent of terminal panes; can land any time after M0 |
| **M6** | Polish | ⏳ pending | Depends on M2–M5 |

## Action items for user

- **Install `vte4`** to unblock M1:
  ```
  sudo pacman -S --needed vte4
  ```
  Verify with `pkg-config --modversion vte-2.91-gtk4` (expect ≥ 0.84).

## Decisions log

- **2026-05-17 (M0)** — Swapped terminal renderer from libghostty to vte4-rs. libghostty is not packaged as a standalone shared library on Arch / Cachy OS, and building Ghostty from source to extract it is out of scope. Per design.md §13.1 escape hatch.
