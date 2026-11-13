//! The text and hex view of the viewer: only the rows on the screen are
//! laid out, from bytes the document has cached (it never waits for them:
//! while a block is read the last picture stays).

use std::sync::{Arc, Mutex};

use eframe::egui::{self, text::LayoutJob, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, TextFormat, Vec2};
use rolid_i18n::tr;
use rolid_view::doc::{Doc, LineSeek};
use rolid_view::enc::Enc;
use rolid_view::hex;
use rolid_view::index::LineNo;
use rolid_view::rows::{Next, Window, BACK};
use rolid_view::syntax::{self, Highlighter, Off, Span};

mod find;
pub use find::Find;

/// Spaces a tab stands for.
const TAB: usize = 4;
/// Most bytes copied to the clipboard.
pub const COPY_MAX: u64 = 32 << 20;
/// Bytes before the top row kept in the window (scrolling up, line starts).
const BEHIND: u64 = 64 << 10;

/// A jump waiting for its data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Jump {
    /// The row containing this offset.
    Offset(u64),
    /// The last screen.
    End,
    Line(u64),
    /// A place in the file (the scroll bar), 0 – 1.
    Fraction(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sel {
    pub anchor: u64,
    pub head: u64,
}

impl Sel {
    pub fn range(&self) -> (u64, u64) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
}

/// A row as it was drawn (for the mouse).
struct Shown {
    rect: Rect,
    /// Where the galley is drawn.
    origin: Pos2,
    galley: Arc<egui::Galley>,
    /// File offset of every character of the galley.
    offs: Vec<u64>,
    /// Offset after the row (without its line end).
    end: u64,
    /// Hex: the first byte and where the hex and character parts start.
    hex: Option<(u64, f32, f32)>,
}

/// What the view tells the window.
#[derive(Default)]
pub struct Info {
    pub line: Option<LineNo>,
    pub total: Option<LineNo>,
    pub offset: u64,
    pub percent: f64,
    pub indexing: Option<f64>,
    /// Lines are counted only in the start of the file (a server).
    pub limited: bool,
}

pub struct TextView {
    pub doc: Doc,
    name: String,
    /// The encoding shown (`None` until the start is read).
    pub enc: Option<Enc>,
    /// Where the text starts (after a BOM).
    pub start: u64,
    pub hex: bool,
    /// The start of the file looks binary.
    pub binary: bool,
    top: u64,
    /// Wrapped lines of the top row scrolled away.
    top_sub: usize,
    jump: Option<Jump>,
    /// Rows still to scroll (waiting for data).
    pending: i64,
    pub wrap: bool,
    pub numbers: bool,
    pub font: f32,
    hscroll: f32,
    widest: f32,
    pub sel: Option<Sel>,
    pub follow: bool,
    shown: Vec<Shown>,
    /// The language highlighted (`None` = off) and whether the user chose it.
    pub syntax: Option<String>,
    pub syntax_chosen: bool,
    pub hl: Option<Highlighter>,
    hl_dark: bool,
    pub hl_limit: u64,
    pub hl_disabled: bool,
    /// Ctrl+G: the text typed.
    pub goto: Option<String>,
    goto_focus: bool,
    /// A copy made in the background: the text (or an error) for the clipboard.
    clip: Arc<Mutex<Option<Result<String, String>>>>,
    pub copying: bool,
    last_size: u64,
    /// Whole rows on the screen.
    page: usize,
    dragging: bool,
    rt: tokio::runtime::Handle,
    repaint: Arc<dyn Fn() + Send + Sync>,
    pub info: Info,
    /// Search hits on the screen: file ranges (from the search, see find.rs).
    pub marks: Vec<(u64, u64)>,
    pub current_mark: Option<(u64, u64)>,
    /// Ctrl+F: the search bar.
    pub find: Option<Find>,
    /// The server's shell for searching (grep), if any.
    exec: Option<Arc<dyn rolid_view::search::Exec>>,
    /// Scroll sideways to show this offset (a hit).
    reveal_x: Option<u64>,
}

impl TextView {
    pub fn new(doc: Doc, name: String, rt: tokio::runtime::Handle, repaint: Arc<dyn Fn() + Send + Sync>, s: &rolid_config::Settings) -> Self {
        TextView {
            doc,
            name,
            enc: None,
            start: 0,
            hex: false,
            binary: false,
            top: 0,
            top_sub: 0,
            jump: None,
            pending: 0,
            wrap: !s.viewer_nowrap,
            numbers: !s.viewer_numbers_off,
            font: s.viewer_font_size(),
            hscroll: 0.0,
            widest: 0.0,
            sel: None,
            follow: false,
            shown: Vec::new(),
            syntax: None,
            syntax_chosen: false,
            hl: None,
            hl_dark: false,
            hl_limit: s.viewer_highlight_max(),
            hl_disabled: s.viewer_highlight_off,
            goto: None,
            goto_focus: false,
            clip: Arc::default(),
            copying: false,
            last_size: 0,
            page: 20,
            dragging: false,
            rt,
            repaint,
            info: Info::default(),
            marks: Vec::new(),
            current_mark: None,
            find: None,
            exec: None,
            reveal_x: None,
        }
    }

    /// The first data came: encoding, binary, language.
    fn init(&mut self) {
        if self.enc.is_some() {
            return;
        }
        let Some(d) = self.doc.detected() else { return };
        self.enc = Some(d.enc);
        self.start = d.bom as u64;
        self.binary = d.binary;
        self.hex = d.binary;
        self.top = self.start;
        if !d.binary {
            let first = self.doc.window(self.start, 1024).unwrap_or_default();
            let first = d.enc.decode(&first);
            let line = first.lines().next().unwrap_or("");
            self.syntax = syntax::guess(&self.name, line);
        }
    }

    pub fn set_enc(&mut self, enc: Enc) {
        let Some(d) = self.doc.detected() else { return };
        self.enc = Some(enc);
        // The BOM counts only for its own encoding.
        self.start = if d.enc == enc { d.bom as u64 } else { 0 };
        self.doc.set_index_enc(enc, self.start);
        self.hl = None;
        let t = self.top.max(self.start);
        self.top = t - (t - self.start) % enc.unit() as u64;
        self.jump = Some(Jump::Offset(self.top));
    }

    pub fn set_syntax(&mut self, s: Option<String>) {
        self.syntax = s;
        self.syntax_chosen = true;
        self.hl = None;
    }

    /// Why highlighting is off now (`None` = on, or no language).
    pub fn hl_off(&self) -> Option<Off> {
        self.hl.as_ref().and_then(|h| h.off())
    }

    pub fn jump(&mut self, j: Jump) {
        self.jump = Some(j);
        self.pending = 0;
        self.top_sub = 0;
        if j != Jump::End {
            self.follow = false;
            self.doc.set_follow(false);
        }
    }

    pub fn set_follow(&mut self, on: bool) {
        self.follow = on;
        self.doc.set_follow(on);
        if on {
            self.jump(Jump::End);
        }
    }

    /// The runtime and the repaint of this view (for the image mode).
    pub fn runtime(&self) -> (tokio::runtime::Handle, Arc<dyn Fn() + Send + Sync>) {
        (self.rt.clone(), self.repaint.clone())
    }

    pub fn top(&self) -> u64 {
        self.top
    }

    /// Start (or restart) highlighting when it is wanted and possible.
    fn ensure_highlighter(&mut self, dark: bool) {
        let Some(enc) = self.enc else { return };
        if self.hex || self.hl_disabled || self.syntax.is_none() {
            self.hl = None;
            return;
        }
        if self.hl.is_some() && self.hl_dark == dark && self.hl.as_ref().map(|h| &h.syntax) == self.syntax.as_ref() {
            return;
        }
        let src = syntax::Source { fs: self.doc.fs().clone(), path: self.doc.path().to_string(), size: self.doc.size(), enc, start: self.start };
        self.hl_dark = dark;
        self.hl = Some(Highlighter::new(self.rt.clone(), src, self.syntax.clone().unwrap_or_default(), dark, self.hl_limit, self.repaint.clone()));
    }

    pub fn reload(&mut self) {
        self.doc.reload();
        self.hl = None;
        self.jump = Some(Jump::Offset(self.top));
    }

    // ------------------------------------------------------------ keys

    /// Keys of the text area (the window took its own already).
    pub fn keys(&mut self, ctx: &egui::Context) {
        if self.goto.is_some() {
            return;
        }
        let page = self.page.max(2) as i64 - 1;
        let mut scroll = 0i64;
        let mut sel_move: Option<i64> = None;
        ctx.input_mut(|i| {
            let ctrl = Modifiers::COMMAND;
            if i.consume_key(ctrl, Key::Home) || i.consume_key(Modifiers::NONE, Key::Home) {
                self.jump(Jump::Offset(0));
            }
            if i.consume_key(ctrl, Key::End) || i.consume_key(Modifiers::NONE, Key::End) {
                self.jump(Jump::End);
            }
            if i.consume_key(Modifiers::SHIFT, Key::ArrowDown) {
                sel_move = Some(1);
            }
            if i.consume_key(Modifiers::SHIFT, Key::ArrowUp) {
                sel_move = Some(-1);
            }
            if i.consume_key(Modifiers::SHIFT, Key::PageDown) {
                sel_move = Some(page);
            }
            if i.consume_key(Modifiers::SHIFT, Key::PageUp) {
                sel_move = Some(-page);
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                scroll += 1;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                scroll -= 1;
            }
            if i.consume_key(Modifiers::NONE, Key::PageDown) || i.consume_key(Modifiers::NONE, Key::Space) {
                scroll += page;
            }
            if i.consume_key(Modifiers::NONE, Key::PageUp) {
                scroll -= page;
            }
            if !self.wrap && !self.hex {
                if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                    self.hscroll += 40.0;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                    self.hscroll = (self.hscroll - 40.0).max(0.0);
                }
            }
            if i.consume_key(ctrl, Key::A) {
                self.sel = Some(Sel { anchor: self.start, head: self.doc.size() });
            }
            // Ctrl+C comes as a copy event (not as a key).
            let copy_event = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            if copy_event || i.consume_key(ctrl, Key::C) || i.consume_key(ctrl, Key::Insert) {
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
                self.copy();
            }
            if i.consume_key(ctrl, Key::G) {
                self.goto = Some(String::new());
                self.goto_focus = true;
            }
            if i.consume_key(ctrl, Key::Plus) || i.consume_key(ctrl, Key::Equals) {
                self.font = (self.font + 1.0).min(32.0);
            }
            if i.consume_key(ctrl, Key::Minus) {
                self.font = (self.font - 1.0).max(8.0);
            }
            if i.consume_key(ctrl, Key::Num0) {
                self.font = 13.0;
            }
        });
        if let Some(d) = sel_move {
            self.extend_selection(d);
        }
        if scroll != 0 {
            self.scroll(scroll);
        }
    }

    /// Scroll by `n` rows (negative = up); stops following when going up.
    pub fn scroll(&mut self, n: i64) {
        if n < 0 && self.follow {
            self.follow = false;
            self.doc.set_follow(false);
        }
        self.pending += n;
    }

    /// Move the end of the selection by `d` rows (starting at the top row).
    fn extend_selection(&mut self, d: i64) {
        let head = self.sel.map_or(self.top, |s| s.head);
        // Find the row of the head among the shown rows.
        let i = self.shown.iter().position(|r| r.offs.first().is_some_and(|&o| o >= head) || r.end >= head).unwrap_or(0) as i64;
        let j = i + d;
        let target = if j < 0 {
            self.scroll(j);
            self.shown.first().and_then(|r| r.offs.first().copied()).unwrap_or(self.top)
        } else if j as usize >= self.shown.len() {
            self.scroll(j - self.shown.len() as i64 + 1);
            self.shown.last().map_or(head, |r| r.end)
        } else {
            let r = &self.shown[j as usize];
            r.offs.first().copied().unwrap_or(r.end)
        };
        let anchor = self.sel.map_or(head, |s| s.anchor);
        self.sel = Some(Sel { anchor, head: target });
    }

    /// Copy the selection to the clipboard (in the background when it is not
    /// all on hand); at most [`COPY_MAX`] bytes.
    pub fn copy(&mut self) {
        let Some(sel) = self.sel else { return };
        let (a, b) = sel.range();
        if a == b {
            return;
        }
        let b = b.min(a + COPY_MAX);
        let enc = self.enc.unwrap_or(Enc::Utf8);
        let hex = self.hex;
        let render = move |bytes: &[u8]| -> String {
            if hex {
                bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
            } else {
                enc.decode(bytes).into_owned()
            }
        };
        if let Some(bytes) = self.doc.window(a, (b - a) as usize).filter(|w| w.len() as u64 == b - a) {
            *self.clip.lock().unwrap() = Some(Ok(render(&bytes)));
            return;
        }
        let (fs, path) = (self.doc.fs().clone(), self.doc.path().to_string());
        let clip = self.clip.clone();
        let repaint = self.repaint.clone();
        self.copying = true;
        self.rt.spawn(async move {
            let r = async {
                let mut f = rolid_core::vfs::open_random(&fs, &path).await?;
                f.read_at(a, (b - a) as usize).await
            }
            .await;
            *clip.lock().unwrap() = Some(r.map(|d| render(&d)).map_err(|e| rolid_i18n::t(&e.text())));
            repaint();
        });
    }

    /// A finished copy for the clipboard.
    pub fn take_clip(&mut self) -> Option<Result<String, String>> {
        let r = self.clip.lock().unwrap().take();
        if r.is_some() {
            self.copying = false;
        }
        r
    }

    // ------------------------------------------------------------ drawing

    /// The text area (fills `ui`).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.init();
        let dark = ui.visuals().dark_mode;
        self.ensure_highlighter(dark);
        let size = self.doc.size();
        if self.follow && size != self.last_size {
            self.jump = Some(Jump::End);
            self.pending = 0;
        }
        self.last_size = size;
        self.find_bar(ui);
        if let Some(text) = self.goto.clone() {
            self.goto_bar(ui, text);
        }
        let full = ui.available_rect_before_wrap();
        let bar_w = 14.0;
        let area = Rect::from_min_max(full.min, Pos2::new(full.max.x - bar_w, full.max.y));
        let bar = Rect::from_min_max(Pos2::new(full.max.x - bar_w, full.min.y), full.max);
        let resp = ui.interact(area, ui.scope_id().with("text"), Sense::click_and_drag());
        let font = FontId::monospace(self.font);
        let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
        self.page = ((area.height() / row_h).floor() as usize).max(1);
        let rows_wanted = self.page + 2;

        let Some(enc) = self.enc else {
            ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, tr!("Loading…"), egui::FontId::proportional(crate::fonts::fs(14.0)), ui.visuals().weak_text_color());
            ui.allocate_rect(full, Sense::hover());
            return;
        };
        if self.hex {
            self.hex_ui(ui, area, &resp, &font, row_h, char_w, rows_wanted);
        } else {
            self.text_ui(ui, area, &resp, &font, row_h, char_w, rows_wanted, enc, dark);
        }
        self.mouse(ui, &resp, area, row_h);
        self.scroll_bar(ui, bar, size);
        ui.allocate_rect(full, Sense::hover());
    }

    fn goto_bar(&mut self, ui: &mut egui::Ui, mut text: String) {
        let mut close = false;
        let mut go = None;
        ui.horizontal(|ui| {
            ui.label(tr!("Go to line:"));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(160.0).hint_text(tr!("line, 50 % or 0x1F00")));
            if self.goto_focus {
                r.request_focus();
                self.goto_focus = false;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if enter || ui.button(tr!("Go")).clicked() {
                go = parse_goto(&text);
                close = go.is_some();
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) || ui.button(tr!("Cancel")).clicked() {
                close = true;
            }
            if !text.trim().is_empty() && parse_goto(&text).is_none() {
                ui.colored_label(ui.visuals().error_fg_color, tr!("Enter a line number, a percentage or an offset."));
            }
        });
        if let Some(j) = go {
            self.jump(j);
        }
        self.goto = if close { None } else { Some(text) };
    }

    /// Bytes around the top row: `(base, bytes)`.
    fn window(&self, ahead: u64) -> (u64, Vec<u8>) {
        let unit = self.enc.map_or(1, |e| e.unit()) as u64;
        let mut from = self.top.saturating_sub(BEHIND.max(BACK)).max(self.start);
        from -= (from - self.start) % unit;
        let len = (self.top - from) + ahead;
        (from, self.doc.window_partial(from, len as usize))
    }

    /// Resolve a waiting jump and pending scrolling with the data on hand.
    fn settle(&mut self, enc: Enc, rows_wanted: usize) {
        let size = self.doc.size();
        if let Some(j) = self.jump {
            let target = match j {
                Jump::Offset(o) => Some(o.min(size)),
                Jump::Fraction(f) => Some(((size as f64) * f.clamp(0.0, 1.0)) as u64),
                Jump::End => Some(size),
                Jump::Line(n) => match self.doc.seek_line(n) {
                    // Past the last line: the last screen.
                    LineSeek::Found(o) if o >= size => {
                        self.jump = Some(Jump::End);
                        return self.settle(enc, rows_wanted);
                    }
                    LineSeek::Found(o) | LineSeek::About(o) => Some(o),
                    LineSeek::Pending => None,
                },
            };
            if let Some(t) = target {
                let t = t.max(self.start);
                let mut from = t.saturating_sub(BACK).max(self.start);
                from -= (from - self.start) % enc.unit() as u64;
                if let Some(data) = self.doc.window(from, (t - from + 8) as usize) {
                    let w = Window { base: from, data: &data, size, start: self.start, enc };
                    if let Some(r) = w.row_start(t) {
                        self.top = r;
                        self.top_sub = 0;
                        self.jump = None;
                        if j == Jump::End {
                            // The last screen: up from the end; an empty row after a final line end is not shown.
                            self.pending = -(rows_wanted as i64 - 3).max(0) - i64::from(r == size && size > self.start);
                        }
                    }
                }
            }
        }
        if self.jump.is_some() || self.pending == 0 {
            return;
        }
        let (base, data) = self.window((self.pending.max(0) as u64 + 2) * 3 * rolid_view::rows::SEG + 4096);
        let w = Window { base, data: &data, size, start: self.start, enc };
        while self.pending > 0 {
            if self.wrap && self.top_sub != usize::MAX {
                // Wrapped rows scroll by their lines (counted when drawn).
                let lines = self.shown.first().map_or(1, |r| r.galley.rows.len());
                if self.top_sub + 1 < lines {
                    self.top_sub += 1;
                    self.pending -= 1;
                    continue;
                }
            }
            match w.next_row(self.top) {
                Next::Row(n) => {
                    self.top = n;
                    self.top_sub = 0;
                    self.pending -= 1;
                    // The view must not run past the end.
                }
                Next::End => self.pending = 0,
                Next::NeedData => break,
            }
            // Only the first scrolled row uses the drawn row's lines.
            self.shown.clear();
        }
        while self.pending < 0 {
            if self.wrap && self.top_sub > 0 && self.top_sub != usize::MAX {
                self.top_sub -= 1;
                self.pending += 1;
                continue;
            }
            match w.prev_row(self.top) {
                Some(p) => {
                    self.top = p;
                    self.top_sub = if self.wrap { usize::MAX } else { 0 };
                    self.pending += 1;
                }
                None if self.top <= self.start => {
                    self.pending = 0;
                    self.top_sub = 0;
                }
                None => break,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_ui(&mut self, ui: &mut egui::Ui, area: Rect, resp: &egui::Response, font: &FontId, row_h: f32, char_w: f32, rows_wanted: usize, enc: Enc, dark: bool) {
        let _ = resp;
        self.settle(enc, rows_wanted);
        let size = self.doc.size();
        let ahead = rows_wanted as u64 * 3 * rolid_view::rows::SEG + 8192;
        self.update_marks(ahead);
        let (base, data) = self.window(ahead);
        let w = Window { base, data: &data, size, start: self.start, enc };
        // Rows on the screen.
        let mut rows: Vec<(u64, &[u8], bool, u64)> = Vec::new(); // start, bytes, line start, next
        let mut r = self.top;
        let mut complete = false;
        while rows.len() < rows_wanted {
            let Some(line_start) = w.is_line_start(r) else { break };
            match w.row(r) {
                Some((bytes, next)) => {
                    let n = match next {
                        Next::Row(n) => n,
                        _ => size,
                    };
                    rows.push((r, bytes, line_start, n));
                    match next {
                        Next::Row(n) => r = n,
                        _ => {
                            complete = true;
                            break;
                        }
                    }
                }
                None => break,
            }
        }
        if rows.is_empty() && !complete && self.top < size {
            // Data on its way: keep the last picture.
            self.paint_shown(ui, area);
            return;
        }
        // Line numbers.
        let first_line = self.doc.line_at(self.top);
        let total = self.doc.index_state().map(|i| i.total);
        let gutter = if self.numbers {
            let digits = total.map_or(6, |t| t.get().max(first_line.get() + rows.len() as u64).to_string().len()).max(4) + 1;
            digits as f32 * char_w + 12.0
        } else {
            4.0
        };
        // Highlighting: the lines on the screen.
        let line_of_top = if w.is_line_start(self.top) == Some(true) {
            Some(self.top)
        } else {
            enc.rfind_newline(&data[..(self.top - base) as usize], (self.top - base) as usize).map(|p| base + (p + enc.unit()) as u64)
        };
        if let (Some(h), Some(l0)) = (&self.hl, line_of_top) {
            h.show(l0, rows.last().map_or(l0, |r| r.3));
        }
        let text_x = area.left() + gutter;
        let wrap_w = (area.right() - text_x - 4.0).max(50.0);
        let painter = ui.painter_at(area);
        let visuals = ui.visuals().clone();
        let fg = visuals.text_color();
        let sel_bg = visuals.selection.bg_fill;
        let mark_bg = if dark { Color32::from_rgb(0x6b, 0x55, 0x00) } else { Color32::from_rgb(0xff, 0xe8, 0x7a) };
        let cur_bg = if dark { Color32::from_rgb(0xb0, 0x60, 0x00) } else { Color32::from_rgb(0xff, 0xa5, 0x3a) };
        let sel = self.sel.map(|s| s.range());
        let mut y = area.top();
        let mut line_no = first_line;
        let mut line_start_of_row = line_of_top;
        self.shown.clear();
        let mut widest = 0.0f32;
        for (i, &(start, bytes, is_line_start, next)) in rows.iter().enumerate() {
            if y > area.bottom() {
                break;
            }
            if is_line_start {
                line_start_of_row = Some(start);
                if i > 0 {
                    line_no = match line_no {
                        LineNo::Exact(n) => LineNo::Exact(n + 1),
                        LineNo::About(n) => LineNo::About(n + 1),
                    };
                }
            }
            let (text, src) = enc.decode_mapped(bytes);
            let spans = match (&self.hl, line_start_of_row) {
                (Some(h), Some(l)) => h.spans(l).map(|(s, _)| (s, l)),
                _ => None,
            };
            let (job, offs) = layout_row(&text, &src, start, bytes.len(), font, fg, spans.as_ref().map(|(s, l)| (s.as_slice(), *l)), sel, &self.marks, self.current_mark, sel_bg, mark_bg, cur_bg, if self.wrap { wrap_w } else { f32::INFINITY });
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            widest = widest.max(galley.size().x);
            let lines = galley.rows.len().max(1);
            let skip = if i == 0 && self.wrap {
                if self.top_sub == usize::MAX {
                    self.top_sub = lines - 1;
                }
                self.top_sub = self.top_sub.min(lines - 1);
                self.top_sub
            } else {
                0
            };
            let origin = Pos2::new(text_x - if self.wrap { 0.0 } else { self.hscroll }, y - skip as f32 * row_h);
            let h = galley.size().y.max(row_h) - skip as f32 * row_h;
            let rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(area.width(), h));
            painter.galley(origin, galley.clone(), fg);
            if self.numbers && is_line_start && skip == 0 {
                let label = match line_no {
                    LineNo::Exact(n) => n.to_string(),
                    LineNo::About(n) => format!("~{n}"),
                };
                painter.text(Pos2::new(text_x - 8.0, y), egui::Align2::RIGHT_TOP, label, font.clone(), visuals.weak_text_color());
            } else if self.numbers && !is_line_start && skip == 0 {
                // A continuation of a cut line.
                painter.text(Pos2::new(text_x - 8.0, y), egui::Align2::RIGHT_TOP, "↪", font.clone(), visuals.weak_text_color());
            }
            let end = start + bytes.len() as u64;
            let _ = next;
            self.shown.push(Shown { rect, origin, galley, offs, end, hex: None });
            y += h;
        }
        if self.numbers {
            painter.vline(text_x - 4.0, area.y_range(), egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color));
        }
        self.widest = widest;
        if !self.wrap {
            self.hscroll = self.hscroll.min((widest - wrap_w + 3.0 * char_w).max(0.0));
            // A hit off the side of the screen: scroll to it.
            if let Some(off) = self.reveal_x {
                if let Some(r) = self.shown.iter().find(|r| r.offs.first().is_some_and(|&a| a <= off) && off <= r.end) {
                    let k = r.offs.partition_point(|&o| o < off);
                    let x = r.galley.pos_from_cursor(egui::text::CCursor::new(k)).min.x;
                    if x < self.hscroll || x > self.hscroll + wrap_w - 4.0 * char_w {
                        self.hscroll = (x - wrap_w / 3.0).max(0.0);
                    }
                    self.reveal_x = None;
                }
            }
        } else {
            self.reveal_x = None;
        }
        // Status.
        self.info.line = Some(first_line);
        self.info.total = total;
        self.info.offset = self.top;
        self.info.percent = if size == 0 { 100.0 } else { 100.0 * (rows.last().map_or(self.top, |r| r.3) as f64 / size as f64).min(1.0) };
        self.info.limited = self.doc.index_state().is_some_and(|i| i.limited);
        self.info.indexing = self.doc.index_state().filter(|i| !i.done && !i.limited && i.scanned < size).map(|i| if size == 0 { 1.0 } else { i.scanned as f64 / size as f64 });
    }

    fn paint_shown(&self, ui: &egui::Ui, area: Rect) {
        let painter = ui.painter_at(area);
        let fg = ui.visuals().text_color();
        for r in &self.shown {
            painter.galley(r.origin, r.galley.clone(), fg);
        }
        painter.text(area.right_top() + Vec2::new(-8.0, 4.0), egui::Align2::RIGHT_TOP, tr!("Loading…"), egui::FontId::proportional(crate::fonts::fs(12.0)), ui.visuals().weak_text_color());
    }

    #[allow(clippy::too_many_arguments)]
    fn hex_ui(&mut self, ui: &mut egui::Ui, area: Rect, resp: &egui::Response, font: &FontId, row_h: f32, char_w: f32, rows_wanted: usize) {
        let _ = resp;
        let size = self.doc.size();
        let width = hex::WIDTH as u64;
        // Jumps and scrolling are plain arithmetic here.
        if let Some(j) = self.jump.take() {
            self.top = match j {
                Jump::Offset(o) => o.min(size),
                Jump::Fraction(f) => ((size as f64) * f.clamp(0.0, 1.0)) as u64,
                Jump::End => size.saturating_sub(((rows_wanted as u64).saturating_sub(2)) * width),
                Jump::Line(_) => self.top,
            } / width
                * width;
        }
        if self.pending != 0 {
            let max_top = size.saturating_sub(1) / width * width;
            let t = self.top as i64 + self.pending * width as i64;
            self.top = (t.max(0) as u64).min(max_top);
            self.pending = 0;
        }
        let len = rows_wanted as u64 * width;
        self.update_marks(len);
        self.reveal_x = None;
        let data = self.doc.window_partial(self.top, len as usize);
        if data.is_empty() && self.top < size {
            self.paint_shown(ui, area);
            return;
        }
        let digits = hex::offset_digits(size);
        let painter = ui.painter_at(area);
        let visuals = ui.visuals().clone();
        let fg = visuals.text_color();
        let weak = visuals.weak_text_color();
        let sel = self.sel.map(|s| s.range());
        let sel_bg = visuals.selection.bg_fill;
        let mark_bg = if visuals.dark_mode { Color32::from_rgb(0x6b, 0x55, 0x00) } else { Color32::from_rgb(0xff, 0xe8, 0x7a) };
        let cur_bg = if visuals.dark_mode { Color32::from_rgb(0xb0, 0x60, 0x00) } else { Color32::from_rgb(0xff, 0xa5, 0x3a) };
        let hex_x = area.left() + (digits + 2) as f32 * char_w;
        let chars_x = hex_x + (hex::WIDTH * 3 + 2) as f32 * char_w;
        self.shown.clear();
        let mut y = area.top();
        for (i, chunk) in data.chunks(hex::WIDTH).enumerate() {
            if y > area.bottom() {
                break;
            }
            let off = self.top + i as u64 * width;
            let (pos, hx, chars) = hex::row(off, chunk, digits);
            painter.text(Pos2::new(area.left() + 4.0, y), egui::Align2::LEFT_TOP, pos, font.clone(), weak);
            // Selection and hits behind the bytes.
            for (j, _) in chunk.iter().enumerate() {
                let b = off + j as u64;
                let bg = if sel.is_some_and(|(a, z)| b >= a && b < z) {
                    Some(sel_bg)
                } else if self.current_mark.is_some_and(|(a, z)| b >= a && b < z) {
                    Some(cur_bg)
                } else if self.marks.iter().any(|&(a, z)| b >= a && b < z) {
                    Some(mark_bg)
                } else {
                    None
                };
                if let Some(bg) = bg {
                    let x = hex_x + hex::column_of_byte(j) as f32 * char_w;
                    painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0 * char_w, row_h)), 0.0, bg);
                    painter.rect_filled(Rect::from_min_size(Pos2::new(chars_x + j as f32 * char_w, y), Vec2::new(char_w, row_h)), 0.0, bg);
                }
            }
            painter.text(Pos2::new(hex_x, y), egui::Align2::LEFT_TOP, hx, font.clone(), fg);
            let galley = ui.fonts_mut(|f| f.layout_no_wrap(chars, font.clone(), fg));
            painter.galley(Pos2::new(chars_x, y), galley.clone(), fg);
            self.shown.push(Shown {
                rect: Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(area.width(), row_h)),
                origin: Pos2::new(chars_x, y),
                galley,
                offs: (0..chunk.len() as u64).map(|k| off + k).collect(),
                end: off + chunk.len() as u64,
                hex: Some((off, hex_x, chars_x)),
            });
            y += row_h;
        }
        self.info.line = None;
        self.info.total = None;
        self.info.offset = self.top;
        self.info.percent = if size == 0 { 100.0 } else { 100.0 * ((self.top + data.len() as u64) as f64 / size as f64).min(1.0) };
        self.info.indexing = None;
    }

    /// The file offset under the pointer.
    fn offset_at(&self, pos: Pos2, char_w: f32) -> Option<u64> {
        let row = self.shown.iter().find(|r| pos.y < r.rect.bottom()).or(self.shown.last())?;
        if let Some((off, hex_x, chars_x)) = row.hex {
            let n = row.offs.len() as u64;
            let col = if pos.x >= chars_x {
                ((pos.x - chars_x) / char_w) as u64
            } else {
                let c = ((pos.x - hex_x).max(0.0) / char_w) as usize;
                hex::byte_at_column(c).or_else(|| hex::byte_at_column(c.saturating_sub(1))).unwrap_or(0) as u64
            };
            return Some(off + col.min(n));
        }
        let c = row.galley.cursor_from_pos(pos - row.origin);
        Some(row.offs.get(c.index.0).copied().unwrap_or(row.end))
    }

    fn mouse(&mut self, ui: &egui::Ui, resp: &egui::Response, area: Rect, row_h: f32) {
        let font = FontId::monospace(self.font);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        // Wheel: rows (Ctrl = text size; Shift = sideways).
        if resp.hovered() || resp.contains_pointer() {
            let (dy, dx, ctrl) = ui.input(|i| (i.smooth_scroll_delta.y, i.smooth_scroll_delta.x, i.modifiers.command));
            if ctrl && dy != 0.0 {
                let z = ui.input(|i| i.zoom_delta());
                if z != 1.0 {
                    self.font = (self.font * z).clamp(8.0, 32.0);
                }
            } else if dy != 0.0 {
                let acc_id = ui.scope_id().with("wheel");
                let mut acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0);
                acc += dy;
                let n = (acc / row_h).trunc();
                if n != 0.0 {
                    acc -= n * row_h;
                    self.scroll(-(n as i64));
                }
                ui.data_mut(|d| d.insert_temp(acc_id, acc));
            }
            if dx != 0.0 && !self.wrap {
                self.hscroll = (self.hscroll - dx).clamp(0.0, (self.widest - 40.0).max(0.0));
            }
        }
        // Selection by dragging (a click clears it).
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.drag_started() || (resp.is_pointer_button_down_on() && !self.dragging) {
                if let Some(o) = self.offset_at(p, char_w) {
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.sel = match (shift, self.sel) {
                        (true, Some(s)) => Some(Sel { anchor: s.anchor, head: o }),
                        _ => Some(Sel { anchor: o, head: o }),
                    };
                    self.dragging = true;
                }
            } else if self.dragging {
                if let Some(o) = self.offset_at(p, char_w) {
                    if let Some(s) = &mut self.sel {
                        s.head = o;
                    }
                }
                // Beyond the edges: scroll on.
                if p.y > area.bottom() - 2.0 {
                    self.scroll(1);
                    ui.ctx().request_repaint();
                } else if p.y < area.top() + 2.0 {
                    self.scroll(-1);
                    ui.ctx().request_repaint();
                }
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            if self.dragging && self.sel.is_some_and(|s| s.anchor == s.head) {
                self.sel = None;
            }
            self.dragging = false;
        }
        if resp.double_clicked() {
            self.select_word(ui, char_w);
        }
    }

    /// Double click: the word under the pointer.
    fn select_word(&mut self, ui: &egui::Ui, char_w: f32) {
        let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return };
        let Some(row) = self.shown.iter().find(|r| p.y < r.rect.bottom()) else { return };
        if row.hex.is_some() {
            return;
        }
        let c = row.galley.cursor_from_pos(p - row.origin).index.0;
        let text: Vec<char> = row.galley.job.text.chars().collect();
        let word = |ch: &char| ch.is_alphanumeric() || *ch == '_';
        if !text.get(c).is_some_and(word) {
            return;
        }
        let mut a = c;
        while a > 0 && word(&text[a - 1]) {
            a -= 1;
        }
        let mut z = c;
        while z < text.len() && word(&text[z]) {
            z += 1;
        }
        let off = |i: usize| row.offs.get(i).copied().unwrap_or(row.end);
        self.sel = Some(Sel { anchor: off(a), head: off(z) });
        let _ = char_w;
    }

    fn scroll_bar(&mut self, ui: &egui::Ui, bar: Rect, size: u64) {
        let resp = ui.interact(bar, ui.scope_id().with("vbar"), Sense::click_and_drag());
        let v = ui.visuals();
        ui.painter().rect_filled(bar, 0.0, v.extreme_bg_color);
        if size == 0 {
            return;
        }
        let shown = self.shown.last().map_or(0, |r| r.end).saturating_sub(self.top).max(1);
        let frac_len = (shown as f32 / size as f32).clamp(0.02, 1.0);
        let pos = self.top as f32 / size as f32;
        let h = bar.height();
        let th = (frac_len * h).max(16.0).min(h);
        let thumb = Rect::from_min_size(Pos2::new(bar.left() + 2.0, (bar.top() + pos * h).min(bar.bottom() - th)), Vec2::new(bar.width() - 4.0, th));
        let col = if resp.dragged() || resp.hovered() { v.widgets.hovered.bg_fill } else { v.widgets.inactive.bg_fill };
        ui.painter().rect_filled(thumb, 4.0, col);
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let f = ((p.y - bar.top()) / h).clamp(0.0, 1.0) as f64;
                self.jump(if f >= 0.999 { Jump::End } else { Jump::Fraction(f) });
            }
        }
    }
}

/// What to type for Ctrl+G: a line number, `50%`, or an offset `0x1F00`.
pub fn parse_goto(s: &str) -> Option<Jump> {
    let s = s.trim().replace([' ', '\u{a0}'], "");
    if let Some(p) = s.strip_suffix('%') {
        let v: f64 = p.replace(',', ".").parse().ok()?;
        return (0.0..=100.0).contains(&v).then_some(if v >= 100.0 { Jump::End } else { Jump::Fraction(v / 100.0) });
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).ok().map(Jump::Offset);
    }
    s.parse::<u64>().ok().filter(|&n| n > 0).map(Jump::Line)
}

/// Lay out one row: tabs as spaces, control characters as dots, colors of
/// the language, search hits and the selection behind. Returns the job and
/// the file offset of every character of its text.
#[allow(clippy::too_many_arguments)]
fn layout_row(
    text: &str,
    src: &[u32],
    start: u64,
    len: usize,
    font: &FontId,
    fg: Color32,
    spans: Option<(&[Span], u64)>,
    sel: Option<(u64, u64)>,
    marks: &[(u64, u64)],
    current: Option<(u64, u64)>,
    sel_bg: Color32,
    mark_bg: Color32,
    cur_bg: Color32,
    wrap: f32,
) -> (LayoutJob, Vec<u64>) {
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: wrap, break_anywhere: false, ..Default::default() }, ..Default::default() };
    let mut offs = Vec::with_capacity(text.len());
    let mut buf = String::new();
    let mut cur_fmt: Option<TextFormat> = None;
    let mut col = 0usize;
    let mut span_i = 0usize;
    let push = |job: &mut LayoutJob, buf: &mut String, fmt: &Option<TextFormat>| {
        if let Some(f) = fmt {
            if !buf.is_empty() {
                job.append(buf, 0.0, f.clone());
                buf.clear();
            }
        }
    };
    for (k, ch) in text.chars().enumerate() {
        let off = start + src.get(k).copied().unwrap_or(len as u32) as u64;
        // Color by the language.
        let (mut color, mut italics) = (fg, false);
        if let Some((sp, line)) = spans {
            let rel = off.saturating_sub(line) as u32;
            while span_i < sp.len() && sp[span_i].end <= rel {
                span_i += 1;
            }
            if let Some(s) = sp.get(span_i).filter(|s| s.start <= rel) {
                color = Color32::from_rgb(s.rgb[0], s.rgb[1], s.rgb[2]);
                italics = s.italic;
            }
        }
        let bg = if sel.is_some_and(|(a, z)| off >= a && off < z) {
            sel_bg
        } else if current.is_some_and(|(a, z)| off >= a && off < z) {
            cur_bg
        } else if marks.iter().any(|&(a, z)| off >= a && off < z) {
            mark_bg
        } else {
            Color32::TRANSPARENT
        };
        let fmt = TextFormat { font_id: font.clone(), color, background: bg, italics, ..Default::default() };
        if cur_fmt.as_ref() != Some(&fmt) {
            push(&mut job, &mut buf, &cur_fmt);
            cur_fmt = Some(fmt);
        }
        match ch {
            '\t' => {
                let n = TAB - col % TAB;
                for _ in 0..n {
                    buf.push(' ');
                    offs.push(off);
                }
                col += n;
            }
            c if c.is_control() => {
                buf.push('·');
                offs.push(off);
                col += 1;
            }
            c => {
                buf.push(c);
                offs.push(off);
                col += 1;
            }
        }
    }
    push(&mut job, &mut buf, &cur_fmt);
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat { font_id: font.clone(), color: fg, ..Default::default() });
    }
    (job, offs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_understands_lines_percent_and_offsets() {
        assert_eq!(parse_goto("1 234"), Some(Jump::Line(1234)));
        assert_eq!(parse_goto("50 %"), Some(Jump::Fraction(0.5)));
        assert_eq!(parse_goto("100%"), Some(Jump::End));
        assert_eq!(parse_goto("0x1F"), Some(Jump::Offset(31)));
        assert_eq!(parse_goto("0"), None);
        assert_eq!(parse_goto("abc"), None);
    }

    #[test]
    fn rows_expand_tabs_and_keep_file_offsets() {
        let text = "a\tb\u{1}";
        let src = [0u32, 1, 2, 3];
        let (job, offs) = layout_row(text, &src, 100, 4, &FontId::monospace(12.0), Color32::WHITE, None, None, &[], None, Color32::BLUE, Color32::YELLOW, Color32::RED, f32::INFINITY);
        assert_eq!(job.text, "a   b·");
        assert_eq!(offs, vec![100, 101, 101, 101, 102, 103]);
    }
}
