#!/usr/bin/env bash
# Requires GNU gettext tools with Rust support. Normal Cargo builds do not.
set -euo pipefail
cd "$(dirname "$0")/../.."
mode=${1:-update}
if [[ "$mode" != update && "$mode" != --check ]]; then
    echo "Usage: $0 [--check]" >&2
    exit 2
fi
translation_template=$(mktemp)
trap 'rm -f "$translation_template"' EXIT
xgettext --language=Rust --from-code=UTF-8 \
    --keyword= --keyword=gettext:2 --keyword=ngettext:2,3 --keyword=pgettext:2c,3 \
    --add-comments=Translators: \
    --flag=ngettext:2:rust-format --flag=ngettext:3:rust-format \
    --package-name=Spotifast --copyright-holder='Spotifast contributors' \
    --msgid-bugs-address='https://github.com/crmne/spotifast/issues/new?template=translation.yml' \
    --files-from=assets/i18n/POTFILES --output="$translation_template"
if [[ "$mode" == --check ]]; then
    # The extraction timestamp is the only nondeterministic header.
    diff -u <(sed '/^"POT-Creation-Date:/d' assets/i18n/spotifast.pot) \
        <(sed '/^"POT-Creation-Date:/d' "$translation_template")
else
    cp "$translation_template" assets/i18n/spotifast.pot
    for catalog in assets/i18n/*.po; do
        # No fuzzy guesses: catalogues may be partial, and an untranslated
        # message must stay empty so it shows the English source.
        msgmerge --update --backup=none --no-fuzzy-matching "$catalog" assets/i18n/spotifast.pot
    done
fi
for catalog in assets/i18n/*.po; do
    msgfmt --check --check-format --output-file=/dev/null "$catalog"
done
