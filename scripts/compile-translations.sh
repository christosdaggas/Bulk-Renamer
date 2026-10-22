#!/usr/bin/env bash
# Compile every .po catalogue to .mo under DEST (default: target/locale),
# laid out as <lang>/LC_MESSAGES/bulk-renamer.mo for bindtextdomain.
set -euo pipefail

cd "$(dirname "$0")/.."

DEST="${1:-target/locale}"

command -v msgfmt >/dev/null || { echo "msgfmt not found (install gettext)"; exit 1; }

while read -r lang; do
    [ -z "$lang" ] && continue
    po="po/${lang}.po"
    [ -f "$po" ] || { echo "skipping missing $po"; continue; }
    out_dir="$DEST/$lang/LC_MESSAGES"
    mkdir -p "$out_dir"
    msgfmt --check --output-file="$out_dir/bulk-renamer.mo" "$po"
    echo "compiled $out_dir/bulk-renamer.mo"
done < po/LINGUAS

echo "Run the app with BULK_RENAMER_LOCALEDIR=$DEST to test translations."
