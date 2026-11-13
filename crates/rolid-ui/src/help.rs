//! The F1 help: `help/<lang>.md` built into the program (English when the
//! language has none), drawn by a small Markdown subset: `#`/`##` headings,
//! paragraphs, `- ` lists, `**bold**` and `` `code` ``.

use eframe::egui::{self, text::LayoutJob, TextFormat, TextStyle};

/// The help pages built in, by language code.
const PAGES: &[(&str, &str)] = &[
    ("en", include_str!("../../../help/en.md")),
    ("cs", include_str!("../../../help/cs.md")),
    ("de", include_str!("../../../help/de.md")),
    ("fr", include_str!("../../../help/fr.md")),
    ("pl", include_str!("../../../help/pl.md")),
    ("es", include_str!("../../../help/es.md")),
    ("nb", include_str!("../../../help/nb.md")),
    ("da", include_str!("../../../help/da.md")),
];

/// The help text for `lang` (English when there is none).
pub fn text(lang: &str) -> &'static str {
    PAGES.iter().find(|(l, _)| *l == lang).or_else(|| PAGES.iter().find(|(l, _)| *l == "en")).map(|(_, t)| *t).unwrap_or("")
}

/// A piece of the page.
#[derive(Debug, PartialEq)]
pub enum Block {
    Title(String),
    Heading(String),
    Para(String),
    Item(String),
}

/// Split Markdown into blocks; lines continuing a paragraph or a list item
/// are joined with a space.
pub fn parse(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut open = false; // the last block takes continuation lines
    for line in md.lines() {
        let l = line.trim();
        if l.is_empty() {
            open = false;
        } else if let Some(h) = l.strip_prefix("## ") {
            out.push(Block::Heading(h.trim().to_string()));
            open = false;
        } else if let Some(h) = l.strip_prefix("# ") {
            out.push(Block::Title(h.trim().to_string()));
            open = false;
        } else if let Some(i) = l.strip_prefix("- ") {
            out.push(Block::Item(i.trim().to_string()));
            open = true;
        } else if open {
            if let Some(Block::Para(s) | Block::Item(s)) = out.last_mut() {
                s.push(' ');
                s.push_str(l);
            }
        } else {
            out.push(Block::Para(l.to_string()));
            open = true;
        }
    }
    out
}

/// Inline spans: (text, bold, code).
pub fn spans(s: &str) -> Vec<(String, bool, bool)> {
    let mut out = Vec::new();
    let (mut bold, mut code) = (false, false);
    let mut buf = String::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if !code && rest.starts_with("**") {
            if !buf.is_empty() {
                out.push((std::mem::take(&mut buf), bold, code));
            }
            bold = !bold;
            rest = &rest[2..];
        } else if c == '`' {
            if !buf.is_empty() {
                out.push((std::mem::take(&mut buf), bold, code));
            }
            code = !code;
            rest = &rest[1..];
        } else {
            buf.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !buf.is_empty() {
        out.push((buf, bold, code));
    }
    out
}

fn job(ui: &egui::Ui, s: &str) -> LayoutJob {
    let body = TextStyle::Body.resolve(ui.style());
    let mono = TextStyle::Monospace.resolve(ui.style());
    let (normal, strong) = (ui.visuals().text_color(), ui.visuals().strong_text_color());
    let mut job = LayoutJob::default();
    for (t, bold, code) in spans(s) {
        let font_id = if code { mono.clone() } else { body.clone() };
        let color = if bold { strong } else { normal };
        let background = if code { ui.visuals().code_bg_color } else { egui::Color32::TRANSPARENT };
        job.append(&t, 0.0, TextFormat { font_id, color, background, ..Default::default() });
    }
    job
}

/// Draw the page (without its `#` title, which the window shows).
pub fn show(ui: &mut egui::Ui, md: &str) {
    for b in parse(md) {
        match b {
            Block::Title(_) => {}
            Block::Heading(h) => {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(h).strong().size(TextStyle::Body.resolve(ui.style()).size * 1.15));
                ui.add_space(2.0);
            }
            Block::Para(p) => {
                ui.add(egui::Label::new(job(ui, &p)).wrap());
                ui.add_space(4.0);
            }
            Block::Item(i) => {
                ui.horizontal_top(|ui| {
                    ui.label("•");
                    ui.add(egui::Label::new(job(ui, &i)).wrap());
                });
            }
        }
    }
}

/// The title of the page (its `#` line).
pub fn title(md: &str) -> Option<String> {
    parse(md).into_iter().find_map(|b| match b {
        Block::Title(t) => Some(t),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_subset() {
        let md = "# Title\n\nSome text\ngoes on.\n\n## Part\n\n- one\n  more\n- two\n";
        assert_eq!(
            parse(md),
            vec![
                Block::Title("Title".into()),
                Block::Para("Some text goes on.".into()),
                Block::Heading("Part".into()),
                Block::Item("one more".into()),
                Block::Item("two".into()),
            ]
        );
        assert_eq!(
            spans("a **b** `c*` d"),
            vec![("a ".into(), false, false), ("b".into(), true, false), (" ".into(), false, false), ("c*".into(), false, true), (" d".into(), false, false)]
        );
    }

    #[test]
    fn every_language_has_help_and_english_is_the_fallback() {
        for (code, _) in rolid_i18n::LANGUAGES {
            assert!(PAGES.iter().any(|(l, _)| *l == code), "no help/{code}.md");
            let t = text(code);
            assert!(title(t).is_some(), "{code}");
            assert!(parse(t).len() > 20, "{code}");
        }
        assert_eq!(text("xx"), text("en"));
    }
}
