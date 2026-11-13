//! Reading and writing .po / .pot files with everything the tool keeps:
//! the header and its comments, translator comments, flags.

use rolid_text::placeholders;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub ctx: Option<String>,
    pub id: String,
    pub plural: Option<String>,
    /// msgstr, or msgstr[0..] for a plural message.
    pub strs: Vec<String>,
    /// `#: file:line`
    pub refs: Vec<String>,
    /// `#. ` comments for translators (from `// TRANSLATORS:`).
    pub comments: Vec<String>,
    /// `# ` comments written by translators.
    pub translator: Vec<String>,
    /// `#, fuzzy, …`
    pub flags: Vec<String>,
}

impl Entry {
    pub fn translated(&self) -> bool {
        !self.flags.iter().any(|f| f == "fuzzy") && !self.strs.is_empty() && self.strs.iter().all(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Header {
    /// Comment lines before the header (`# …`), without the "# ".
    pub comments: Vec<String>,
    /// The header msgstr ("Language: cs\nPlural-Forms: …\n").
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct File {
    pub header: Header,
    pub entries: Vec<Entry>,
}

/// Context of the release notes in About (English except in Czech).
pub const NEWS: &str = "news";

pub fn pot_header() -> Header {
    Header {
        comments: vec![
            "Translatable texts of RolidSCP, generated from the code by `scripts/update-po.sh`.".into(),
            "Do not edit: edit po/<language>.po instead.".into(),
        ],
        text: "Project-Id-Version: RolidSCP\nReport-Msgid-Bugs-To: https://github.com/MrRolid/RolidSCP/issues\nMIME-Version: 1.0\nContent-Type: text/plain; charset=UTF-8\nContent-Transfer-Encoding: 8bit\nLanguage: \nPlural-Forms: nplurals=2; plural=(n != 1);\n".into(),
    }
}

fn unquote(s: &str, line: usize) -> Result<String, String> {
    let s = s.trim();
    if !(s.len() >= 2 && s.starts_with('"') && s.ends_with('"')) {
        return Err(format!("line {line}: expected a quoted string"));
    }
    let inner = &s[1..s.len() - 1];
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(o) => return Err(format!("line {line}: unknown escape \\{o}")),
            None => return Err(format!("line {line}: trailing backslash")),
        }
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `keyword "text"`, split after each "\n" like gettext does.
fn field(out: &mut String, key: &str, s: &str) {
    let parts: Vec<&str> = s.split_inclusive('\n').collect();
    if parts.len() <= 1 {
        out.push_str(&format!("{key} {}\n", quote(s)));
    } else {
        out.push_str(&format!("{key} \"\"\n"));
        for p in parts {
            out.push_str(&quote(p));
            out.push('\n');
        }
    }
}

impl File {
    pub fn read(path: &std::path::Path) -> Result<File, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        File::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<File, String> {
        let mut file = File::default();
        let mut cur = Entry::default();
        let mut started = false;
        // Which field the continuation lines belong to.
        #[derive(Clone, Copy, PartialEq)]
        enum F {
            None,
            Ctx,
            Id,
            Plural,
            Str(usize),
        }
        let mut f = F::None;
        let finish = |cur: &mut Entry, started: &mut bool, file: &mut File| {
            if !*started {
                return;
            }
            let e = std::mem::take(cur);
            if e.id.is_empty() && e.ctx.is_none() && file.entries.is_empty() && file.header.text.is_empty() {
                file.header.text = e.strs.first().cloned().unwrap_or_default();
                file.header.comments = e.translator;
            } else {
                file.entries.push(e);
            }
            *started = false;
        };
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let l = raw.trim_end();
            if l.trim().is_empty() {
                finish(&mut cur, &mut started, &mut file);
                f = F::None;
                continue;
            }
            if let Some(rest) = l.strip_prefix('#') {
                if started && f != F::None {
                    finish(&mut cur, &mut started, &mut file);
                    f = F::None;
                }
                if rest.starts_with('~') {
                    continue; // obsolete entries are not kept
                }
                match rest.chars().next() {
                    Some(':') => cur.refs.extend(rest[1..].split_whitespace().map(String::from)),
                    Some('.') => cur.comments.push(rest[1..].trim().to_string()),
                    Some(',') => cur.flags.extend(rest[1..].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
                    Some('|') => {}
                    _ => cur.translator.push(rest.strip_prefix(' ').unwrap_or(rest).to_string()),
                }
                started = true;
                continue;
            }
            let l = l.trim_start();
            if l.starts_with('"') {
                let s = unquote(l, line)?;
                match f {
                    F::Ctx => cur.ctx.get_or_insert_with(String::new).push_str(&s),
                    F::Id => cur.id.push_str(&s),
                    F::Plural => cur.plural.get_or_insert_with(String::new).push_str(&s),
                    F::Str(i) => cur.strs[i].push_str(&s),
                    F::None => return Err(format!("line {line}: string without a keyword")),
                }
                continue;
            }
            let (key, val) = l.split_once(char::is_whitespace).ok_or(format!("line {line}: cannot read \"{l}\""))?;
            if (key == "msgctxt" || (key == "msgid" && f != F::Ctx)) && started && f != F::None {
                finish(&mut cur, &mut started, &mut file);
            }
            started = true;
            let s = unquote(val, line)?;
            f = match key {
                "msgctxt" => {
                    cur.ctx = Some(s);
                    F::Ctx
                }
                "msgid" => {
                    cur.id = s;
                    F::Id
                }
                "msgid_plural" => {
                    cur.plural = Some(s);
                    F::Plural
                }
                "msgstr" => {
                    cur.strs = vec![s];
                    F::Str(0)
                }
                k if k.starts_with("msgstr[") && k.ends_with(']') => {
                    let i: usize = k[7..k.len() - 1].parse().map_err(|_| format!("line {line}: bad index {k}"))?;
                    if i != cur.strs.len() {
                        return Err(format!("line {line}: {k} out of order"));
                    }
                    cur.strs.push(s);
                    F::Str(i)
                }
                k => return Err(format!("line {line}: unknown keyword {k}")),
            };
        }
        finish(&mut cur, &mut started, &mut file);
        Ok(file)
    }

    pub fn write(&self) -> String {
        let mut out = String::new();
        for c in &self.header.comments {
            out.push_str(if c.is_empty() { "#\n" } else { "# " });
            if !c.is_empty() {
                out.push_str(c);
                out.push('\n');
            }
        }
        out.push_str("msgid \"\"\n");
        field(&mut out, "msgstr", &self.header.text);
        for e in &self.entries {
            out.push('\n');
            for t in &e.translator {
                out.push_str(&format!("# {t}\n"));
            }
            for c in &e.comments {
                out.push_str(&format!("#. {c}\n"));
            }
            for r in &e.refs {
                out.push_str(&format!("#: {r}\n"));
            }
            if !e.flags.is_empty() {
                out.push_str(&format!("#, {}\n", e.flags.join(", ")));
            }
            if let Some(c) = &e.ctx {
                field(&mut out, "msgctxt", c);
            }
            field(&mut out, "msgid", &e.id);
            match &e.plural {
                Some(p) => {
                    field(&mut out, "msgid_plural", p);
                    let n = e.strs.len().max(2);
                    for i in 0..n {
                        field(&mut out, &format!("msgstr[{i}]"), e.strs.get(i).map(String::as_str).unwrap_or(""));
                    }
                }
                None => field(&mut out, "msgstr", e.strs.first().map(String::as_str).unwrap_or("")),
            }
        }
        out
    }

    /// nplurals from the header (2 when missing).
    pub fn nplurals(&self) -> usize {
        self.header
            .text
            .lines()
            .find_map(|l| l.strip_prefix("Plural-Forms:"))
            .and_then(|v| v.split(';').find_map(|p| p.trim().strip_prefix("nplurals=")?.trim().parse().ok()))
            .unwrap_or(2)
    }

    /// (translated, all)
    pub fn coverage(&self) -> (usize, usize) {
        (self.entries.iter().filter(|e| e.translated()).count(), self.entries.len())
    }

    /// Messages that must be translated but are not. The release notes
    /// (`msgctxt "news"`) stay English in every language but Czech.
    pub fn untranslated(&self, lang: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| !e.translated() && (lang == "cs" || e.ctx.as_deref() != Some(NEWS)))
            .map(|e| e.ctx.as_ref().map(|c| format!("[{c}] {}", e.id)).unwrap_or_else(|| e.id.clone()))
            .collect()
    }

    /// Translations whose placeholders differ from the English text, and
    /// plural messages with a wrong number of forms.
    pub fn placeholder_problems(&self) -> Vec<String> {
        let np = self.nplurals();
        let mut v = Vec::new();
        for e in self.entries.iter().filter(|e| e.translated()) {
            let mut want = placeholders(&e.id);
            if let Some(p) = &e.plural {
                want.extend(placeholders(p));
                want.sort();
                want.dedup();
                if e.strs.len() != np {
                    v.push(format!("\"{}\": {} plural forms, the header says {np}", e.id, e.strs.len()));
                }
            }
            for s in &e.strs {
                let mut got = placeholders(s);
                if e.plural.is_some() {
                    // A form may leave out {n} ("one file" for n = 1) but add nothing.
                    got.retain(|x| x != "n");
                    let w: Vec<_> = want.iter().filter(|x| *x != "n").cloned().collect();
                    if got != w {
                        v.push(format!("\"{}\": placeholders {:?} in \"{s}\", expected {:?}", e.id, got, w));
                    }
                } else if got != want {
                    v.push(format!("\"{}\": placeholders {:?} in \"{s}\", expected {:?}", e.id, got, want));
                }
            }
        }
        v
    }
}

/// The new .po for `pot`: translations of `old` kept where the message is
/// the same; returns the messages that are gone from the code.
pub fn merge(old: &File, pot: &[Entry]) -> (File, Vec<Entry>) {
    let np = old.nplurals();
    let mut entries = Vec::with_capacity(pot.len());
    for p in pot {
        let mut e = p.clone();
        match old.entries.iter().find(|o| o.ctx == p.ctx && o.id == p.id) {
            Some(o) if o.plural.is_some() == p.plural.is_some() => {
                e.strs = o.strs.clone();
                e.translator = o.translator.clone();
                e.flags = o.flags.clone();
                // An English plural changed: the translation needs a look.
                if o.plural != p.plural && !e.flags.iter().any(|f| f == "fuzzy") {
                    e.flags.push("fuzzy".into());
                }
            }
            _ => e.strs = vec![String::new(); if p.plural.is_some() { np } else { 1 }],
        }
        entries.push(e);
    }
    let dropped = old.entries.iter().filter(|o| !pot.iter().any(|p| p.ctx == o.ctx && p.id == o.id)).cloned().collect();
    (File { header: old.header.clone(), entries }, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PO: &str = r#"# Czech translation of RolidSCP.
# Machine translation, waiting for a review.
msgid ""
msgstr ""
"Language: cs\n"
"Plural-Forms: nplurals=3; plural=(n==1) ? 0 : (n>=2 && n<=4) ? 1 : 2;\n"

# checked
#. the button
#: crates/a.rs:1
msgctxt "verb"
msgid "Open"
msgstr "Otevřít"

#: crates/a.rs:2
msgid "{n} file"
msgid_plural "{n} files"
msgstr[0] "{n} soubor"
msgstr[1] "{n} soubory"
msgstr[2] "{n} souborů"

#, fuzzy
msgid "Two\nlines"
msgstr ""
"Dva\n"
"řádky"

msgid "Gone"
msgstr "Pryč"
"#;

    #[test]
    fn read_write_round_trip_and_merge() {
        let f = File::parse(PO).unwrap();
        assert_eq!(f.header.comments, ["Czech translation of RolidSCP.", "Machine translation, waiting for a review."]);
        assert_eq!(f.nplurals(), 3);
        assert_eq!(f.entries.len(), 4);
        assert_eq!(f.entries[0].ctx.as_deref(), Some("verb"));
        assert_eq!(f.entries[0].translator, ["checked"]);
        assert_eq!(f.entries[1].strs.len(), 3);
        assert_eq!(f.entries[2].strs, ["Dva\nřádky"]);
        assert!(!f.entries[2].translated());
        assert_eq!(File::parse(&f.write()).unwrap(), f);
        assert_eq!(f.coverage(), (3, 4));
        let pot = vec![
            Entry { ctx: Some("verb".into()), id: "Open".into(), refs: vec!["crates/b.rs:9".into()], ..Default::default() },
            Entry { id: "{n} file".into(), plural: Some("{n} files".into()), ..Default::default() },
            Entry { id: "New".into(), ..Default::default() },
        ];
        let (m, dropped) = merge(&f, &pot);
        assert_eq!(m.entries[0].strs, ["Otevřít"]);
        assert_eq!(m.entries[0].refs, ["crates/b.rs:9"]);
        assert_eq!(m.entries[1].strs.len(), 3);
        assert_eq!(m.entries[2].strs, [""]);
        assert_eq!(dropped.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["Two\nlines", "Gone"]);
        // polib reads what we write.
        polib::po_file::parse_from_reader(m.write().as_bytes()).unwrap();
    }

    #[test]
    fn placeholders_must_match() {
        let po = "msgid \"\"\nmsgstr \"Plural-Forms: nplurals=3; plural=0;\\n\"\n\nmsgid \"Copy {a}\"\nmsgstr \"Kopie {b}\"\n\nmsgid \"{n} file in {dir}\"\nmsgid_plural \"{n} files in {dir}\"\nmsgstr[0] \"jeden soubor v {dir}\"\nmsgstr[1] \"{n} soubory v {dir}\"\nmsgstr[2] \"{n} souborů\"\n";
        let p = File::parse(po).unwrap().placeholder_problems();
        assert_eq!(p.len(), 2, "{p:?}");
    }
}
