#!/usr/bin/env bash
# clean.sh – Clean build artifacts
# Usage: ./scripts/clean.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "🧹 Cleaning build artifacts..."

cargo clean

# Clean dist folder
if [[ -d dist ]]; then
    rm -rf dist/rpm/*.rpm dist/deb/*.deb dist/appimage/*.AppImage 2>/dev/null || true
    echo "Cleaned dist packages"
fi

echo "✅ Clean complete"
