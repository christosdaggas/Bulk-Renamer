#!/usr/bin/env bash
# Regenerate the POT template from the sources listed in po/POTFILES.in and
# merge it into every .po catalogue listed in po/LINGUAS.
set -euo pipefail

cd "$(dirname "$0")/.."

command -v xgettext >/dev/null || { echo "xgettext not found (install gettext)"; exit 1; }
command -v msgmerge >/dev/null || { echo "msgmerge not found (install gettext)"; exit 1; }

POT=po/bulk-renamer.pot

# xgettext has no Rust mode; the C parser understands the gettext("...")
# calls the sources use.
xgettext \
    --files-from=po/POTFILES.in \
    --from-code=UTF-8 \
    --language=C \
    --keyword=gettext \
    --add-comments=TRANSLATORS \
    --package-name=bulk-renamer \
    --msgid-bugs-address=https://github.com/christosdaggas/Bulk-Renamer/issues \
    --output="$POT"

while read -r lang; do
    [ -z "$lang" ] && continue
    po="po/${lang}.po"
    if [ -f "$po" ]; then
        echo "merging $po"
        msgmerge --update --backup=off "$po" "$POT"
    else
        echo "skipping missing $po"
    fi
done < po/LINGUAS

echo "POT and catalogues updated."
