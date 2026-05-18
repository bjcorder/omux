# Releasing omux

Step-by-step procedure for cutting and publishing a new release. End-user
instructions for **verifying** a downloaded release live in
[`docs/VERIFYING.md`](./docs/VERIFYING.md).

The release flow is **semi-manual**: the developer runs `cargo release` locally,
which pushes a `vX.Y.Z` tag. Everything from "tag exists on GitHub" onwards is
automated by [`.github/workflows/release.yml`](./.github/workflows/release.yml).

---

## Prerequisites (one-time setup)

```sh
cargo install cargo-release            # local version-bump + tag + push tooling
cargo install --locked slsa-verifier   # optional, for verifying after publish
sudo pacman -S github-cli              # or your distro equivalent; for gh release download
```

You also need:

- Push access to `bjcorder/omux` (the `chore: release X.Y.Z` commit will be
  authored as you and `cargo-release` pushes the tag to `origin`).
- A clean `git config user.name` / `user.email`.

---

## Step 1 — Pre-flight checks

Before you touch anything:

- [ ] `main` is **green** on the [`ci` workflow](https://github.com/bjcorder/omux/actions/workflows/ci.yml).
      A failing main is a hard stop — fix it first.
- [ ] You're on `main` and up to date: `git switch main && git pull --ff-only`.
- [ ] Working tree is clean: `git status` shows nothing.
- [ ] No PRs are mid-merge. A release that races a merge picks up half the
      changes.

If anything's off, fix it before continuing. There's no recovering from a bad
tag except cutting a new one — see [Rollback](#rolling-back-a-botched-tag).

---

## Step 2 — Stage the CHANGELOG

The `release` workflow extracts the matching `## [X.Y.Z]` section from
[`CHANGELOG.md`](./CHANGELOG.md) verbatim into the GitHub release body. So
whatever you put under `## [Unreleased]` *now* becomes the release notes when
`cargo release` rewrites that heading.

1. Open `CHANGELOG.md`.
2. Confirm `## [Unreleased]` is the top section and has accurate, user-facing
   content describing what's about to ship. Write for end users, not for git
   archaeologists.
3. If anything's missing, add it. Suggested sub-headings
   (Keep-a-Changelog style):
   - `### Highlights` — one-liners per major capability
   - `### Added` / `### Changed` / `### Fixed` / `### Removed` / `### Security`
4. Commit the changelog edit on a branch, open a PR against `main`, get it
   green on CI, and merge. **Don't release from a branch.**

After the merge, `git switch main && git pull --ff-only`. You're now on the
exact commit that will become the release.

---

## Step 3 — Dry-run

```sh
cargo release minor --dry-run        # or patch / major
```

Read the output carefully. It shows you:

- The version bump it'll apply (e.g. `0.0.1 → 0.1.0`).
- The exact CHANGELOG diff (`## [Unreleased]` → `## [Unreleased]` +
  `## [X.Y.Z] — YYYY-MM-DD`).
- The commit message (`chore: release X.Y.Z`).
- The tag name (`vX.Y.Z`).

If anything's wrong, **stop**. Fix the workspace state (CHANGELOG, version
bump policy, etc.) and re-run the dry-run. The cost of a botched real run is
much higher than the cost of looking at a dry-run twice.

---

## Step 4 — Cut the release

```sh
cargo release minor --execute        # same bump as the dry-run
```

`cargo-release` will execute these steps in order, halting on any failure:

| # | Step | Notes |
|---|---|---|
| 1 | Pre-release hook: `cargo test --workspace --locked` | If a test fails, **nothing else happens**. Fix the test and re-run. **Do not** pass `--skip-pre-release-hook`. |
| 2 | Bump `[workspace.package].version` in root `Cargo.toml` | Both crates inherit via `version.workspace = true`; no other edit needed. |
| 3 | Rewrite `CHANGELOG.md` | `## [Unreleased]` → `## [Unreleased]` + new `## [X.Y.Z] — YYYY-MM-DD`. |
| 4 | Commit | Message: `chore: release X.Y.Z`. |
| 5 | Tag | Annotated, named `vX.Y.Z`, message `Release vX.Y.Z`. |
| 6 | Push to `origin` | Commit + tag, in one `git push`. |

**You stop touching things now.** The tag push fires `release.yml` on GitHub.

---

## Step 5 — Watch CI

Open [the release workflow page](https://github.com/bjcorder/omux/actions/workflows/release.yml).
The run for your tag has three jobs:

| Job | Time (cold) | What it does |
|---|---|---|
| `build` | ~3–6 min | Inside the Arch container: `cargo build --workspace --release --locked`, assembles `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` (binaries + `.desktop` + icon + `install.sh` + `LICENSE` + `README.md`) and `omux-X.Y.Z-source.tar.gz` (via `git archive`), computes SHA256s, emits the base64 SLSA subject list. |
| `provenance` | ~2 min | Calls the official `slsa-framework/slsa-github-generator` reusable workflow. Mints a Sigstore OIDC token, signs an in-toto attestation pinning the artifact SHA256s + builder identity + source commit + source tag, uploads to [Rekor](https://search.sigstore.dev/), attaches `omux-X.Y.Z.intoto.jsonl` to the release. |
| `release` | ~30 s | Pulls the `## [X.Y.Z]` section from `CHANGELOG.md` into release notes, calls `softprops/action-gh-release` to create the release with both tarballs + `.sha256` sidecars. |

Total wall time: ~6 min. The release page is published **before** the provenance
attestation lands — that's expected; the `provenance` job appends the
`.intoto.jsonl` seconds after `release` finishes.

---

## Step 6 — Verify what you shipped

Don't skip this. CI green doesn't mean the artifacts work; this is the only
real end-to-end check. Run on a different machine if you can (or at least a
fresh shell):

```sh
TAG=v0.1.0   # whatever you just released

gh release download "$TAG" -R bjcorder/omux \
    -p '*.tar.gz' -p '*.intoto.jsonl'

slsa-verifier verify-artifact \
    --provenance-path "omux-${TAG#v}.intoto.jsonl" \
    --source-uri      github.com/bjcorder/omux \
    --source-tag      "$TAG" \
    "omux-${TAG#v}-x86_64-unknown-linux-gnu.tar.gz"
```

The final line should be:

```
PASSED: SLSA verification passed
```

Then smoke-test the install:

```sh
tar xzf "omux-${TAG#v}-x86_64-unknown-linux-gnu.tar.gz"
cd "omux-${TAG#v}-x86_64-unknown-linux-gnu"
./install.sh
omux --version    # should print X.Y.Z
```

If both pass, the release is real.

---

## What a successful release looks like

The [release page for `vX.Y.Z`](https://github.com/bjcorder/omux/releases) has,
when everything works:

- `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` (~5–6 MB)
- `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `omux-X.Y.Z-source.tar.gz`
- `omux-X.Y.Z-source.tar.gz.sha256`
- `omux-X.Y.Z.intoto.jsonl`

Plus a release body whose content matches the `## [X.Y.Z]` section of
`CHANGELOG.md`.

---

## Rolling back a botched tag

If you tagged the wrong commit or notice the release is broken:

```sh
git tag -d vX.Y.Z                                                # delete locally
git push --delete origin vX.Y.Z                                  # delete on GitHub
gh release delete vX.Y.Z -R bjcorder/omux --cleanup-tag --yes    # delete release
```

Then revert the release commit (`git revert HEAD`) and cut a fresh tag at the
next patch version. **Never re-use a version number** anyone might have
downloaded — the bad build's bytes might be cached in proxies / mirrors /
someone's `~/.cache`.

**Note on Rekor:** the SLSA provenance attestation lives in
[Rekor](https://search.sigstore.dev/) and is **immutable**. You can't delete
the entry for the botched build. That's by design — the transparency log
exists precisely so that a bad build leaves a permanent record. Just cut a
new tag; the new attestation supersedes the old one for anyone fetching the
latest release.

---

## Troubleshooting

**`provenance` job fails with "missing id-token permission":** the job-level
`permissions:` in `release.yml` got edited. The `provenance` job needs
`id-token: write`.

**`build` job fails with `Package 'gtk4' has version 'X.Y.Z', required version
is '>= 4.18'`:** the `container: archlinux:latest` block on the `build` job
got stripped, so it ran on the Ubuntu host runner. Ubuntu 24.04 ships GTK
4.14, below the gtk4-rs 0.10 floor. Restore the `container:` line.

**`Pre-release hook failed`:** `cargo test` is failing locally. Don't pass
`--skip-pre-release-hook` — fix the test first; CI would have caught it anyway.

**`fail_on_unmatched_files`:** the `release` job couldn't find the tarballs in
`dist/`. Look at the `build` job's "Assemble binary tarball" step — usually a
path the workflow expects (like `omux/resources/omux.desktop`) moved.

**Verifier fails on a freshly-released artifact:** check that the `--source-tag`
you passed exactly matches the released tag (`vX.Y.Z`, with the `v` prefix)
and that `--source-uri` is `github.com/bjcorder/omux` (no `https://`, no
trailing `.git`).

**`cargo release` refuses to run:** common reasons:
- `allow-branch = ["main"]` in `release.toml` and you're not on `main`.
- Working tree has uncommitted changes (clean it up; don't pass `--allow-dirty`).
- Your local `main` is behind `origin/main` (pull first).

---

## First-release special case (v0.1.0)

The very first release is slightly different because `0.0.1` was a working
version marker that never got tagged or published:

1. **Workspace version is already `0.0.1`** in `Cargo.toml`. `cargo release
   minor --execute` will bump to `0.1.0`. (Patch would go to `0.0.2`; minor is
   correct for "first public release".)
2. **`CHANGELOG.md` has a single `## [Unreleased]` heading** with the full
   release notes already staged. Confirm it reads as the v0.1.0 release notes
   you want users to see.
3. Run the standard procedure from Step 1 above.
4. After it ships, do the Step 6 verification end-to-end **on a different
   machine** if possible. If `slsa-verifier` prints `PASSED`, the pipeline is
   genuinely working — not just CI-green-by-luck.

---

## Quick reference

```sh
# Pre-flight
git switch main && git pull --ff-only
gh run list -R bjcorder/omux --workflow ci.yml --branch main --limit 1  # green?

# Cut
cargo release minor --dry-run
cargo release minor --execute

# Watch (open in browser)
gh run watch -R bjcorder/omux

# Verify
TAG=v0.1.0
gh release download "$TAG" -R bjcorder/omux -p '*.tar.gz' -p '*.intoto.jsonl'
slsa-verifier verify-artifact \
    --provenance-path "omux-${TAG#v}.intoto.jsonl" \
    --source-uri github.com/bjcorder/omux \
    --source-tag "$TAG" \
    "omux-${TAG#v}-x86_64-unknown-linux-gnu.tar.gz"
```
