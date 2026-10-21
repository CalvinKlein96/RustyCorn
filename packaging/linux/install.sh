#!/usr/bin/env bash
# Installs RustyCorn for the current user (no root needed):
#   ~/.local/bin/rustycorn                                    the program
#   ~/.local/share/icons/hicolor/256x256/apps/rustycorn.png   its icon
#   ~/.local/share/applications/rustycorn.desktop             the menu entry
#
#   packaging/linux/install.sh              build the release version (if cargo
#                                           is available) and install it
#   packaging/linux/install.sh --uninstall  remove those three files again
#
# Settings (~/.config/RustyCorn) and data files are never touched.
# RUSTYCORN_BIN=<file> installs that binary instead of building one;
# CARGO_TARGET_DIR is respected.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/packaging/linux"
BIN_DIR="$HOME/.local/bin"
ICON_ROOT="$HOME/.local/share/icons/hicolor"
ICON="$ICON_ROOT/256x256/apps/rustycorn.png"
APPS="$HOME/.local/share/applications"
DESKTOP="$APPS/rustycorn.desktop"

refresh_menus() {
    # the menu and icon theme pick up the change (both optional)
    touch "$ICON_ROOT" 2>/dev/null || true
    if command -v update-desktop-database >/dev/null && [ -d "$APPS" ]; then
        update-desktop-database -q "$APPS" 2>/dev/null || true
    fi
}

case "${1:-}" in
    --uninstall)
        rm -fv "$BIN_DIR/rustycorn" "$ICON" "$DESKTOP"
        refresh_menus
        echo "RustyCorn removed (settings in ~/.config/RustyCorn and data files are kept)."
        exit 0
        ;;
    "") ;;
    -h|--help)
        sed -n '2,13s/^# \{0,1\}//p' "$0"
        exit 0
        ;;
    *)
        echo "usage: $0 [--uninstall]" >&2
        exit 2
        ;;
esac

if [ -n "${RUSTYCORN_BIN:-}" ]; then
    SRC="$RUSTYCORN_BIN"
else
    export PATH="$HOME/.cargo/bin:$PATH"
    SRC="${CARGO_TARGET_DIR:-$ROOT/target}/release/rustycorn"
    if command -v cargo >/dev/null; then
        cargo build --release -p rustycorn --manifest-path "$ROOT/Cargo.toml"
    elif [ ! -x "$SRC" ]; then
        echo "error: no release build at $SRC and cargo not found (see README.md)" >&2
        exit 1
    fi
fi
[ -x "$SRC" ] || { echo "error: $SRC is not an executable" >&2; exit 1; }

mkdir -p "$BIN_DIR" "$(dirname "$ICON")" "$APPS"
# copy, then rename: replacing a running program in place would fail ("text file busy")
cp "$SRC" "$BIN_DIR/.rustycorn.new"
chmod 755 "$BIN_DIR/.rustycorn.new"
mv -f "$BIN_DIR/.rustycorn.new" "$BIN_DIR/rustycorn"
install -m 644 "$ROOT/assets/icon-256.png" "$ICON"
# the menu entry runs the installed program by its full path: ~/.local/bin is
# not on the PATH of every desktop session
sed "s|^Exec=rustycorn|Exec=\"$BIN_DIR/rustycorn\"|" "$HERE/rustycorn.desktop" > "$DESKTOP"
chmod 644 "$DESKTOP"
refresh_menus

echo "Installed:"
echo "    $BIN_DIR/rustycorn"
echo "    $ICON"
echo "    $DESKTOP"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH; add it to start 'rustycorn' from a terminal." ;;
esac
echo "Remove again with: $0 --uninstall"
