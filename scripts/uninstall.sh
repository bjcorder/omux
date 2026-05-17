#!/usr/bin/env bash
#
# Uninstall omux + omux-hook from the user's XDG directories.
# Pass `--system` to uninstall from /usr/local (requires sudo).
#
# Does NOT touch your config / state / data dirs. If you also want
# to wipe those:
#   rm -rf ~/.config/omux ~/.local/state/omux

set -euo pipefail

MODE="user"
for arg in "$@"; do
    case "$arg" in
        --system) MODE="system" ;;
        -h|--help)
            cat <<EOF
Usage: $0 [--system]

  --system  uninstall from /usr/local + /usr/share (needs sudo).
            Default is user-local under ~/.local.
EOF
            exit 0
            ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

if [[ "$MODE" == "system" ]]; then
    BIN_DIR="/usr/local/bin"
    SHARE="/usr/share"
    SUDO="sudo"
else
    BIN_DIR="$HOME/.local/bin"
    SHARE="$HOME/.local/share"
    SUDO=""
fi

DESKTOP_DIR="$SHARE/applications"
ICON_DIR="$SHARE/icons/hicolor/scalable/apps"

echo "==> omux uninstaller (mode: $MODE)"

removed=0
for p in \
    "$BIN_DIR/omux" \
    "$BIN_DIR/omux-hook" \
    "$DESKTOP_DIR/omux.desktop" \
    "$ICON_DIR/omux.svg"; do
    if [[ -e "$p" ]]; then
        $SUDO rm -f "$p"
        echo "  removed $p"
        removed=$((removed + 1))
    fi
done

if [[ "$removed" -gt 0 ]]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
        $SUDO update-desktop-database "$DESKTOP_DIR" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        $SUDO gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" || true
    fi
fi

echo "==> $removed file(s) removed"
echo "Config and state are preserved under \$XDG_CONFIG_HOME/omux and \$XDG_STATE_HOME/omux."
echo "Wipe them manually if you want a clean slate."
