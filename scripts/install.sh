#!/usr/bin/env bash
#
# Install omux + omux-hook into the user's XDG directories.
#
# Defaults to a user-local install (no sudo). Pass `--system` to install
# under /usr/local instead (requires sudo).
#
# Re-runnable: overwrites existing files in place. To uninstall use
# scripts/uninstall.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MODE="user"
SKIP_BUILD=0
for arg in "$@"; do
    case "$arg" in
        --system) MODE="system" ;;
        --skip-build) SKIP_BUILD=1 ;;
        -h|--help)
            cat <<EOF
Usage: $0 [--system] [--skip-build]

  --system      install to /usr/local + /usr/share (needs sudo).
                Default is user-local under ~/.local.
  --skip-build  skip 'cargo build --release'; use existing binaries.
EOF
            exit 0
            ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

if [[ "$MODE" == "system" ]]; then
    PREFIX="/usr/local"
    SHARE="/usr/share"
    SUDO="sudo"
else
    PREFIX="${XDG_DATA_HOME:-$HOME/.local}"
    # ~/.local/bin/ + ~/.local/share/
    BIN_DIR="$HOME/.local/bin"
    SHARE="$HOME/.local/share"
    SUDO=""
fi

if [[ "$MODE" == "system" ]]; then
    BIN_DIR="$PREFIX/bin"
fi

DESKTOP_DIR="$SHARE/applications"
ICON_DIR="$SHARE/icons/hicolor/scalable/apps"

echo "==> omux installer"
echo "    mode:       $MODE"
echo "    binaries:   $BIN_DIR"
echo "    desktop:    $DESKTOP_DIR"
echo "    icon:       $ICON_DIR"
echo

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "==> cargo build --release"
    (cd "$REPO_DIR" && cargo build --release --workspace)
fi

OMUX_BIN="$REPO_DIR/target/release/omux"
HOOK_BIN="$REPO_DIR/target/release/omux-hook"
if [[ ! -x "$OMUX_BIN" || ! -x "$HOOK_BIN" ]]; then
    echo "error: release binaries not found. Run 'cargo build --release' first." >&2
    exit 1
fi

echo "==> install binaries"
$SUDO install -Dm755 "$OMUX_BIN" "$BIN_DIR/omux"
$SUDO install -Dm755 "$HOOK_BIN" "$BIN_DIR/omux-hook"

echo "==> install icon"
$SUDO install -Dm644 "$REPO_DIR/omux/resources/omux.svg" "$ICON_DIR/omux.svg"

echo "==> install desktop file"
$SUDO install -Dm644 "$REPO_DIR/omux/resources/omux.desktop" \
    "$DESKTOP_DIR/omux.desktop"

echo "==> refresh desktop + icon caches"
if command -v update-desktop-database >/dev/null 2>&1; then
    $SUDO update-desktop-database "$DESKTOP_DIR" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    $SUDO gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" || true
fi

echo
echo "==> done"
echo "Launch from your application menu (search 'omux'), or run:"
echo "    $BIN_DIR/omux"
echo
if [[ "$MODE" == "user" && ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Note: $BIN_DIR is not on your PATH. Add it via:"
    echo "    export PATH=\"$BIN_DIR:\$PATH\""
fi
