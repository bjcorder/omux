# Verifying an omux release

Every `omux` release ships with a **SLSA Build Level 3** provenance
attestation. This document shows you how to use it to verify that
the binary you downloaded was built from the source you can read on
GitHub.

## What this verifies (and what it doesn't)

**Verifies:**

- The tarball you downloaded has a specific SHA256.
- That SHA256 appears in a Sigstore-signed attestation pinned to:
  - source repo `github.com/bjcorder/omux`
  - source tag `vX.Y.Z`
  - source commit (recorded in the attestation; resolves through the
    tag)
  - builder workflow YAML at that commit
  - GitHub Actions runner identity (Microsoft-controlled)
- The attestation entry is logged in the public
  [Rekor transparency log](https://search.sigstore.dev/), so anyone
  can audit which versions were ever signed.

In plain English: **the binary in your hands was built from the source
you can see on GitHub, on Microsoft-controlled hardware, in a way
that's recorded forever in a public log.**

**Does NOT verify:**

- That the source code is free of bugs or vulnerabilities. SLSA is a
  *build integrity* statement, not a code audit.
- That the maintainer is trustworthy. SLSA proves the build pipeline
  wasn't tampered with — it doesn't prove the source code wasn't
  malicious to begin with.
- That GTK4 / WebKitGTK / VTE — which `omux` dynamically links —
  don't have CVEs. Check your distro's security advisories.

## Install `slsa-verifier`

`slsa-verifier` is written in Go; it's not on crates.io. Pick one:

```sh
# Option A: via Go (requires a Go toolchain)
go install github.com/slsa-framework/slsa-verifier/v2/cli/slsa-verifier@latest

# Option B: prebuilt binary
# Download the latest release for your OS / arch from
# https://github.com/slsa-framework/slsa-verifier/releases
# and place it on your $PATH.

# Option C: package manager
# Arch Linux (AUR):   paru -S slsa-verifier        # (or yay, etc.)
# macOS / Linuxbrew:  brew install slsa-verifier
```

Confirm:

```sh
slsa-verifier version
```

## Verify a release

```sh
# 1. Pick a release. Replace X.Y.Z with the version you want.
TAG=v0.1.0

# 2. Download the artifacts you want to verify, plus the provenance.
gh release download "$TAG" -R bjcorder/omux \
    -p '*.tar.gz' -p '*.intoto.jsonl'

# 3. Verify the binary tarball.
slsa-verifier verify-artifact \
    --provenance-path "omux-${TAG#v}.intoto.jsonl" \
    --source-uri      github.com/bjcorder/omux \
    --source-tag      "$TAG" \
    "omux-${TAG#v}-x86_64-unknown-linux-gnu.tar.gz"
```

A passing run prints:

```
Verified signature against tlog entry index <N> at URL: https://rekor.sigstore.dev/api/v1/log/entries/<digest>
Verified build using builder "https://github.com/slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@refs/tags/v2.0.0" at commit <sha>
PASSED: SLSA verification passed
```

If you see `PASSED` on the last line, the tarball is trustworthy in
the SLSA-L3 sense above.

Repeat for the source tarball if you want to verify it too:

```sh
slsa-verifier verify-artifact \
    --provenance-path "omux-${TAG#v}.intoto.jsonl" \
    --source-uri      github.com/bjcorder/omux \
    --source-tag      "$TAG" \
    "omux-${TAG#v}-source.tar.gz"
```

(The same `.intoto.jsonl` covers every artifact in the release, so
you don't need separate provenance files per tarball.)

## Reading a failed verification

If `slsa-verifier` exits non-zero, **do not run the binary.** Common
failure modes:

- **`expected source github.com/bjcorder/omux, got github.com/<other>`**:
  someone uploaded a binary with a provenance file from a different
  repo. Treat as compromised.
- **`expected tag v0.1.0, got v0.0.9`**: the provenance covers a
  different version than the artifact you downloaded. Probably a
  fetch error; re-download.
- **`tlog entry not found`**: Rekor lookup failed. Could be a network
  issue (Rekor is at `https://rekor.sigstore.dev`); could be tampering.
  Retry with `--rekor-offline` if you fetched the Rekor bundle
  yourself, or re-download both files.
- **`hash mismatch`**: the tarball's SHA256 doesn't match what's
  signed in the attestation. The artifact was modified after build.

In any failure case, file an issue at
https://github.com/bjcorder/omux/issues and paste the verifier output.

## Why the `.intoto.jsonl` extension

The provenance file is an [in-toto](https://in-toto.io) v1
attestation encoded as JSON-Lines (one DSSE envelope per line — for
omux releases, one line covering all artifacts). The full SLSA v1.0
spec is at https://slsa.dev/spec/v1.0/.

## Offline / air-gapped verification

SLSA verification needs to contact Rekor by default. If your build
environment can't reach `rekor.sigstore.dev`:

```sh
# Fetch the Rekor entry once, on a machine that has internet:
cosign verify-blob --bundle rekor-bundle.json ...
# Then verify offline using that bundle:
slsa-verifier verify-artifact \
    --provenance-path omux-X.Y.Z.intoto.jsonl \
    --source-uri github.com/bjcorder/omux \
    --source-tag vX.Y.Z \
    --rekor-bundle rekor-bundle.json \
    omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz
```

See the [slsa-verifier docs](https://github.com/slsa-framework/slsa-verifier#option-2-verification-with-offline-validation-of-the-rekor-entry)
for the full offline workflow.
