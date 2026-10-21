#! /bin/bash
# Starts RustyCorn from this folder (Linux/macOS), passing on all arguments:
#
#   ./RustyCorn.sh                 main window (live or demo, as set in Settings)
#   ./RustyCorn.sh /dev/ttyUSB0    use that serial port (and remember it)
#   ./RustyCorn.sh run.csv         open a saved log, read-only
#   ./RustyCorn.sh --methods       only the method editor
#
# Builds the release version first; cargo only recompiles when the code
# changed, so code changes are never silently ignored. Without cargo, an
# existing build is started.
# It does not change folder: a relative .csv path is taken from where you
# are, and with no data folder set that is where logs go.

DIR="$(cd "$(dirname "$0")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
BIN="${CARGO_TARGET_DIR:-$DIR/target}/release/rustycorn"

if command -v cargo >/dev/null; then
    cargo build --release -p rustycorn --manifest-path "$DIR/Cargo.toml" || {
        echo "RustyCorn.sh: the build failed (see above)." >&2
        exit 1
    }
elif [ ! -x "$BIN" ]; then
    echo "RustyCorn.sh: cargo not found and no build at $BIN." >&2
    echo "             Install Rust from https://rustup.rs and try again." >&2
    exit 1
fi
exec "$BIN" "$@"
