#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Increment build number (e.g. 0.1.2.10043 -> 0.1.2.10044)
increment_version() {
    local file="$1"
    if [ ! -f "$file" ]; then
        echo "0.1.0.100" > "$file"
    fi
    local ver=$(cat "$file" | tr -d '\r\n')
    local prefix=$(echo "$ver" | sed 's/\.[0-9]*$//')
    local build=$(echo "$ver" | sed 's/.*\.//')
    local new_build=$((build + 1))
    local new_ver="${prefix}.${new_build}"
    echo "$new_ver" > "$file"
    echo "$new_ver"
}

# Output directory (set DIST_DIR externally to copy binary there)
DIST_DIR="${DIST_DIR:-}"

cd "$SCRIPT_DIR"
VERSION=$(increment_version "VERSION.txt")
echo "[Rust] Building tail-f_plot..."
echo "  Version: $VERSION"
cargo build --release

if [ -n "$DIST_DIR" ]; then
    mkdir -p "$DIST_DIR"
    cp "$SCRIPT_DIR/target/release/tail-f_plot.exe" "$DIST_DIR/"
    echo "[Rust] Done -> $DIST_DIR/tail-f_plot.exe"
else
    echo "[Rust] Done -> target/release/tail-f_plot.exe"
fi
