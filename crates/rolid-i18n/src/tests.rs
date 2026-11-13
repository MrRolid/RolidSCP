use super::*;
use crate::rolid_text::{txt, txtc, txtn};

/// The active language is global: tests that switch it take turns.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn langs() -> Vec<String> {
    LANGUAGES.iter().map(|(c, _)| c.to_string()).collect()
}

#[test]
fn language_from_the_environment() {
    let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    let l = langs();
    assert_eq!(language_from_env(env(&[("LANG", "cs_CZ.UTF-8")]), &l), "cs");
    assert_eq!(language_from_env(env(&[("LANG", "de_DE.UTF-8"), ("LC_MESSAGES", "fr_FR.UTF-8")]), &l), "fr");
    assert_eq!(language_from_env(env(&[("LANG", "de_DE.UTF-8"), ("LC_MESSAGES", "fr_FR"), ("LC_ALL", "pl_PL.UTF-8")]), &l), "pl");
    // An empty LC_ALL does not count (as in glibc).
    assert_eq!(language_from_env(env(&[("LC_ALL", ""), ("LANG", "es_ES.UTF-8")]), &l), "es");
    assert_eq!(language_from_env(env(&[("LANG", "nb_NO.UTF-8")]), &l), "nb");
    assert_eq!(language_from_env(env(&[("LANG", "no_NO")]), &l), "nb");
    assert_eq!(language_from_env(env(&[("LANG", "da_DK.UTF-8@euro")]), &l), "da");
    for unsupported in ["C", "POSIX", "C.UTF-8", "it_IT.UTF-8", "ja_JP", "en_GB.UTF-8"] {
        let v: &'static str = Box::leak(unsupported.to_string().into_boxed_str());
        let pairs: &'static [(&str, &str)] = Box::leak(vec![("LANG", v)].into_boxed_slice());
        assert_eq!(language_from_env(env(pairs), &l), "en", "{unsupported}");
    }
    assert_eq!(language_from_env(env(&[]), &l), "en");
    // A language added by the user's file.
    let mut with_it = l.clone();
    with_it.push("it".into());
    assert_eq!(language_from_env(env(&[("LANG", "it_IT.UTF-8")]), &with_it), "it");
}

const CS_PO: &str = r#"
msgid ""
msgstr ""
"Language: cs\n"
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=3; plural=(n==1) ? 0 : (n>=2 && n<=4) ? 1 : 2;\n"

msgid "Copy"
msgstr "Kopírovat"

msgctxt "verb"
msgid "Open"
msgstr "Otevřít"

msgid "Copy {name} to {dir}"
msgstr "Kopírovat {name} do {dir}"

msgid "{n} file"
msgid_plural "{n} files"
msgstr[0] "{n} soubor"
msgstr[1] "{n} soubory"
msgstr[2] "{n} souborů"

#, fuzzy
msgid "Delete"
msgstr "Smazat (nejisté)"

msgid "Refresh"
msgstr ""
"#;

const PL_PO: &str = r#"
msgid ""
msgstr ""
"Language: pl\n"
"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\n"

msgid "{n} file"
msgid_plural "{n} files"
msgstr[0] "{n} plik"
msgstr[1] "{n} pliki"
msgstr[2] "{n} plików"
"#;

fn with_po(lang: &str, po: &str) -> Active {
    let mut c = Catalog::new(lang);
    c.merge_po(po.as_bytes()).unwrap();
    Active { lang: lang.into(), catalog: Arc::new(c), locale: Locale::of(lang) }
}

#[test]
fn plural_forms_for_czech_polish_and_english() {
    let counts = [0u64, 1, 2, 5, 21, 22];
    let files = |a: &Active| counts.iter().map(|n| translate_with(a, &txtn!("{n} file", "{n} files", *n))).collect::<Vec<_>>();
    let cs = with_po("cs", CS_PO);
    assert_eq!(files(&cs), ["0 souborů", "1 soubor", "2 soubory", "5 souborů", "21 souborů", "22 souborů"]);
    let pl = with_po("pl", PL_PO);
    assert_eq!(files(&pl), ["0 plików", "1 plik", "2 pliki", "5 plików", "21 plików", "22 pliki"]);
    let en = Active { lang: "en".into(), catalog: Arc::new(Catalog::new("en")), locale: Locale::EN };
    assert_eq!(files(&en), ["0 files", "1 file", "2 files", "5 files", "21 files", "22 files"]);
    // The count is written the language's way.
    assert_eq!(translate_with(&cs, &txtn!("{n} file", "{n} files", 12_345u64)), "12\u{a0}345 souborů");
    assert_eq!(translate_with(&en, &txtn!("{n} file", "{n} files", 12_345u64)), "12,345 files");
}

#[test]
fn missing_translations_fall_back_to_english() {
    let cs = with_po("cs", CS_PO);
    assert_eq!(translate_with(&cs, &txt!("Copy")), "Kopírovat");
    assert_eq!(translate_with(&cs, &txtc!("verb", "Open")), "Otevřít");
    // Same text without the context, another context, untranslated, fuzzy, unknown:
    for (t, en) in [
        (txt!("Open"), "Open"),
        (txtc!("adjective", "Open"), "Open"),
        (txt!("Refresh"), "Refresh"),
        (txt!("Delete"), "Delete"),
        (txt!("Never seen"), "Never seen"),
    ] {
        assert_eq!(translate_with(&cs, &t), en);
    }
    // Placeholders and nested messages are filled in after the translation.
    let t = txt!("Copy {name} to {dir}", name = "a.txt", dir = txt!("Copy"));
    assert_eq!(translate_with(&cs, &t), "Kopírovat a.txt do Kopírovat");
    // An unknown language: English.
    let _g = lock();
    let a = set_language(Some("xx"), false, None);
    assert!(["en", "cs", "de", "fr", "pl", "es", "nb", "da"].contains(&a.lang.as_str()));
    let a = set_language(Some("en"), false, None);
    assert_eq!(a.lang, "en");
    assert_eq!(tr!("Copy {n} files", n = 3), "Copy 3 files");
    assert_eq!(trn!("{n} file", "{n} files", 1), "1 file");
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-i18n-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn user_po_wins_over_the_built_in_and_a_broken_one_is_ignored() {
    let _g = lock();
    let dir = temp_dir("user");
    // A new language from the user.
    std::fs::write(
        dir.join("it.po"),
        "msgid \"\"\nmsgstr \"\"\n\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n\nmsgid \"Copy\"\nmsgstr \"Copia\"\n",
    )
    .unwrap();
    assert!(available(Some(&dir)).contains(&"it".to_string()));
    let a = set_language(Some("it"), false, Some(&dir));
    assert_eq!((a.lang.as_str(), a.warnings.len()), ("it", 0));
    assert_eq!(tr!("Copy"), "Copia");
    assert_eq!(tr!("Move"), "Move");
    // The user's Czech over the built-in Czech: their message wins, the rest stays.
    std::fs::write(dir.join("cs.po"), "msgid \"\"\nmsgstr \"\"\n\"Plural-Forms: nplurals=3; plural=(n==1) ? 0 : (n>=2 && n<=4) ? 1 : 2;\\n\"\n\nmsgid \"Copy\"\nmsgstr \"Zkopírovat\"\n").unwrap();
    let mut c = Catalog::new("cs");
    c.merge_po(CS_PO.as_bytes()).unwrap();
    let (user, w) = load("cs", Some(&dir));
    assert!(w.is_empty());
    let mut merged = c.clone();
    merged.merge_po(&std::fs::read(dir.join("cs.po")).unwrap()).unwrap();
    assert_eq!(merged.template(&txt!("Copy")), "Zkopírovat");
    assert_eq!(merged.template(&txtc!("verb", "Open")), "Otevřít");
    assert_eq!(user.template(&txt!("Copy")), "Zkopírovat");
    // Broken files: a warning, the built-in catalog, no panic.
    for (name, bytes) in [
        ("bad-utf8", b"msgid \"Copy\"\nmsgstr \"\xff\xfe\"\n".to_vec()),
        ("bad-syntax", b"msgid \"Copy\nmsgstr Kopie\"\n\"\"\"\n".to_vec()),
        ("bad-plural", b"msgid \"\"\nmsgstr \"\"\n\"Plural-Forms: nplurals=3; plural=n ===;\\n\"\n".to_vec()),
        ("binary", (0u8..=255).cycle().take(4096).collect()),
    ] {
        let d = temp_dir(name);
        std::fs::write(d.join("cs.po"), &bytes).unwrap();
        let (cat, w) = load("cs", Some(&d));
        assert_eq!(w.len(), 1, "{name}: {w:?}");
        assert_eq!(cat.len(), Catalog::embedded("cs").len(), "{name}");
        let a = set_language(Some("cs"), false, Some(&d));
        assert_eq!(a.warnings.len(), 1, "{name}");
        std::fs::remove_file(d.join("cs.po")).unwrap();
        std::fs::remove_dir(&d).unwrap();
    }
    for f in ["it.po", "cs.po"] {
        std::fs::remove_file(dir.join(f)).unwrap();
    }
    std::fs::remove_dir(&dir).unwrap();
    set_language(Some("en"), false, None);
}

#[test]
fn formats_follow_the_language_and_the_iso_choice() {
    let _g = lock();
    use chrono::TimeZone;
    let when: std::time::SystemTime = chrono::Local.with_ymd_and_hms(2026, 11, 8, 21, 8, 0).unwrap().into();
    set_language(Some("en"), false, None);
    assert_eq!((datetime(when, false), num(1234), size(40_000_000)), ("2026-11-08 21:08".into(), "1,234".into(), "38.1 MB".into()));
    set_language(Some("cs"), false, None);
    assert_eq!((datetime(when, false), num(1234), decimal(2.5, 1)), ("08.11.2026 21:08".into(), "1\u{a0}234".into(), "2,5".into()));
    set_language(Some("cs"), true, None);
    assert_eq!(datetime(when, false), "2026-11-08 21:08");
    assert_eq!(num(1234), "1\u{a0}234", "ISO dates keep the language's numbers");
    set_language(Some("en"), false, None);
}
