# Contributing to omux

Thanks for taking a look. omux is a small project and PRs are
welcome.

## Local dev loop

```sh
# Build prereqs (see README.md "Build from source" for other distros)
sudo pacman -S --needed gtk4 libadwaita webkitgtk-6.0 vte4 pkgconf

# Lint + test + build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

CI runs the same four commands on every PR (see
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml)). If they
pass locally, CI is unlikely to surprise you.

## Commit style: Conventional Commits

Every commit on `main` follows the
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
style. This isn't decorative — `cargo-release` and the release notes
generator read commit subjects to categorize what changed.

Format:

```
<type>: <subject>

<optional body explaining "why">

<optional footer, e.g. "Closes #123">
```

`<type>` is one of:

| Type     | Use for                                                         |
|----------|-----------------------------------------------------------------|
| `feat`   | A user-facing new feature                                       |
| `fix`    | A user-facing bug fix                                           |
| `perf`   | A performance improvement                                       |
| `refactor` | Code shuffling with no behavior change                        |
| `docs`   | README, design docs, code comments                              |
| `test`   | Test-only changes                                               |
| `build`  | Build system, Cargo, CI config                                  |
| `chore`  | Maintenance: bumps, file moves, version cuts                    |

Examples:

```
feat: per-leaf + button popover with split + new-tab actions
fix: hoist manager.borrow_mut() to avoid RefMut lifetime extension
docs: rewrite README with quick-install + verify-artifact one-liner
build: pin slsa-github-generator to v2.0.0
chore: release 0.1.0
```

Breaking changes get a `!` and an explanatory footer:

```
feat!: change workspace TOML schema to nested layout nodes

BREAKING CHANGE: existing $XDG_CONFIG_HOME/omux/workspaces/*.toml
files will fail to load. Run `omux --migrate-workspaces` before
upgrading.
```

## What goes in CHANGELOG.md

The `## [Unreleased]` section at the top of [`CHANGELOG.md`](./CHANGELOG.md)
is the staging area for the next release. When you land a user-facing
change, add a one-line bullet there. Categorize loosely under
**Added** / **Changed** / **Fixed** / **Removed** /
**Security** / **Internal** as you go.

At release time, `cargo-release` renames `## [Unreleased]` to
`## [X.Y.Z] — YYYY-MM-DD` and the `release.yml` workflow extracts
that section verbatim into the GitHub release body. So write it for
end users, not for git archaeologists.

## Releases

If you have merge rights and need to cut a release, see
[`docs/RELEASING.md`](./docs/RELEASING.md). The TL;DR is:

```sh
cargo release minor --execute
```

…then watch CI handle the rest.

## Security issues

Don't open a public issue. See [`SECURITY.md`](./SECURITY.md) for the
private report channel.
