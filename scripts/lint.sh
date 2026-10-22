#!/usr/bin/env bash
# lint.sh – Run code quality checks
# Usage: ./scripts/lint.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "🔍 Running lint checks..."
echo

echo "=== Clippy ==="
cargo clippy --all-targets -- -D warnings

echo
echo "=== Format Check ==="
cargo fmt -- --check

echo
echo "✅ All checks passed!"
