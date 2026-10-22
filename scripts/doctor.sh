#!/usr/bin/env bash
# doctor.sh – Verify development environment
# Usage: ./scripts/doctor.sh
set -euo pipefail

echo "🩺 Checking development environment..."
echo

check() {
    if command -v "$1" &>/dev/null; then
        printf "✅ %-20s %s\n" "$1" "$(command -v "$1")"
        return 0
    else
        printf "❌ %-20s not found\n" "$1"
        return 1
    fi
}

MISSING=0

echo "=== Required Tools ==="
check cargo || MISSING=$((MISSING + 1))
check rustc || MISSING=$((MISSING + 1))
check pkg-config || MISSING=$((MISSING + 1))
# build.rs hard-requires glib-compile-resources to compile the GResource bundle
check glib-compile-resources || {
    echo "   Install: glib2-devel (Fedora) / libglib2.0-dev-bin (Ubuntu/Debian)"
    MISSING=$((MISSING + 1))
}

echo
echo "=== Packaging Tools ==="
check cargo-deb || echo "   Install: cargo install cargo-deb"
check rpmbuild || echo "   Install: sudo dnf install rpm-build"
check msgfmt || echo "   Install: sudo dnf install gettext"

echo
echo "=== GTK4 Libraries ==="
if pkg-config --exists gtk4 2>/dev/null; then
    printf "✅ %-20s %s\n" "gtk4" "$(pkg-config --modversion gtk4)"
else
    printf "❌ %-20s not found\n" "gtk4"
    MISSING=$((MISSING + 1))
fi

if pkg-config --exists libadwaita-1 2>/dev/null; then
    printf "✅ %-20s %s\n" "libadwaita-1" "$(pkg-config --modversion libadwaita-1)"
else
    printf "❌ %-20s not found\n" "libadwaita-1"
    MISSING=$((MISSING + 1))
fi

echo
if [[ $MISSING -eq 0 ]]; then
    echo "✅ All required dependencies found!"
else
    echo "❌ Missing $MISSING required dependencies"
    exit 1
fi
