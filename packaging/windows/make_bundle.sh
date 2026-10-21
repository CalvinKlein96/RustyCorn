#!/usr/bin/env bash
# Cross-builds RustyCorn for 64-bit Windows on Linux and packs
# dist/RustyCorn-<version>-windows.zip: RustyCorn.exe, README.txt and the
# licences. The executable needs only DLLs that come with Windows 10/11.
#
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64 (gcc and windres; the
# icon is embedded with windres), zip.
#
#   packaging/windows/make_bundle.sh [output folder]     (default: dist/)
#
# CARGO_TARGET_DIR is respected (default: target-win in the RustyCorn folder).

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/packaging/windows"
TARGET=x86_64-pc-windows-gnu
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target-win}"
OUT_DIR=$(mkdir -p "${1:-$ROOT/dist}" && cd "${1:-$ROOT/dist}" && pwd)

for tool in cargo x86_64-w64-mingw32-gcc x86_64-w64-mingw32-windres zip; do
    command -v "$tool" >/dev/null || { echo "error: $tool not found" >&2; exit 1; }
done

VERSION=$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" |
    python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "rustycorn"))')
NAME="RustyCorn-$VERSION"

echo "Building $NAME for Windows ($TARGET) …"
cargo build --release --target "$TARGET" -p rustycorn --manifest-path "$ROOT/Cargo.toml"
EXE="$CARGO_TARGET_DIR/$TARGET/release/rustycorn.exe"

# Only Windows system DLLs may be needed: a mingw runtime DLL (libgcc_s_seh,
# libwinpthread, libstdc++) would be missing on the lab PC.
if command -v x86_64-w64-mingw32-objdump >/dev/null; then
    DLLS=$(x86_64-w64-mingw32-objdump -p "$EXE" | awk '/DLL Name:/ {print tolower($3)}' | sort -u)
    if echo "$DLLS" | grep -E '^(lib|mingw)' >/dev/null; then
        echo "error: the executable needs mingw runtime DLLs:" >&2
        echo "$DLLS" | grep -E '^(lib|mingw)' >&2
        exit 1
    fi
fi

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
DIR="$STAGE/$NAME"
mkdir -p "$DIR/licenses"
cp "$EXE" "$DIR/RustyCorn.exe"
# Windows line endings, so the texts read well in any editor
crlf() { sed -e 's/\r$//' -e 's/$/\r/' "$1" > "$2"; }
sed "s/@VERSION@/$VERSION/" "$HERE/README.txt" > "$STAGE/README.txt"
crlf "$STAGE/README.txt" "$DIR/README.txt"
crlf "$ROOT/LICENSE" "$DIR/LICENSE.txt"
crlf "$ROOT/assets/fonts/Inter-LICENSE.txt" "$DIR/licenses/Inter-LICENSE.txt"
crlf "$ROOT/assets/fonts/JetBrainsMono-OFL.txt" "$DIR/licenses/JetBrainsMono-OFL.txt"

ZIP="$OUT_DIR/$NAME-windows.zip"
rm -f "$ZIP"
(cd "$STAGE" && zip -q -r -9 -X "$ZIP" "$NAME")

echo
echo "  $ZIP ($(du -h "$ZIP" | cut -f1))"
(cd "$STAGE" && find "$NAME" -type f -printf '      %p  %s bytes\n' | sort)
