//! Find the translatable texts in Rust sources: the string literals given to
//! `tr!`, `trc!`, `trn!`, `trnc!`, `txt!`, `txtc!`, `txtn!`, `txtnc!`, `n!`
//! and `nc!`. A small lexer skips comments, char literals and items under
//! `#[cfg(test)]`; a `// TRANSLATORS: …` comment right before a message
//! goes to the translators.

/// One message found in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub ctx: Option<String>,
    pub id: String,
    pub plural: Option<String>,
    pub file: String,
    pub line: usize,
    pub comment: Option<String>,
}

/// (macro, has context, has plural)
const MACROS: [(&str, bool, bool); 10] = [
    ("tr", false, false),
    ("trc", true, false),
    ("trn", false, true),
    ("trnc", true, true),
    ("txt", false, false),
    ("txtc", true, false),
    ("txtn", false, true),
    ("txtnc", true, true),
    ("n", false, false),
    ("nc", true, false),
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
    /// `// TRANSLATORS:` comment text.
    Note(String),
}

/// Tokens with their line numbers (only what the extraction needs).
fn lex(src: &str) -> Vec<(Tok, usize)> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            let start = i;
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            let text: String = c[start + 2..i].iter().collect();
            if let Some(n) = text.trim().strip_prefix("TRANSLATORS:") {
                out.push((Tok::Note(n.trim().to_string()), line));
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if c[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if ch == 'r' && (c.get(i + 1) == Some(&'"') || (c.get(i + 1) == Some(&'#') && c.get(i + 2).is_some_and(|x| *x == '"' || *x == '#'))) {
            // Raw string r"…", r#"…"#.
            let start_line = line;
            i += 1;
            let mut hashes = 0;
            while c.get(i) == Some(&'#') {
                hashes += 1;
                i += 1;
            }
            i += 1; // opening quote
            let mut s = String::new();
            loop {
                if i >= c.len() {
                    break;
                }
                if c[i] == '"' && (0..hashes).all(|k| c.get(i + 1 + k) == Some(&'#')) {
                    i += 1 + hashes;
                    break;
                }
                if c[i] == '\n' {
                    line += 1;
                }
                s.push(c[i]);
                i += 1;
            }
            out.push((Tok::Str(s), start_line));
        } else if ch == '"' || (ch == 'b' && c.get(i + 1) == Some(&'"')) {
            let start_line = line;
            if ch == 'b' {
                i += 1;
            }
            i += 1;
            let mut s = String::new();
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' {
                    i += 1;
                    match c.get(i) {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('0') => s.push('\0'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('\'') => s.push('\''),
                        Some('u') => {
                            let close = (i..c.len()).find(|k| c[*k] == '}').unwrap_or(i);
                            let hex: String = c[i + 2..close].iter().collect();
                            if let Some(v) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                s.push(v);
                            }
                            i = close;
                        }
                        Some('x') => {
                            let hex: String = c[i + 1..i + 3].iter().collect();
                            if let Ok(v) = u8::from_str_radix(&hex, 16) {
                                s.push(v as char);
                            }
                            i += 2;
                        }
                        Some('\n') => {
                            // Line continuation: the newline and the indentation vanish.
                            line += 1;
                            while c.get(i + 1).is_some_and(|x| x.is_whitespace()) {
                                if c[i + 1] == '\n' {
                                    line += 1;
                                }
                                i += 1;
                            }
                        }
                        Some(other) => s.push(*other),
                        None => {}
                    }
                    i += 1;
                } else {
                    if c[i] == '\n' {
                        line += 1;
                    }
                    s.push(c[i]);
                    i += 1;
                }
            }
            i += 1;
            out.push((Tok::Str(s), start_line));
        } else if ch == '\'' {
            // A char literal ('a', '\n', '\u{1F600}') or a lifetime ('a).
            if c.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1; // lifetime: the name follows as an identifier
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push((Tok::Ident(c[start..i].iter().collect()), line));
        } else {
            out.push((Tok::Punct(ch), line));
            i += 1;
        }
    }
    out
}

/// Remove the tokens of items marked `#[cfg(test)]` (test modules and functions).
fn without_tests(toks: Vec<(Tok, usize)>) -> Vec<(Tok, usize)> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    let is = |t: &Tok, p: char| *t == Tok::Punct(p);
    while i < toks.len() {
        // # [ cfg ( test ) ]
        let pattern = i + 6 < toks.len()
            && is(&toks[i].0, '#')
            && is(&toks[i + 1].0, '[')
            && toks[i + 2].0 == Tok::Ident("cfg".into())
            && is(&toks[i + 3].0, '(')
            && toks[i + 4].0 == Tok::Ident("test".into())
            && is(&toks[i + 5].0, ')')
            && is(&toks[i + 6].0, ']');
        if !pattern {
            out.push(toks[i].clone());
            i += 1;
            continue;
        }
        // Skip to the item's end: a `;` before any brace, or the matching `}`.
        i += 7;
        let mut depth = 0;
        while i < toks.len() {
            match &toks[i].0 {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                Tok::Punct(';') if depth == 0 => {
                    i += 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
    }
    out
}

/// The messages in one source file.
pub fn extract(file: &str, src: &str) -> Vec<Found> {
    let toks = without_tests(lex(src));
    let mut found = Vec::new();
    let mut note: Option<(String, usize)> = None;
    for i in 0..toks.len() {
        if let Tok::Note(n) = &toks[i].0 {
            note = Some((n.clone(), toks[i].1));
            continue;
        }
        let Tok::Ident(name) = &toks[i].0 else { continue };
        let Some((_, has_ctx, has_plural)) = MACROS.iter().find(|(m, _, _)| m == name) else { continue };
        if toks.get(i + 1).map(|t| &t.0) != Some(&Tok::Punct('!')) || !matches!(toks.get(i + 2).map(|t| &t.0), Some(Tok::Punct('(' | '[' | '{'))) {
            continue;
        }
        // Not a macro of ours when called through another path (`foo::n!`)…
        if i > 1 && matches!(toks[i - 1].0, Tok::Punct(':')) && matches!(toks[i - 2].0, Tok::Punct(':')) && !(i >= 3 && matches!(&toks[i - 3].0, Tok::Ident(p) if p == "rolid_text" || p == "rolid_i18n" || p == "crate")) {
            continue;
        }
        let want = 1 + *has_ctx as usize + *has_plural as usize;
        let mut lits = Vec::new();
        let mut k = i + 3;
        while lits.len() < want {
            match toks.get(k).map(|t| &t.0) {
                Some(Tok::Str(s)) => lits.push(s.clone()),
                _ => break,
            }
            k += 1;
            if lits.len() < want {
                if toks.get(k).map(|t| &t.0) != Some(&Tok::Punct(',')) {
                    break;
                }
                k += 1;
            }
        }
        if lits.len() < want {
            continue; // a macro definition or a call with a non-literal: nothing to extract
        }
        let line = toks[i].1;
        let comment = note.take().filter(|(_, l)| line.saturating_sub(*l) <= 3).map(|(n, _)| n);
        let mut it = lits.into_iter();
        let ctx = if *has_ctx { it.next() } else { None };
        let id = it.next().unwrap();
        let plural = if *has_plural { it.next() } else { None };
        found.push(Found { ctx, id, plural, file: file.to_string(), line, comment });
    }
    found
}

/// UI calls whose string argument is text for the user.
const UI_TEXT_CALLS: [&str; 30] = [
    "label", "button", "small_button", "heading", "on_hover_text", "on_disabled_hover_text", "hint_text", "weak", "strong",
    "colored_label", "checkbox", "radio_value", "radio", "selectable_label", "selectable_value", "menu_button", "from_label",
    "selected_text", "monospace", "text", "btn", "item", "show_tooltip_text", "new", "toggle_value", "link", "hyperlink_to",
    "with_title", "title", "error_text",
];

/// Text for the user that does not go through the translation: string
/// literals with non-English letters, and words given straight to the UI
/// calls above. A line with `i18n-ok` is allowed.
pub fn untranslated(file: &str, src: &str, ui: bool) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let ok = |line: usize| lines.get(line.wrapping_sub(1)).is_some_and(|l| l.contains("i18n-ok"));
    let toks = without_tests(lex(src));
    let mut out = Vec::new();
    for (i, (t, line)) in toks.iter().enumerate() {
        let Tok::Str(s) = t else { continue };
        if ok(*line) {
            continue;
        }
        if s.chars().any(|c| c.is_alphabetic() && !c.is_ascii()) {
            out.push(format!("{file}:{line}: {s:?}"));
            continue;
        }
        if !ui {
            continue;
        }
        // Walk back to the call this literal is an argument of.
        let mut depth = 0;
        let mut k = i;
        let mut call = None;
        while k > 0 {
            k -= 1;
            match &toks[k].0 {
                Tok::Punct(')') | Tok::Punct(']') => depth += 1,
                Tok::Punct('(') | Tok::Punct('[') if depth > 0 => depth -= 1,
                Tok::Punct('(') => {
                    call = match toks.get(k.wrapping_sub(1)).map(|t| &t.0) {
                        Some(Tok::Ident(name)) => Some(name.clone()),
                        _ => None,
                    };
                    break;
                }
                Tok::Punct(';') | Tok::Punct('{') | Tok::Punct('}') => break,
                _ => {}
            }
        }
        let Some(call) = call else { continue };
        // The literal must be a direct argument (not inside tr!(…), format!(…)…).
        if !UI_TEXT_CALLS.contains(&call.as_str()) {
            continue;
        }
        let words = s.split(|c: char| !c.is_ascii_alphabetic()).any(|w| w.len() >= 3 && w.chars().skip(1).all(|c| c.is_ascii_lowercase()));
        // `RichText::new("…")`, `Label::new("…")`: only for these types.
        if call == "new" && !matches!(toks.get(k.wrapping_sub(4)).map(|t| &t.0), Some(Tok::Ident(n)) if ["RichText", "Label", "Button", "Checkbox", "SelectableLabel", "Hyperlink"].contains(&n.as_str())) {
            continue;
        }
        if words {
            out.push(format!("{file}:{line}: {s:?} given to {call}() without tr!"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_messages_and_skips_the_rest() {
        let src = r##"
fn a() {
    // TRANSLATORS: the button in the copy dialog
    ui.button(tr!("Copy"));
    let s = trc!("verb", "Open {name}", name = x);
    let p = trn!("{n} file", "{n} files", count);
    let q = rolid_text::txtnc!("disk", "{n} sector", "{n} sectors", k);
    const C: &str = n!("Pink");
    let w = "tr!(\"not this\")";
    // tr!("nor this")
    /* trc!("x", "nor /* nested */ this") */
    let c = '"'; let l: &'static str = tr!("Line one \
                 continues");
    let raw = tr!(r#"Say "hi""#);
    other::n!("foreign");
    tr!(dynamic);
    format!("{}", 1);
}
#[cfg(test)]
mod tests {
    fn t() { tr!("Only in a test"); }
}
fn after() { txt!("After the tests") }
"##;
        let f = extract("x.rs", src);
        let ids: Vec<_> = f.iter().map(|m| (m.ctx.as_deref(), m.id.as_str(), m.plural.as_deref())).collect();
        assert_eq!(
            ids,
            vec![
                (None, "Copy", None),
                (Some("verb"), "Open {name}", None),
                (None, "{n} file", Some("{n} files")),
                (Some("disk"), "{n} sector", Some("{n} sectors")),
                (None, "Pink", None),
                (None, "Line one continues", None),
                (None, "Say \"hi\"", None),
                (None, "After the tests", None),
            ]
        );
        assert_eq!(f[0].comment.as_deref(), Some("the button in the copy dialog"));
        assert_eq!(f[0].line, 4);
        assert_eq!(f[1].comment, None);
    }

    #[test]
    fn a_field_value_is_not_a_path() {
        let f = extract("x.rs", "fn f() { Failure { error: txt!(\"cannot read\") }; foo::n!(\"no\"); }");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "cannot read");
    }

    #[test]
    fn finds_untranslated_text() {
        let src = r#"
fn ui() {
    ui.label("Kopírovat");
    ui.label(tr!("Copy"));
    ui.button("Copy files");
    ui.label("F5");
    let x = RichText::new("Free space");
    let y = Vec::new("not ui text");
    let p = format!("{dir}/{name}");
    ui.label("Čeština"); // i18n-ok: language name
    tracing::info!("copy started");
}
#[cfg(test)]
mod tests { fn t() { ui.label("Testovací"); } }
"#;
        let v = untranslated("x.rs", src, true);
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v[0].contains("Kopírovat") && v[1].contains("Copy files") && v[2].contains("Free space"));
        assert_eq!(untranslated("x.rs", src, false).len(), 1);
    }
}
