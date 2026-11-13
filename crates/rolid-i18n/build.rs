//! Compile the translations in `po/*.po` into the binary: each catalog becomes
//! a static table of (context, msgid, forms) in `$OUT_DIR/catalogs.rs`.
//! Untranslated and fuzzy messages are left out (the UI shows English).
//! A .po file that does not parse fails the build.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let po_dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../po");
    println!("cargo:rerun-if-changed={}", po_dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&po_dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "po"));
    files.sort();
    let mut out = String::from("pub static EMBEDDED: &[Embedded] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let lang = path.file_stem().unwrap().to_string_lossy().to_string();
        let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let catalog = polib::po_file::parse_from_reader(file).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let rules = &catalog.metadata.plural_rules;
        let plural_forms = format!("nplurals={}; plural={};", rules.nplurals, rules.expr);
        writeln!(out, "    Embedded {{ lang: {lang:?}, plural_forms: {plural_forms:?}, entries: &[").unwrap();
        for m in catalog.messages() {
            if m.is_fuzzy() || !m.is_translated() || m.msgid().is_empty() {
                continue;
            }
            let ctx = m.msgctxt().unwrap_or("");
            let forms: Vec<&str> = if m.is_plural() {
                m.msgstr_plural().unwrap().iter().map(String::as_str).collect()
            } else {
                vec![m.msgstr().unwrap()]
            };
            writeln!(out, "        ({ctx:?}, {:?}, &{forms:?}),", m.msgid()).unwrap();
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalogs.rs");
    std::fs::write(dest, out).unwrap();
}
