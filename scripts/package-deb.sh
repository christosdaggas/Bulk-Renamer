#!/usr/bin/env bash
# package-deb.sh – Build Debian package
# Usage: ./scripts/package-deb.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "📦 Building DEB package..."

# Always rebuild so the package never contains a stale binary
cargo build --release

# Compile translations; the deb assets in Cargo.toml ship them from target/locale
bash scripts/compile-translations.sh target/locale

# Generate DEB
cargo deb

# Create dist directory and move package
mkdir -p dist/deb
rm -f dist/deb/*.deb
mv target/debian/*.deb dist/deb/

echo "✅ DEB package created:"
ls -lh dist/deb/*.deb
