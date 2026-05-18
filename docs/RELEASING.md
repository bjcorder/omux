# Releasing omux

This is the maintainer-facing runbook for cutting a new release of
`omux`. End-user instructions for **verifying** a downloaded release
live in [`VERIFYING.md`](./VERIFYING.md).

The release flow is **semi-manual**: you run `cargo release` locally,
which pushes a `vX.Y.Z` tag. Everything from "tag exists on GitHub"
onwards is automated by [`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Prerequisites (one-time)

```sh
cargo install cargo-release      # version-bump + tag + push
cargo install --locked slsa-verifier   # optional, for verifying your own release after
sudo apt install gh              # or whatever your distro calls it; used for post-release smoke
```

You also need push access to `bjcorder/omux` and your Git config
needs a sensible name/email (the release commit is authored as you).

## Pre-flight checklist

Before you run `cargo release`:

- [ ] `main` is green on the `ci` workflow.
- [ ] `CHANGELOG.md` has an `## [Unreleased]` heading with non-empty
      content describing what's about to ship. The `release.yml`
      workflow extracts that section verbatim into the GitHub release
      body, so write it for end users, not for git archaeologists.
- [ ] Working tree is clean (`git status` shows nothing to commit).
- [ ] You're on `main` and up to date with `origin/main`.
- [ ] No PRs are mid-merge — a release that races a merge will pick
      up half the changes.

## Cut a release

One command:

```sh
cargo release minor --execute    # or patch / major
```

Without `--execute`, cargo-release runs in dry-run mode and shows
what it *would* do. Run that first if you want to see the diff:

```sh
cargo release minor --dry-run
```

When you run with `--execute`, cargo-release will:

1. Run the **pre-release hook** (`cargo test --workspace --locked`).
   If tests fail, nothing is bumped. **Do not skip this.** Fix the
   test instead.
2. Bump `[workspace.package].version` in the root `Cargo.toml`. Both
   member crates inherit via `version.workspace = true`, so nothing
   else needs touching.
3. Rewrite `CHANGELOG.md`: the `## [Unreleased]` heading becomes
   `## [Unreleased]` + a new `## [X.Y.Z] — YYYY-MM-DD` heading below
   it, so the next development cycle starts with an empty Unreleased
   section.
4. Commit with message `chore: release X.Y.Z`.
5. Tag `vX.Y.Z` annotated with `Release vX.Y.Z`.
6. Push both the commit and the tag to `origin`.

You now stop touching things. The tag push fires `release.yml`.

## Watch CI

Open https://github.com/bjcorder/omux/actions/workflows/release.yml.
The run for your tag has three jobs:

| Job | Time | What it does |
|---|---|---|
| `build` | ~3 min cold / ~90 s warm | Builds release binaries, assembles the `*.tar.gz` files, computes SHA256s. |
| `provenance` | ~2 min | Calls the official `slsa-github-generator` reusable workflow. Mints a Sigstore OIDC token, signs an in-toto attestation pinning the artifact SHA256s + builder identity + source commit, uploads to Rekor (public transparency log), attaches `omux-X.Y.Z.intoto.jsonl` to the release. |
| `release` | ~30 s | Downloads the build artifacts, extracts the `## [X.Y.Z]` section from `CHANGELOG.md` into release notes, calls `softprops/action-gh-release` to create the release and upload tarballs + `.sha256` sidecars. |

Total wall time is ~6 min cold. The release page is published
**before** the provenance attestation is uploaded — that's expected;
the `provenance` job appends the `.intoto.jsonl` to the existing
release seconds after.

## What a successful release looks like

https://github.com/bjcorder/omux/releases/tag/vX.Y.Z has, when
everything works:

- `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` (~5–6 MB)
- `omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `omux-X.Y.Z-source.tar.gz`
- `omux-X.Y.Z-source.tar.gz.sha256`
- `omux-X.Y.Z.intoto.jsonl`

Verify it yourself end-to-end on a clean machine using the steps in
[`VERIFYING.md`](./VERIFYING.md). Doing this at least once per
release catches workflow drift that pure CI green can hide.

## Rolling back a botched tag

If you tagged the wrong commit or notice the release is broken:

```sh
git tag -d vX.Y.Z                        # delete locally
git push --delete origin vX.Y.Z          # delete on GitHub
gh release delete vX.Y.Z -R bjcorder/omux --cleanup-tag --yes
```

Then revert the release commit (`git revert HEAD`) or, if it hasn't
been pulled by anyone else and you're sure, hard-reset and force-push
`main`. Cut a fresh tag at the next patch version — never re-use a
version number that anyone might have downloaded.

**Note on Rekor:** the SLSA provenance attestation lives in
[Rekor](https://search.sigstore.dev/) and is **immutable**. You can't
delete the entry for the botched build. That's by design — the
transparency log exists precisely so that a bad build leaves a
permanent record. Just cut a new tag; the new attestation supersedes
the old one for anyone fetching the latest release.

## Troubleshooting

**`provenance` job fails with "missing id-token permission":** the
job-level `permissions:` in `release.yml` got edited; check that the
`provenance` job has `id-token: write`.

**`build` job fails to find GTK headers:** the runner pin slipped
back to `ubuntu-latest` (currently 22.04, which lacks WebKitGTK 6 in
the main archive). Re-pin to `ubuntu-24.04`.

**`Pre-release hook failed`:** `cargo test` is failing locally on
your machine. Don't pass `--skip-pre-release-hook`. Fix the test
first; it'll fail in CI anyway.

**`fail_on_unmatched_files`:** the `release` job couldn't find the
tarballs in `dist/`. Look at the `build` job's "Assemble binary
tarball" step — usually a path the workflow expects (like
`omux/resources/omux.desktop`) moved.

**Verifier fails on a freshly-released artifact:** check that the
`source-tag` you passed exactly matches the released tag (`vX.Y.Z`,
with the `v` prefix) and that `--source-uri` is
`github.com/bjcorder/omux` (no `https://`, no trailing `.git`).

## First-release special case (v0.1.0)

The very first release is slightly different from steady-state
because v0.0.1 was never actually published:

1. Edit `CHANGELOG.md` once: the existing top section is
   `## [Unreleased]` (formerly `## [0.0.1] — Unreleased`). Leave it
   as `## [Unreleased]`; don't manually add a `## [0.0.1]` section.
2. Run `cargo release minor --execute`. The bump goes
   `0.0.1 → 0.1.0`. The first GitHub release is v0.1.0.
3. After it ships, do a `VERIFYING.md`-driven smoke test from a
   clean shell. If `slsa-verifier` prints `PASSED`, the pipeline is
   actually working end-to-end and not just CI-green-by-luck.
