#!/usr/bin/env bash
# release-build.sh – Optimized release build
# Usage: ./scripts/release-build.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "🔨 Building Bulk Renamer (release)..."
cargo build --release

echo "✅ Build complete: target/release/bulk-renamer"
ls -lh target/release/bulk-renamer
