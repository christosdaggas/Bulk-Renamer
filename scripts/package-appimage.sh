#!/usr/bin/env bash
# package-appimage.sh – Build AppImage package
# Usage: ./scripts/package-appimage.sh
#
# RUNTIME REQUIREMENTS: this AppImage bundles only the application binary,
# not the GTK stack. The host system must provide GTK4 >= 4.12 and
# libadwaita >= 1.5 at runtime (see the AppImage note in README.md).
#
# The appimagetool download can be pinned by exporting APPIMAGETOOL_SHA256
# with the checksum of a known-good build; the "continuous" release moves,
# so no checksum is hardcoded here.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Bulk Renamer"
APP_ID="com.chrisdaggas.bulk-renamer"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
ARCH="x86_64"

echo "📦 Building AppImage for ${APP_NAME} v${VERSION}..."

# Always rebuild so the package never contains a stale binary
cargo build --release

# Create AppDir structure
APPDIR="dist/appimage/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$APPDIR/usr/share/metainfo"

# Copy files
cp target/release/bulk-renamer "$APPDIR/usr/bin/"
cp "data/${APP_ID}.desktop" "$APPDIR/usr/share/applications/"
cp "data/${APP_ID}.desktop" "$APPDIR/${APP_ID}.desktop"
cp "data/icons/hicolor/scalable/apps/${APP_ID}.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
cp "data/icons/hicolor/scalable/apps/${APP_ID}.svg" "$APPDIR/"
ln -sf "${APP_ID}.svg" "$APPDIR/.DirIcon"
cp "data/${APP_ID}.metainfo.xml" "$APPDIR/usr/share/metainfo/"

# Create AppRun
cat > "$APPDIR/AppRun" << 'APPRUN'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
export XDG_DATA_DIRS="${HERE}/usr/share:${XDG_DATA_DIRS:-/usr/share}"
exec "${HERE}/usr/bin/bulk-renamer" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

# Download appimagetool if needed (HTTPS, --fail so an error page is never
# saved as the tool). Optionally verified against APPIMAGETOOL_SHA256.
APPIMAGETOOL="dist/appimage/appimagetool"
if [[ ! -x "$APPIMAGETOOL" ]]; then
    echo "Downloading appimagetool..."
    mkdir -p dist/appimage
    curl --fail -sSL "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" -o "$APPIMAGETOOL"
    if [[ -n "${APPIMAGETOOL_SHA256:-}" ]]; then
        echo "${APPIMAGETOOL_SHA256}  ${APPIMAGETOOL}" | sha256sum -c - || {
            echo "❌ appimagetool checksum mismatch" >&2
            rm -f "$APPIMAGETOOL"
            exit 1
        }
    fi
    chmod +x "$APPIMAGETOOL"
fi

# Build AppImage
OUTPUT="dist/appimage/${APP_NAME// /-}-${VERSION}-${ARCH}.AppImage"
rm -f "$OUTPUT"
ARCH="$ARCH" "$APPIMAGETOOL" "$APPDIR" "$OUTPUT"

# Cleanup
rm -rf "$APPDIR"

echo "✅ AppImage created:"
ls -lh "$OUTPUT"
