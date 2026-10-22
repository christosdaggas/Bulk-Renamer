#!/usr/bin/env bash
# package-rpm.sh – Build the Fedora RPM package into dist/rpm/
# Usage: ./scripts/package-rpm.sh   (RELEASE=2 ./scripts/package-rpm.sh to bump the release)
#
# Always rebuilds the release binary, then packages it together with the
# desktop file, AppStream metainfo, icons, licence, README and translations.
# Needs: cargo, rpmbuild (rpm-build), msgfmt (gettext).
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in cargo rpmbuild msgfmt; do
    command -v "$tool" >/dev/null || { echo "❌ $tool not found" >&2; exit 1; }
done

APP_ID="com.chrisdaggas.bulk-renamer"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
RELEASE="${RELEASE:-1}"
# Separate target dir: the path-remapping RUSTFLAGS below would otherwise
# invalidate the normal target/release build cache on every switch.
TARGET_DIR="target/package"

echo "📦 Building RPM package for Bulk Renamer ${VERSION}-${RELEASE}..."

# Validate the desktop integration files when the validators are installed
if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "data/${APP_ID}.desktop"
fi
if command -v appstreamcli >/dev/null; then
    appstreamcli validate --no-net "data/${APP_ID}.metainfo.xml"
fi

# Keep local paths (home directory, cargo registry, checkout) out of the
# binary: they otherwise end up in panic location strings.
REMAP="--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$PWD=/build/bulk-renamer"
if [[ -n "${HOME:-}" && "$HOME" != "/" ]]; then
    # rustc applies the last matching prefix, so the generic home rule goes first
    REMAP="--remap-path-prefix=$HOME=/home/builder $REMAP"
fi
RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$REMAP" cargo build --release --locked --target-dir "$TARGET_DIR"

RPM_TOPDIR="$(mktemp -d "${TMPDIR:-/tmp}/bulk-renamer-rpmbuild.XXXXXX")"
trap 'rm -rf "$RPM_TOPDIR"' EXIT
SOURCES="$RPM_TOPDIR/SOURCES"
SPEC_FILE="$RPM_TOPDIR/SPECS/bulk-renamer.spec"
mkdir -p "$RPM_TOPDIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Stage sources (pre-built binary, no source archive)
install -m755 "$TARGET_DIR/release/bulk-renamer" "$SOURCES/bulk-renamer"
cp "data/${APP_ID}.desktop" \
   "data/${APP_ID}.metainfo.xml" \
   "data/icons/hicolor/scalable/apps/${APP_ID}.svg" \
   "data/icons/symbolic/apps/${APP_ID}-symbolic.svg" \
   LICENSE README.md "$SOURCES/"

# Compile translations into SOURCES/locale; %find_lang below picks them up
bash scripts/compile-translations.sh "$SOURCES/locale"

cat > "$SPEC_FILE" <<EOF
# The binary is already stripped by the release profile
%global debug_package %{nil}

Name:           bulk-renamer
Version:        $VERSION
Release:        $RELEASE%{?dist}
Summary:        Rename many files at once with a live preview and undo
License:        MIT
URL:            https://chrisdaggas.com

# Built from a binary compiled by scripts/package-rpm.sh; no source archive
Source0:        bulk-renamer
Source1:        ${APP_ID}.desktop
Source2:        ${APP_ID}.metainfo.xml
Source3:        ${APP_ID}.svg
Source4:        ${APP_ID}-symbolic.svg
Source5:        LICENSE
Source6:        README.md

Requires:       gtk4%{?_isa} >= 4.12
Requires:       libadwaita%{?_isa} >= 1.5
Requires:       hicolor-icon-theme

%description
Bulk Renamer is a GNOME app for renaming many files at once. Combine rules
such as replace, change case, numbering, dates and photo or music details,
preview every new name before anything changes, and undo whole batches later.
It also offers presets, CSV import and a command-line mode.

%prep
cp -p %{SOURCE5} %{SOURCE6} .

%install
install -Dpm 0755 %{SOURCE0} %{buildroot}%{_bindir}/bulk-renamer
install -Dpm 0644 %{SOURCE1} %{buildroot}%{_datadir}/applications/${APP_ID}.desktop
install -Dpm 0644 %{SOURCE2} %{buildroot}%{_metainfodir}/${APP_ID}.metainfo.xml
install -Dpm 0644 %{SOURCE3} %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/${APP_ID}.svg
install -Dpm 0644 %{SOURCE4} %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/${APP_ID}-symbolic.svg
mkdir -p %{buildroot}%{_datadir}
cp -pr %{_sourcedir}/locale %{buildroot}%{_datadir}/
%find_lang %{name}

%files -f %{name}.lang
%license LICENSE
%doc README.md
%{_bindir}/bulk-renamer
%{_datadir}/applications/${APP_ID}.desktop
%{_metainfodir}/${APP_ID}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/${APP_ID}.svg
%{_datadir}/icons/hicolor/symbolic/apps/${APP_ID}-symbolic.svg

%changelog
* $(LC_ALL=C date +'%a %b %d %Y') Christos A. Daggas <info@chrisdaggas.com> - ${VERSION}-${RELEASE}
- Bulk Renamer ${VERSION}, see CHANGELOG.md for the full list of changes
EOF

# _buildhost: keep this machine's hostname out of the package header
rpmbuild \
    --define "_topdir $RPM_TOPDIR" \
    --define "_buildhost localhost" \
    -bb "$SPEC_FILE"

# Create dist directory and move package
mkdir -p dist/rpm
rm -f dist/rpm/*.rpm
mv "$RPM_TOPDIR"/RPMS/*/*.rpm dist/rpm/

echo "✅ RPM package created:"
ls -lh dist/rpm/*.rpm
