//! Ctrl+F / F7 in the viewer: the search bar, hits on the screen, F3 and
//! Shift+F3. The search itself runs in the background (rolid_view::search).

use std::sync::Arc;

use eframe::egui::{self, Key, RichText};
use rolid_i18n::tr;
use rolid_view::enc::Enc;
use rolid_view::search::{self, Exec, Method, Query, Search, Target};

use super::{Jump, TextView, BACK};

/// The search bar and its running search.
#[derive(Default)]
pub struct Find {
    pub text: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    focus: bool,
    search: Option<Search>,
    /// The hit shown as the current one.
    current: Option<(u64, u32)>,
    /// The first hit after a new search is shown by itself.
    show_first: bool,
    error: Option<String>,
    /// The query and encoding the running search is for.
    searched: Option<(Query, Enc)>,
}

impl Find {
    fn query(&self) -> Query {
        Query { text: self.text.clone(), case: self.case, word: self.word, regex: self.regex }
    }
}

impl TextView {
    /// Ctrl+F / F7: open the bar (with the selected text when it is short).
    pub fn open_find(&mut self) {
        let mut f = self.find.take().unwrap_or_default();
        if let Some((a, b)) = self.sel.map(|s| s.range()).filter(|(a, b)| b > a && b - a <= 200) {
            if let (Some(bytes), Some(enc)) = (self.doc.window(a, (b - a) as usize), self.enc) {
                let t = enc.decode(&bytes).into_owned();
                if !t.contains('\n') {
                    f.text = t;
                }
            }
        }
        f.focus = true;
        self.find = Some(f);
    }

    /// Esc: close the bar (the hits stay away too).
    pub fn close_find(&mut self) -> bool {
        let open = self.find.is_some();
        self.find = None;
        self.marks.clear();
        self.current_mark = None;
        open
    }

    /// F3 (`forward`) or Shift+F3: the next or previous hit.
    pub fn find_next(&mut self, forward: bool) {
        let Some(f) = &mut self.find else {
            self.open_find();
            return;
        };
        if f.text.is_empty() {
            f.focus = true;
            return;
        }
        let Some(s) = &f.search else { return };
        let found = s.found.lock().unwrap();
        let n = found.len();
        if n == 0 {
            return;
        }
        let i = match f.current {
            Some(c) => {
                let i = found.first_from(c.0);
                if forward {
                    if i + 1 < n {
                        i + 1
                    } else if found.done {
                        0
                    } else {
                        i
                    }
                } else if i > 0 {
                    i - 1
                } else if found.done {
                    n - 1
                } else {
                    0
                }
            }
            None => {
                let i = found.first_from(self.top);
                if forward {
                    if i < n { i } else { 0 }
                } else if i > 0 {
                    i - 1
                } else {
                    n - 1
                }
            }
        };
        let hit = found.get(i);
        drop(found);
        f.current = hit;
        f.show_first = false;
        if let Some(h) = hit {
            self.reveal(h.0);
        }
    }

    /// Show the hit at `off` (scroll only when it is not on the screen).
    fn reveal(&mut self, off: u64) {
        let on_screen = self.shown.len() > 1 && {
            let a = self.shown.first().and_then(|r| r.offs.first().copied()).unwrap_or(u64::MAX);
            // The last row may be cut by the window edge.
            let z = self.shown[self.shown.len() - 2].end;
            off >= a && off <= z
        };
        if !on_screen {
            self.jump(Jump::Offset(off));
            // A few rows above the hit stay in view. Wrapped, a row can be a long piece of a line
            // (its wrapped lines are known only when drawn): then one row up is enough.
            let up = (self.page / 3) as i64;
            let short_lines = || {
                let from = off.saturating_sub(4096).max(self.start);
                let enc = self.enc.unwrap_or(Enc::Utf8);
                self.doc.window(from, (off - from) as usize).is_some_and(|b| {
                    let mut n = 0;
                    let mut end = b.len();
                    while let Some(p) = enc.rfind_newline(&b, end) {
                        n += 1;
                        end = p;
                    }
                    n > up
                })
            };
            // No line longer than a row (as far as counted), or short lines before the hit.
            let no_long = self.doc.index_state().is_some_and(|i| i.longest < rolid_view::rows::SEG);
            self.pending = if !self.wrap || no_long || short_lines() { -up } else { -1 };
        }
        self.reveal_x = Some(off);
    }

    /// Restart the search when the query or the encoding changed.
    fn restart_search(&mut self) {
        let Some(enc) = self.enc else { return };
        let Some(f) = &mut self.find else { return };
        let q = f.query();
        if f.searched.as_ref().is_some_and(|(sq, se)| *sq == q && *se == enc) {
            return;
        }
        f.searched = Some((q.clone(), enc));
        f.search = None;
        f.current = None;
        f.error = None;
        if q.text.is_empty() {
            return;
        }
        let t = Target {
            fs: self.doc.fs().clone(),
            path: self.doc.path().to_string(),
            size: self.doc.size(),
            enc,
            start: self.start,
            from: self.top,
            exec: self.exec.clone(),
            chunk: self.doc.opts().block.max(256 << 10) * 4,
        };
        match search::start(&self.rt, t, q, self.repaint.clone()) {
            Ok(s) => {
                f.search = Some(s);
                f.show_first = true;
            }
            Err(e) => f.error = Some(e),
        }
    }

    /// The search bar (above the text).
    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui) {
        if self.find.is_none() {
            return;
        }
        self.restart_search();
        let remote = self.exec.is_some() || self.doc.fs().is_remote();
        let mut next: Option<bool> = None;
        let mut close = false;
        let mut cancel = false;
        let f = self.find.as_mut().unwrap();
        ui.horizontal(|ui| {
            ui.label(tr!("Find:"));
            let r = ui.add(egui::TextEdit::singleline(&mut f.text).desired_width(240.0).hint_text(tr!("text or regular expression")).id_salt("viewer-find"));
            if f.focus {
                r.request_focus();
                f.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                next = Some(!ui.input(|i| i.modifiers.shift));
                r.request_focus();
            }
            ui.toggle_value(&mut f.case, RichText::new("Aa").monospace()).on_hover_text(tr!("Match case"));
            ui.toggle_value(&mut f.word, RichText::new("ab|").monospace()).on_hover_text(tr!("Whole words"));
            ui.toggle_value(&mut f.regex, RichText::new(".*").monospace()).on_hover_text(tr!("Regular expression"));
            if ui.button("⬆").on_hover_text(tr!("Previous (Shift+F3)")).clicked() {
                next = Some(false);
            }
            if ui.button("⬇").on_hover_text(tr!("Next (F3)")).clicked() {
                next = Some(true);
            }
            if let Some(e) = &f.error {
                if !e.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, tr!("Not a valid regular expression.")).on_hover_text(e);
                }
            }
            if let Some(s) = &f.search {
                let found = s.found.lock().unwrap();
                let pos = f.current.map(|c| found.first_from(c.0) + 1);
                let count = found.count;
                let label = match (pos, found.done) {
                    (_, true) if count == 0 => tr!("Not found"),
                    (Some(p), true) => tr!("{n} of {count}", n = p, count = count),
                    (Some(p), false) => tr!("{n} of {count} so far", n = p, count = count),
                    (None, true) => tr!("{count} found", count = count),
                    (None, false) => tr!("{count} found so far", count = count),
                };
                ui.label(label);
                if found.capped {
                    ui.label(RichText::new("ⓘ").color(ui.visuals().warn_fg_color)).on_hover_text(tr!("Only the first {max} hits are marked; all are counted.", max = search::MAX_HITS)); // i18n-ok: an icon
                }
                if !found.done {
                    ui.spinner();
                    if let Some(sc) = found.scanned {
                        let p = if found.total == 0 { 100.0 } else { 100.0 * sc as f64 / found.total as f64 };
                        ui.label(format!("{p:.0} %")); // i18n-ok: a number
                    }
                    if ui.button(tr!("Cancel")).clicked() {
                        cancel = true;
                    }
                } else if let Some(e) = &found.error {
                    ui.colored_label(ui.visuals().error_fg_color, tr!("Search failed: {error}", error = rolid_i18n::t(e)));
                }
                if remote {
                    match found.method {
                        Some(Method::Grep) => {
                            ui.label(RichText::new(tr!("grep on the server")).weak()).on_hover_text(tr!("The server searched the file itself; nothing was downloaded."));
                        }
                        Some(Method::Read) => {
                            ui.label(RichText::new(tr!("read over SFTP")).weak()).on_hover_text(tr!("The server has no usable grep for this search, so the file is read here."));
                        }
                        None => {}
                    }
                }
            }
            if ui.button("✕").on_hover_text(tr!("Close (Esc)")).clicked() {
                close = true;
            }
        });
        if cancel {
            if let Some(s) = &f.search {
                s.cancel();
            }
        }
        if close {
            self.close_find();
            return;
        }
        if let Some(fw) = next {
            self.find_next(fw);
        }
        // The first hit from the shown place on comes by itself.
        let f = self.find.as_mut().unwrap();
        if f.show_first {
            if let Some(s) = &f.search {
                let found = s.found.lock().unwrap();
                let i = found.first_from(found.from);
                let hit = found.get(i).filter(|h| h.0 >= found.from).or_else(|| if found.done { found.get(0) } else { None });
                drop(found);
                if let Some(h) = hit {
                    f.current = Some(h);
                    f.show_first = false;
                    self.reveal(h.0);
                }
            }
        }
    }

    /// Hits in the bytes around the top row, for drawing.
    pub(super) fn update_marks(&mut self, ahead: u64) {
        self.marks.clear();
        self.current_mark = None;
        let Some(f) = &self.find else { return };
        let Some(s) = &f.search else { return };
        let found = s.found.lock().unwrap();
        let a = self.top.saturating_sub(BACK);
        self.marks = found.within(a, self.top + ahead);
        self.current_mark = f.current.map(|(o, l)| (o, o + l as u64));
    }

    /// Is a search running (the window repaints for its progress)?
    pub fn searching(&self) -> bool {
        self.find.as_ref().and_then(|f| f.search.as_ref()).is_some_and(|s| !s.found.lock().unwrap().done)
    }

    pub fn set_exec(&mut self, e: Option<Arc<dyn Exec>>) {
        self.exec = e;
    }
}
