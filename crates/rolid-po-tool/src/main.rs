//! Developer tool for the translations (run from the repository root; also
//! via `scripts/update-po.sh`):
//!
//! - `rolid-po-tool update` — extract the texts from `crates/*/src` into
//!   `po/rolidscp.pot` and merge it into every `po/<lang>.po` (new messages
//!   come in untranslated, translations of unchanged messages stay, messages
//!   gone from the code are dropped and listed).
//! - `rolid-po-tool stats` — how much of each language is translated.
//! - `rolid-po-tool check` — every .po parses and every translation has the
//!   same placeholders as the English text (exit code 1 otherwise).
//! - `rolid-po-tool lint` — text for the user left outside the translation.
//!
//! Needs nothing but cargo (no gettext tools).

mod extract;
mod po;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(".");
    let r = match args.first().map(String::as_str) {
        Some("update") => update(&root),
        Some("extract") => extract_pot(&root).map(|n| println!("po/rolidscp.pot: {n} messages")),
        Some("stats") => stats(&root),
        Some("check") => check(&root),
        Some("lint") => {
            let p = untranslated(&root);
            p.iter().for_each(|l| println!("{l}"));
            if p.is_empty() { Ok(()) } else { Err(format!("{} untranslated texts", p.len())) }
        }
        _ => Err("usage: rolid-po-tool update | extract | stats | check | lint  (run in the repository root)".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// All messages of the code, merged by (context, msgid), in source order.
pub fn collect(root: &Path) -> Result<Vec<po::Entry>, String> {
    let mut files = Vec::new();
    let crates = root.join("crates");
    for c in std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let src = c.map_err(|e| e.to_string())?.path().join("src");
        if src.is_dir() && !src.starts_with(crates.join("rolid-po-tool")) {
            walk(&src, &mut files);
        }
    }
    files.sort();
    let mut entries: Vec<po::Entry> = Vec::new();
    for f in files {
        let name = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        if name.ends_with("/tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&f).map_err(|e| format!("{name}: {e}"))?;
        for m in extract::extract(&name, &src) {
            let loc = format!("{}:{}", m.file, m.line);
            match entries.iter_mut().find(|e| e.ctx == m.ctx && e.id == m.id) {
                Some(e) => {
                    if e.plural != m.plural {
                        return Err(format!("{loc}: \"{}\" is used with different plurals", m.id));
                    }
                    e.refs.push(loc);
                    if let Some(c) = m.comment {
                        if !e.comments.contains(&c) {
                            e.comments.push(c);
                        }
                    }
                }
                None => entries.push(po::Entry {
                    ctx: m.ctx,
                    id: m.id,
                    plural: m.plural,
                    refs: vec![loc],
                    comments: m.comment.into_iter().collect(),
                    ..Default::default()
                }),
            }
        }
    }
    Ok(entries)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn extract_pot(root: &Path) -> Result<usize, String> {
    let entries = collect(root)?;
    let pot = po::File { header: po::pot_header(), entries };
    std::fs::write(root.join("po/rolidscp.pot"), pot.write()).map_err(|e| e.to_string())?;
    Ok(pot.entries.len())
}

fn languages(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(root.join("po"))
        .map_err(|e| format!("po/: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "po"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().to_string(), p))
        .collect();
    v.sort();
    Ok(v)
}

fn update(root: &Path) -> Result<(), String> {
    let n = extract_pot(root)?;
    println!("po/rolidscp.pot: {n} messages");
    let pot = collect(root)?;
    for (lang, path) in languages(root)? {
        let old = po::File::read(&path)?;
        let (new, dropped) = po::merge(&old, &pot);
        std::fs::write(&path, new.write()).map_err(|e| e.to_string())?;
        let (done, total) = new.coverage();
        println!("{lang}: {done}/{total} translated, {} dropped", dropped.len());
        for d in dropped {
            println!("  dropped: {}{}", d.ctx.map(|c| format!("[{c}] ")).unwrap_or_default(), d.id);
        }
    }
    Ok(())
}

fn stats(root: &Path) -> Result<(), String> {
    let total = collect(root)?.len();
    println!("messages in the code: {total}");
    for (lang, path) in languages(root)? {
        let f = po::File::read(&path)?;
        let (done, all) = f.coverage();
        let pct = if all == 0 { 0.0 } else { 100.0 * done as f64 / all as f64 };
        let note = if all != total { " (run update: the .po is out of date)" } else { "" };
        let missing = f.untranslated(&lang).len();
        let news = f.entries.iter().filter(|e| e.ctx.as_deref() == Some(po::NEWS)).count();
        let required = if lang == "cs" { all } else { all - news };
        let req = 100.0 * (required - missing) as f64 / required.max(1) as f64;
        println!("{lang}: {done}/{all} = {pct:.1} %, required {}/{required} = {req:.1} %{note}", required - missing);
    }
    Ok(())
}

fn check(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    for (lang, path) in languages(root)? {
        // The same parser as the build.
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        if let Err(e) = polib::po_file::parse_from_reader(file) {
            problems.push(format!("{lang}: {e}"));
            continue;
        }
        let f = po::File::read(&path)?;
        problems.extend(f.placeholder_problems().into_iter().map(|p| format!("{lang}: {p}")));
    }
    if problems.is_empty() {
        println!("ok");
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Text for the user outside the translation (see `extract::untranslated`).
fn untranslated(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for c in std::fs::read_dir(root.join("crates")).into_iter().flatten().flatten() {
        if c.file_name() != "rolid-po-tool" {
            walk(&c.path().join("src"), &mut files);
        }
    }
    files.sort();
    let mut problems = Vec::new();
    for f in files {
        let name = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        if name.ends_with("/tests.rs") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&f) else { continue };
        problems.extend(extract::untranslated(&name, &src, name.contains("rolid-ui/")));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No text for the user outside the translation layer: non-English
    /// letters in string literals of any crate (outside tests), and plain
    /// words given to UI calls in the UI crate.
    #[test]
    fn no_untranslated_text_in_the_code() {
        let p = untranslated(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        assert!(p.is_empty(), "{} untranslated texts:\n{}", p.len(), p.join("\n"));
    }

    /// Every language translates every message of the code (the release
    /// notes may stay English) and no .po is behind the code.
    #[test]
    fn translations_are_complete_and_up_to_date() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let code: Vec<(Option<String>, String)> = collect(&root).unwrap().into_iter().map(|e| (e.ctx, e.id)).collect();
        let langs = languages(&root).unwrap();
        assert_eq!(langs.len(), 7, "{langs:?}");
        for (lang, path) in langs {
            let f = po::File::read(&path).unwrap();
            let have: Vec<(Option<String>, String)> = f.entries.iter().map(|e| (e.ctx.clone(), e.id.clone())).collect();
            let behind: Vec<_> = code.iter().filter(|k| !have.contains(k)).collect();
            assert!(behind.is_empty(), "{lang}: not in the .po (run scripts/update-po.sh): {behind:?}");
            let missing = f.untranslated(&lang);
            assert!(missing.is_empty(), "{lang}: {} untranslated: {missing:?}", missing.len());
        }
    }

    /// Every .po parses and keeps the placeholders of the English text.
    #[test]
    fn translations_are_well_formed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        check(&root).unwrap();
    }
}
