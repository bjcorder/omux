# Security policy

## Supported versions

`omux` is pre-1.0. Only the **latest tagged release** receives security
fixes. There is no LTS.

| Version | Supported |
|---|---|
| Latest `v0.x.y` | ✅ |
| Older `v0.x.y`  | ❌ — upgrade to latest |

## Reporting a vulnerability

**Please don't open public issues for security bugs.** Use either:

- GitHub Private Vulnerability Reporting:
  https://github.com/bjcorder/omux/security/advisories/new
- Email: `bjcorder@protonmail.com` with subject prefix `[omux-sec]`.

I aim to acknowledge within 72 hours and patch within 14 days for
issues I can reproduce. If a fix needs longer (a third-party
dependency CVE we can't work around, say), I'll keep you in the loop.

## Threat model

`omux` is a desktop GUI application that runs your shell with your
user privileges and embeds WebKitGTK. It is **not** a sandbox, a
security boundary, or a hardened workstation tool.

### What omux defends against

- **Build supply-chain tampering.** Every release artifact carries a
  [SLSA Build Level 3](https://slsa.dev/spec/v1.0/) provenance
  attestation, signed via Sigstore and logged in Rekor. See
  [`docs/VERIFYING.md`](./docs/VERIFYING.md) for verifier usage.
- **Settings.json clobbering during hook install.** The Claude Code
  hook installer (`agent::hook_installer`) takes a backup before
  writing, only adds entries tagged with the sentinel
  `_omux_managed: true`, and reverses surgically on
  `omux --uninstall-hooks`. Other edits Claude Code (or you) make to
  `~/.claude/settings.json` survive an uninstall.

### What omux does NOT defend against

- **A malicious agent inside a pane.** Anything you `claude` or `codex`
  in a pane runs as you with full filesystem access, same as it
  would in any terminal. omux makes agents *easier to notice*, not
  *safer to run*.
- **A malicious tab loaded in the browser pane.** WebKitGTK is the
  sandbox boundary; omux inherits its CVE surface. Keep WebKitGTK
  patched (your distro should handle this).
- **A malicious workspace TOML file.** The TOML parser is `serde`
  + `toml`, both memory-safe. The path-traversal mitigation in
  `workspace::config` validates the slug, but if you `cp` a hostile
  `<slug>.toml` into your own workspaces dir, omux will happily load
  it.
- **Per-workspace cookie isolation as a privacy boundary.** Per-
  workspace `WebKitNetworkSession` keeps cookies separate but doesn't
  prevent fingerprinting; assume any tab can be correlated with any
  other tab of the same browser engine.
- **Timing side-channels, DoS, fault injection,** or other
  adversarial-class attacks. Out of scope.

## Verifying a downloaded binary

Every release has a `.intoto.jsonl` provenance file. The one-liner:

```sh
slsa-verifier verify-artifact \
    --provenance-path omux-X.Y.Z.intoto.jsonl \
    --source-uri      github.com/bjcorder/omux \
    --source-tag      vX.Y.Z \
    omux-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz
```

See [`docs/VERIFYING.md`](./docs/VERIFYING.md) for the full
walkthrough and what to do if verification fails.
