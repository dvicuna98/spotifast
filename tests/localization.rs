use tr::Translator;

include!(concat!(env!("OUT_DIR"), "/test_catalog.rs"));

#[test]
fn compiled_po_omits_unfinished_messages_and_uses_locale_plural_rules() {
    let catalog = fixture::Translator;
    assert_eq!(fixture::PLURALS, 3);
    assert_eq!(catalog.translate("Home", None), "Start");
    for source in ["Search", "Library", "Missing", "Removed"] {
        assert_eq!(catalog.translate(source, None), source);
    }
    for count in [0, 1, 2, 5, 12, 22, 101, 112] {
        let singular = "Playlist • {count} song";
        let plural = "Playlist • {count} songs";
        assert_eq!(
            catalog.ntranslate(count, singular, plural, None),
            if count == 1 { singular } else { plural },
            "an incomplete plural must fall back to a whole English phrase"
        );
        assert_eq!(
            catalog.ntranslate(count, "{count} track", "{count} tracks", None),
            match count {
                1 => "{count} utwór",
                2 | 22 => "{count} utwory",
                _ => "{count} utworów",
            }
        );
    }
}

/// Catalogs expected to translate every message in the template.
///
/// The others are filled in as translators reach them: an empty `msgstr`
/// compiles out and shows the English source, as the test above checks. A
/// catalog listed here must stay complete, and every catalog must keep the
/// placeholders of whatever it does translate.
const COMPLETE: &[&str] = &[
    "de-DE", "es", "fr", "it", "ja", "nl", "pl", "pt-BR", "pt-PT", "ru", "sv", "zh-Hans", "zh-Hant",
];

#[test]
fn catalogs_cover_the_template_and_preserve_named_placeholders() {
    // A POT leaves these values for msginit. For this comparison its source
    // language is English; the translator's PO carries its own actual rules.
    let template = include_str!("../assets/i18n/spotifast.pot").replace(
        "nplurals=INTEGER; plural=EXPRESSION;",
        "nplurals=2; plural=(n != 1);",
    );
    let template = polib::po_file::parse_from_reader(template.as_bytes()).unwrap();
    let mut catalogs = 0;
    for file in std::fs::read_dir("assets/i18n").unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "po") {
            continue;
        }
        catalogs += 1;
        let complete = path
            .file_stem()
            .is_some_and(|stem| COMPLETE.contains(&&*stem.to_string_lossy()));
        let translated_catalog = polib::po_file::parse(&path).unwrap();
        assert_eq!(
            template.count(),
            translated_catalog.count(),
            "{}",
            path.display()
        );
        for source in template.messages() {
            let translated = translated_catalog
                .find_message(source.msgctxt(), source.msgid(), source.msgid_plural().ok())
                .unwrap_or_else(|| panic!("missing template entry for {}", source.msgid()));
            assert!(
                !translated.is_fuzzy(),
                "{}: {}",
                path.display(),
                source.msgid()
            );
            if let Ok(forms) = translated.msgstr_plural() {
                assert_eq!(
                    forms.len(),
                    translated_catalog.metadata.plural_rules.nplurals
                );
                for form in forms {
                    assert!(
                        !complete || !form.is_empty(),
                        "{} claims to be complete but {:?} is untranslated",
                        path.display(),
                        source.msgid()
                    );
                    if !form.is_empty() {
                        assert_eq!(named_placeholders(form), named_placeholders(source.msgid()));
                    }
                }
            } else {
                let text = translated.msgstr().unwrap();
                assert!(
                    !complete || !text.is_empty(),
                    "{} claims to be complete but {:?} is untranslated",
                    path.display(),
                    source.msgid()
                );
                if text.is_empty() {
                    continue;
                }
                assert_eq!(
                    named_placeholders(text),
                    named_placeholders(source.msgid()),
                    "{}: {}",
                    path.display(),
                    source.msgid()
                );
            }
        }
    }
    assert_eq!(
        catalogs + 1,
        <spotifast::i18n::Locale as clap::ValueEnum>::value_variants().len()
    );
}

fn named_placeholders(text: &str) -> Vec<&str> {
    let mut names: Vec<_> = text
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .collect();
    names.sort_unstable();
    names
}
