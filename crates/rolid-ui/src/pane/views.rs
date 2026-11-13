//! The three listing styles of a panel: detailed (columns), brief (names in
//! columns, filled column by column) and thumbnails (a grid of previews).
//! All of them share one way of handling clicks, marks and the context menu.

use std::collections::HashSet;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Response, Sense, Stroke, StrokeKind, Vec2};
use rolid_core::Entry;

use crate::icons;

use super::kinds::{FileKind, KindColors};
use super::layout;
use super::thumbs::{self, Thumb};
use super::columns::{self, Columns};
use super::{ext_of, fmt_mode, fmt_time, human_size, open_with_items, Pane, PaneCmd, PaneEvent, Row, row_h};

/// Narrowest column of the brief listing.
pub const BRIEF_MIN_COL: f32 = 190.0;

/// What the views need from the app besides the panel itself.
pub struct ViewCtx<'a> {
    pub thumbs: &'a mut thumbs::Thumbs,
    pub thumb_px: u32,
    pub remote_thumbs: bool,
    pub remote_max: u64,
    /// Thumbnails also on production servers.
    pub production_thumbs: bool,
    /// Text colors by file type.
    pub kinds: KindColors,
    /// The optional Ext column and dates with seconds.
    pub ext_column: bool,
    pub seconds: bool,
    /// Folder names in [brackets] (full and brief listing).
    pub brackets: bool,
    /// Favorite folders for the "*" list: (label, place).
    pub favorites: &'a [(String, rolid_config::Location)],
    /// The bar above the panel is shown (else the path row has / .. ~ *).
    pub nav_bar: bool,
    /// Column widths of this panel's detailed listing.
    pub widths: rolid_config::ColumnWidths,
}

pub struct ItemColors {
    pub text: Color32,
    pub weak: Color32,
    pub marked: Color32,
    pub sel_fill: Color32,
    pub active: bool,
    /// Color of each file category (`None` = normal text).
    kind: [Option<Color32>; 11],
    pub ext_column: bool,
    pub seconds: bool,
    pub brackets: bool,
    pub widths: rolid_config::ColumnWidths,
    /// Column separators.
    pub line: Color32,
}

/// Cursor row of the active panel and marked names: every name color keeps
/// a contrast of at least 4.5 : 1 on them and on any tab color (see the test).
pub const CURSOR_LIGHT: Color32 = Color32::from_rgb(184, 224, 255);
pub const CURSOR_DARK: Color32 = Color32::from_rgb(20, 60, 90);
pub const MARKED_LIGHT: Color32 = Color32::from_rgb(180, 0, 0);
pub const MARKED_DARK: Color32 = Color32::from_rgb(255, 140, 140);

impl ItemColors {
    pub fn new(ui: &egui::Ui, active: bool, view: &ViewCtx) -> Self {
        let dark = ui.visuals().dark_mode;
        let mut kind = [None; 11];
        for k in FileKind::COLORED {
            kind[k as usize] = view.kinds.color(k, dark);
        }
        ItemColors {
            kind,
            ext_column: view.ext_column,
            seconds: view.seconds,
            brackets: view.brackets,
            widths: view.widths,
            line: separator_color(ui),
            text: ui.visuals().text_color(),
            weak: ui.visuals().weak_text_color(),
            marked: if dark { MARKED_DARK } else { MARKED_LIGHT },
            sel_fill: match (active, dark) {
                (true, false) => CURSOR_LIGHT,
                (true, true) => CURSOR_DARK,
                _ => ui.visuals().widgets.inactive.bg_fill,
            },
            active,
        }
    }

    /// Color of an entry's name and icon.
    pub fn of(&self, k: FileKind) -> Color32 {
        self.kind[k as usize].unwrap_or(self.text)
    }
}

/// Color of the column separators: faint, but seen on the cursor row too.
pub fn separator_color(ui: &egui::Ui) -> Color32 {
    let base = ui.visuals().widgets.noninteractive.bg_stroke.color;
    if ui.visuals().dark_mode {
        base
    } else {
        base.gamma_multiply(0.9)
    }
}

/// The name as the listing shows it: without the extension when the Ext
/// column shows it, a folder in [brackets] when asked. Only for painting; the
/// sort, filter, clipboard and renaming use the real name.
pub fn shown_name(e: &Entry, strip_ext: bool, brackets: bool) -> std::borrow::Cow<'_, str> {
    if e.is_dir_like() {
        return if brackets { format!("[{}]", e.name).into() } else { e.name.as_str().into() };
    }
    let ext = ext_of(&e.name);
    if strip_ext && !ext.is_empty() {
        e.name[..e.name.len() - ext.len() - 1].into()
    } else {
        e.name.as_str().into()
    }
}

impl Pane {
    /// Cursor frame or fill.
    fn paint_cursor(&self, p: &egui::Painter, rect: Rect, c: &ItemColors) {
        if c.active {
            p.rect_filled(rect, 2.0, c.sel_fill);
        } else {
            p.rect_stroke(rect, 2.0, Stroke::new(crate::fonts::hairline(), c.weak), StrokeKind::Inside);
        }
    }

    /// Icon and name of a row in `rect` (left aligned); a name that does not
    /// fit ends with "…". Returns the text rect and whether the name was cut.
    fn paint_name(&self, p: &egui::Painter, rect: Rect, row: Row, c: &ItemColors, strip_ext: bool) -> (Rect, bool) {
        let pos = rect.left_center() + Vec2::new(6.0, 0.0);
        let room = |x: f32| rect.right() - columns::PAD - x;
        match row {
            Row::Up => {
                let text = if self.search.is_some() { format!("⮪ .. {}", tr!("(back to the folder)")) } else { "⮪ ..".to_string() };
                let (g, cut) = columns::elided(p, text, crate::fonts::prop(14.0), c.text, room(pos.x));
                let r = Rect::from_min_size(pos - Vec2::new(0.0, g.size().y / 2.0), g.size());
                p.galley(r.min, g, c.text);
                (r, cut)
            }
            Row::Entry(i) => {
                let Some(e) = self.entries.get(i) else { return (Rect::NOTHING, false) };
                let marked = self.marked.contains(&e.name);
                let kind = FileKind::of(e);
                let kc = c.of(kind);
                let color = if marked { c.marked } else { kc };
                let font = if e.is_dir_like() { crate::fonts::prop(14.0) } else { crate::fonts::prop(13.5) };
                let ir = p.text(pos, Align2::LEFT_CENTER, kind.glyph(), icons::font(15.0), kc);
                let name = shown_name(e, strip_ext, c.brackets);
                let txt = if marked { format!("✔ {name}") } else { name.to_string() };
                let x = ir.right() + 4.0;
                let (g, cut) = columns::elided(p, txt, font, color, room(x));
                let tr = Rect::from_min_size(egui::pos2(x, rect.center().y - g.size().y / 2.0), g.size());
                p.galley(tr.min, g, color);
                (ir.union(tr), cut)
            }
        }
    }

    /// Width of the widest text in column `col` (its title included): a
    /// double click on the separator fits the column to it.
    pub(super) fn content_width(&self, p: &egui::Painter, col: columns::Col, title: &str, seconds: bool) -> f32 {
        use columns::Col;
        let measure = |t: String, font: FontId| p.layout_no_wrap(t, font, Color32::WHITE).size().x;
        let mono = FontId::monospace(crate::fonts::fs(12.0));
        // The title with room for the sort arrow.
        let mut w = measure(format!("{title} ▲"), crate::fonts::prop(12.5));
        // Monospace: the longest text is the widest.
        let longest = |texts: &mut dyn Iterator<Item = String>| texts.max_by_key(|t| t.chars().count()).unwrap_or_default();
        let entries = self.entries.iter();
        let content = match col {
            Col::Ext => {
                let exts: HashSet<&str> = entries.filter(|e| !e.is_dir_like()).map(|e| ext_of(&e.name)).collect();
                exts.into_iter().map(|x| measure(x.to_string(), crate::fonts::prop(13.0))).fold(0.0, f32::max)
            }
            Col::Size => {
                let mut texts = entries.map(|e| match e.is_dir_like().then(|| self.dir_sizes.get(&e.name)).flatten() {
                    Some(d) => d.text(),
                    None if e.is_dir_like() && self.dir_pending.contains(&e.name) => trc!("column", "computing…"),
                    None if e.is_dir_like() => trc!("column", "<DIR>"),
                    None => human_size(e.size),
                });
                measure(longest(&mut texts), mono.clone()).max(measure(trc!("column", "<UP>"), mono))
            }
            Col::Date => measure(longest(&mut entries.map(|e| fmt_time(e.modified, seconds))), mono),
            Col::Mode => measure(longest(&mut entries.map(fmt_mode)), mono),
        };
        w = w.max(content);
        w
    }

    /// The whole name of a row whose name was cut, at the pointer.
    fn full_name_tip(&self, resp: &Response, row: Row) {
        if let (true, Row::Entry(i)) = (resp.hovered(), row) {
            if let Some(e) = self.entries.get(i) {
                resp.clone().on_hover_text_at_pointer(&e.name);
            }
        }
    }

    /// The row of an item being renamed: its icon and the text field after it.
    fn paint_editing(&mut self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect, i: usize, c: &ItemColors, events: &mut Vec<PaneEvent>) -> Rect {
        let Some(e) = self.entries.get(i) else { return Rect::NOTHING };
        let kind = FileKind::of(e);
        let ir = p.text(rect.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, kind.glyph(), icons::font(15.0), c.of(kind));
        let er = Rect::from_min_max(
            egui::pos2(ir.right() + 3.0, rect.top() + 1.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
        );
        self.inline_editor(ui, er, events);
        er
    }

    /// Clicks, Ctrl+click, double click and the context menu of one item.
    fn interact_item(&mut self, ui: &egui::Ui, resp: &Response, idx: usize, row: Row, events: &mut Vec<PaneEvent>) {
        // Dragging a row (not "..", not one being renamed): the app takes over.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Row::Entry(i) = row {
                if !self.renaming_entry(i) {
                    if let Some(e) = self.entries.get(i) {
                        let name = e.name.clone();
                        if !self.marked.contains(&name) {
                            self.cursor = idx;
                        }
                        events.push(PaneEvent::Focus);
                        events.push(PaneEvent::DragStart(name));
                    }
                }
            }
        }
        if resp.clicked() || resp.secondary_clicked() {
            self.cursor = idx;
            events.push(PaneEvent::Focus);
            if resp.clicked() && ui.input(|i| i.modifiers.ctrl) {
                if let Row::Entry(i) = row {
                    let n = self.entries[i].name.clone();
                    if !self.marked.remove(&n) {
                        self.marked.insert(n);
                    }
                }
            }
        }
        if row == Row::Up {
            self.folder_menu(resp, events);
        }
        if let Row::Entry(i) = row {
            let is_file = self.entries.get(i).is_some_and(|e| !e.is_dir_like());
            let marked = self.entries.get(i).is_some_and(|e| self.marked.contains(&e.name));
            let name = self.entries.get(i).map(|e| e.name.clone()).unwrap_or_default();
            let remote = self.fs.is_remote();
            let is_archive = is_file && rolid_archive::Format::from_name(&name).is_some();
            // Inside an archive nothing can be written.
            let writable = !self.fs.read_only();
            resp.context_menu(|ui| {
                let mut picked: Option<PaneCmd> = None;
                let item = |ui: &mut egui::Ui, picked: &mut Option<PaneCmd>, text: &str, key: &str, cmd: PaneCmd, on: bool| {
                    let b = egui::Button::new(text).shortcut_text(key);
                    if ui.add_enabled(on, b).clicked() {
                        *picked = Some(cmd);
                        ui.close();
                    }
                };
                item(ui, &mut picked, &trc!("verb", "Open"), "Enter", PaneCmd::Open, true); // i18n-ok: key name
                ui.add_enabled_ui(is_file, |ui| {
                    ui.menu_button(tr!("Open with"), |ui| {
                        for (cmd, text) in open_with_items(&name) {
                            if ui.button(text).clicked() {
                                picked = Some(cmd);
                                ui.close();
                            }
                        }
                    });
                });
                item(ui, &mut picked, &trc!("verb", "View"), "F3", PaneCmd::View, is_file);
                item(ui, &mut picked, &tr!("Edit in external editor"), "F4", PaneCmd::Edit, is_file && writable);
                item(ui, &mut picked, &tr!("New file…"), "Shift+F4", PaneCmd::NewFile, writable && self.search.is_none()); // i18n-ok: key name
                ui.separator();
                item(ui, &mut picked, &tr!("Copy to other panel"), "F5", PaneCmd::Copy, true);
                item(ui, &mut picked, &tr!("Move to other panel"), "F6", PaneCmd::Move, true);
                item(ui, &mut picked, &tr!("Rename"), "F2 / Shift+F6", PaneCmd::Rename, writable && self.search.is_none()); // i18n-ok: key name
                item(ui, &mut picked, &tr!("Delete"), "F8", PaneCmd::Delete, writable);
                item(ui, &mut picked, &tr!("Permissions…"), "Ctrl+Shift+A", PaneCmd::Perms, writable); // i18n-ok: key name
                ui.separator();
                item(ui, &mut picked, &tr!("Pack…"), "Alt+F5", PaneCmd::Pack, true); // i18n-ok: key name
                item(ui, &mut picked, &tr!("Extract…"), "Alt+F9", PaneCmd::Unpack, is_archive); // i18n-ok: key name
                item(ui, &mut picked, &tr!("Test archive"), "Alt+Shift+F9", PaneCmd::TestArchive, is_archive); // i18n-ok: key name
                ui.separator();
                item(ui, &mut picked, &tr!("Calculate size of selected folders"), "Alt+Space / Shift+Space", PaneCmd::SizeSelected, true); // i18n-ok: key name
                item(ui, &mut picked, &tr!("Calculate size of all folders"), "Alt+Shift+Enter", PaneCmd::SizeAll, true); // i18n-ok: key name
                item(ui, &mut picked, &tr!("Calculate checksums…"), "", PaneCmd::Checksums, true);
                ui.menu_button(tr!("Copy to clipboard"), |ui| {
                    item(ui, &mut picked, &tr!("Copy name"), "Ctrl+Shift+N", PaneCmd::CopyNames, true); // i18n-ok: key name
                    item(ui, &mut picked, &tr!("Copy full path"), "Ctrl+Shift+P", PaneCmd::CopyPaths, true); // i18n-ok: key name
                    item(ui, &mut picked, &tr!("Copy as sftp://…"), "Ctrl+Shift+S", PaneCmd::CopyUrls, remote); // i18n-ok: key name
                });
                ui.separator();
                let t = if marked { tr!("Unmark") } else { tr!("Mark") };
                item(ui, &mut picked, &t, &tr!("Insert / Ctrl+click"), PaneCmd::ToggleMark, true);
                if let Some(c) = picked {
                    events.push(PaneEvent::Command(c));
                }
            });
        }
        if resp.double_clicked() {
            self.cursor = idx;
            events.push(PaneEvent::Activate);
        }
    }

    /// The context menu of the folder itself: a right click on the empty
    /// space of the list (or on "..").
    pub(super) fn folder_menu(&self, resp: &Response, events: &mut Vec<PaneEvent>) {
        if resp.secondary_clicked() {
            events.push(PaneEvent::Focus);
        }
        let in_folder = self.search.is_none();
        let writable = in_folder && !self.fs.read_only() && !self.offline;
        resp.context_menu(|ui| {
            let mut picked: Option<PaneCmd> = None;
            let mut item = |ui: &mut egui::Ui, text: &str, key: &str, cmd: PaneCmd, on: bool| {
                if ui.add_enabled(on, egui::Button::new(text).shortcut_text(key)).clicked() {
                    picked = Some(cmd);
                    ui.close();
                }
            };
            item(ui, &tr!("New file…"), "Shift+F4", PaneCmd::NewFile, writable); // i18n-ok: key name
            item(ui, &tr!("New folder"), "F7", PaneCmd::NewFolder, writable);
            ui.separator();
            item(ui, &tr!("Refresh"), "Ctrl+R", PaneCmd::Refresh, true); // i18n-ok: key name
            item(ui, &tr!("Add folder to favorites"), "", PaneCmd::AddFavorite, in_folder);
            item(ui, &tr!("Open terminal here"), "Ctrl+P", PaneCmd::Terminal, true); // i18n-ok: key name
            ui.separator();
            item(ui, &tr!("Disk properties…"), "", PaneCmd::DiskProps, !self.offline);
            if let Some(c) = picked {
                events.push(PaneEvent::Command(c));
            }
        });
    }

    fn note_drop_dir(&self, row: Row, rect: Rect, clip: Rect, drop_dirs: &mut Vec<(Rect, String)>) {
        // ".." is a target too (the parent folder), except at the top of an archive.
        if row == Row::Up && self.search.is_none() && self.archive.is_none() && self.fs.parent(&self.path).is_some() {
            drop_dirs.push((rect.intersect(clip), super::UP.to_string()));
        }
        if let Row::Entry(i) = row {
            if let Some(e) = self.entries.get(i) {
                if e.is_dir_like() && self.search.is_none() {
                    drop_dirs.push((rect.intersect(clip), e.name.clone()));
                }
            }
        }
    }

    // ------------------------------------------------------------ detailed

    pub(super) fn show_full(
        &mut self,
        ui: &mut egui::Ui,
        list_rect: Rect,
        c: &ItemColors,
        events: &mut Vec<PaneEvent>,
        drop_dirs: &mut Vec<(Rect, String)>,
    ) {
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        list_ui.spacing_mut().item_spacing.y = 0.0;
        self.viewport_h = list_rect.height();
        if self.scroll_to_cursor {
            let y = self.cursor as f32 * row_h();
            if y < self.scroll_offset {
                self.scroll_offset = y;
            } else if y + row_h() > self.scroll_offset + self.viewport_h {
                self.scroll_offset = y + row_h() - self.viewport_h;
            }
        }
        let mut sa = egui::ScrollArea::vertical().id_salt("list").auto_shrink([false, false]);
        if self.scroll_to_cursor {
            sa = sa.vertical_scroll_offset(self.scroll_offset);
            self.scroll_to_cursor = false;
        }
        let total = self.view.len();
        let out = sa.show_rows(&mut list_ui, row_h(), total, |ui, range| {
            for idx in range {
                let row = self.view[idx];
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h()), Sense::click_and_drag());
                let p = ui.painter_at(rect);
                if idx == self.cursor {
                    self.paint_cursor(&p, rect, c);
                }
                let cols = Columns::new(rect, c.ext_column, c.seconds, &c.widths);
                self.note_drop_dir(row, rect, ui.clip_rect(), drop_dirs);
                let np = ui.painter_at(cols.name);
                let pointer = ui.ctx().pointer_hover_pos().filter(|_| resp.hovered());
                let nr = match row {
                    Row::Entry(i) if self.renaming_entry(i) => self.paint_editing(ui, &np, cols.name, i, c, events),
                    _ => {
                        let (r, cut) = self.paint_name(&np, cols.name, row, c, c.ext_column);
                        if cut && pointer.is_some_and(|p| cols.name.contains(p)) {
                            self.full_name_tip(&resp, row);
                        }
                        r
                    }
                };
                for (_, x) in cols.separators() {
                    columns::separator(&p, x, rect.top(), rect.bottom(), c.line);
                }
                match row {
                    Row::Up => {
                        ui.painter_at(cols.size).text(
                            cols.size.right_center() - Vec2::new(6.0, 0.0),
                            Align2::RIGHT_CENTER,
                            trc!("column", "<UP>"),
                            FontId::monospace(crate::fonts::fs(12.0)),
                            c.weak,
                        );
                    }
                    Row::Entry(i) => {
                        let Some(e) = self.entries.get(i) else { continue };
                        let color = if self.marked.contains(&e.name) { c.marked } else { c.text };
                        // Content search: show the first matching line after the name.
                        if let Some((ln, text)) = self.hit(&e.name).and_then(|h| h.line.as_ref()) {
                            np.text(
                                egui::pos2(nr.right() + 10.0, cols.name.center().y),
                                Align2::LEFT_CENTER,
                                format!("{ln}: {text}"),
                                FontId::monospace(crate::fonts::fs(11.5)),
                                c.weak,
                            );
                        }
                        if cols.ext.width() > 1.0 && !e.is_dir_like() {
                            let ep = ui.painter_at(cols.ext);
                            let ext = ext_of(&e.name);
                            let room = cols.ext.width() - 2.0 * columns::PAD;
                            let (g, cut) = columns::elided(&ep, ext.to_string(), crate::fonts::prop(13.0), color, room);
                            ep.galley(cols.ext.left_center() + Vec2::new(columns::PAD, -g.size().y / 2.0), g, color);
                            if cut && pointer.is_some_and(|p| cols.ext.contains(p)) {
                                resp.clone().on_hover_text_at_pointer(ext);
                            }
                        }
                        let dsize = e.is_dir_like().then(|| self.dir_sizes.get(&e.name)).flatten();
                        let size = if let Some(d) = dsize {
                            d.text()
                        } else if e.is_dir_like() && self.dir_pending.contains(&e.name) {
                            trc!("column", "computing…")
                        } else if e.is_dir_like() {
                            trc!("column", "<DIR>")
                        } else {
                            human_size(e.size)
                        };
                        if let Some(note) = dsize.and_then(|d| d.note()) {
                            if resp.hovered() && ui.ctx().pointer_hover_pos().is_some_and(|p| cols.size.contains(p)) {
                                resp.clone().on_hover_text_at_pointer(note);
                            }
                        }
                        ui.painter_at(cols.size).text(
                            cols.size.right_center() - Vec2::new(6.0, 0.0),
                            Align2::RIGHT_CENTER,
                            size,
                            FontId::monospace(crate::fonts::fs(12.0)),
                            color,
                        );
                        ui.painter_at(cols.date).text(
                            cols.date.left_center() + Vec2::new(6.0, 0.0),
                            Align2::LEFT_CENTER,
                            fmt_time(e.modified, c.seconds),
                            FontId::monospace(crate::fonts::fs(12.0)),
                            c.weak,
                        );
                        ui.painter_at(cols.mode).text(
                            cols.mode.left_center() + Vec2::new(6.0, 0.0),
                            Align2::LEFT_CENTER,
                            fmt_mode(e),
                            FontId::monospace(crate::fonts::fs(12.0)),
                            c.weak,
                        );
                    }
                }
                self.interact_item(ui, &resp, idx, row, events);
            }
        });
        self.scroll_offset = out.state.clamped_offset().y;
    }

    // --------------------------------------------------------------- brief

    pub(super) fn show_brief(
        &mut self,
        ui: &mut egui::Ui,
        list_rect: Rect,
        c: &ItemColors,
        events: &mut Vec<PaneEvent>,
        drop_dirs: &mut Vec<(Rect, String)>,
    ) {
        let rows = layout::brief_rows(list_rect.height(), row_h());
        let visible = layout::brief_visible_cols(list_rect.width(), BRIEF_MIN_COL);
        let col_w = list_rect.width() / visible as f32;
        let total = self.view.len();
        let cols = layout::brief_cols(total, rows);
        self.brief_rows = rows;
        self.viewport_h = list_rect.height();
        // Mouse wheel scrolls by columns.
        if ui.rect_contains_pointer(list_rect) {
            let step: f32 = ui.input(|i| {
                i.raw
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel { delta, .. } => {
                            let s = if delta.y.abs() > delta.x.abs() { -delta.y } else { -delta.x };
                            Some(s.signum())
                        }
                        _ => None,
                    })
                    .sum()
            });
            if step != 0.0 {
                let max = cols.saturating_sub(visible) as isize;
                self.brief_first = (self.brief_first as isize + step as isize).clamp(0, max) as usize;
            }
        }
        if self.scroll_to_cursor {
            self.brief_first = layout::brief_scroll(self.brief_first, self.cursor, rows, visible);
            self.scroll_to_cursor = false;
        }
        self.brief_first = self.brief_first.min(cols.saturating_sub(1));
        let clip = list_rect;
        let painter = ui.painter_at(clip);
        let first = self.brief_first * rows;
        let last = ((self.brief_first + visible) * rows).min(total);
        for idx in first..last {
            let (col, r) = layout::brief_pos(idx, rows);
            let x = list_rect.left() + (col - self.brief_first) as f32 * col_w;
            let rect = Rect::from_min_size(egui::pos2(x, list_rect.top() + r as f32 * row_h()), Vec2::new(col_w, row_h()));
            let row = self.view[idx];
            let resp = ui.interact(rect, ui.scope_id().with(("brief", idx)), Sense::click_and_drag());
            if idx == self.cursor {
                self.paint_cursor(&painter, rect.shrink2(Vec2::new(1.0, 0.0)), c);
            }
            self.note_drop_dir(row, rect, clip, drop_dirs);
            let np = ui.painter_at(rect.shrink2(Vec2::new(2.0, 0.0)).intersect(clip));
            match row {
                Row::Entry(i) if self.renaming_entry(i) => {
                    self.paint_editing(ui, &np, rect.shrink2(Vec2::new(2.0, 0.0)), i, c, events);
                }
                _ => {
                    let (_, cut) = self.paint_name(&np, rect, row, c, false);
                    if cut {
                        self.full_name_tip(&resp, row);
                    }
                }
            }
            self.interact_item(ui, &resp, idx, row, events);
        }
        // Column separators.
        for k in 1..visible {
            let x = list_rect.left() + k as f32 * col_w;
            painter.vline(x, list_rect.y_range(), Stroke::new(crate::fonts::hairline(), c.weak.gamma_multiply(0.25)));
        }
        if cols > visible {
            // Where we are among the columns.
            let frac = visible as f32 / cols as f32;
            let w = list_rect.width() * frac;
            let x = list_rect.left() + list_rect.width() * self.brief_first as f32 / cols as f32;
            let bar = Rect::from_min_size(egui::pos2(x, list_rect.bottom() - 3.0), Vec2::new(w, 3.0));
            painter.rect_filled(bar, 1.0, c.weak.gamma_multiply(0.6));
        }
        ui.allocate_rect(list_rect, Sense::hover());
    }

    // ---------------------------------------------------------- thumbnails

    pub(super) fn show_thumbs(
        &mut self,
        ui: &mut egui::Ui,
        list_rect: Rect,
        c: &ItemColors,
        view: &mut ViewCtx,
        events: &mut Vec<PaneEvent>,
        drop_dirs: &mut Vec<(Rect, String)>,
    ) {
        let px = view.thumb_px as f32;
        let label_h = 18.0;
        let tile = Vec2::new(px + 16.0, px + label_h + 12.0);
        let gcols = layout::grid_cols(list_rect.width(), tile.x);
        let tile = Vec2::new(list_rect.width() / gcols as f32, tile.y);
        self.grid_cols = gcols;
        self.thumb_tile_h = tile.y;
        self.viewport_h = list_rect.height();
        let total = self.view.len();
        let grows = total.div_ceil(gcols);
        if self.scroll_to_cursor {
            let y = (self.cursor / gcols) as f32 * tile.y;
            if y < self.scroll_offset {
                self.scroll_offset = y;
            } else if y + tile.y > self.scroll_offset + self.viewport_h {
                self.scroll_offset = y + tile.y - self.viewport_h;
            }
        }
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        list_ui.spacing_mut().item_spacing = Vec2::ZERO;
        let mut sa = egui::ScrollArea::vertical().id_salt("thumbs").auto_shrink([false, false]);
        if self.scroll_to_cursor {
            sa = sa.vertical_scroll_offset(self.scroll_offset);
            self.scroll_to_cursor = false;
        }
        // Archives: previews are made from unpacked copies – the same size limit as remote.
        let remote = self.fs.is_remote() || self.fs.read_only();
        let production = self.remote.as_ref().is_some_and(|r| r.production) && !view.production_thumbs;
        let fs = self.fs.clone();
        let cancel = self.thumb_cancel.clone();
        let out = sa.show_rows(&mut list_ui, tile.y, grows, |ui, range| {
            for gr in range {
                let (row_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), tile.y), Sense::hover());
                for gc in 0..gcols {
                    let idx = gr * gcols + gc;
                    if idx >= total {
                        break;
                    }
                    let rect = Rect::from_min_size(row_rect.min + Vec2::new(gc as f32 * tile.x, 0.0), tile);
                    let row = self.view[idx];
                    let resp = ui.interact(rect, ui.scope_id().with(("thumb", idx)), Sense::click_and_drag());
                    let p = ui.painter_at(rect.intersect(ui.clip_rect()));
                    if idx == self.cursor {
                        self.paint_cursor(&p, rect.shrink(2.0), c);
                    }
                    self.note_drop_dir(row, rect, ui.clip_rect(), drop_dirs);
                    let img_rect = Rect::from_min_size(rect.min + Vec2::new((tile.x - px) / 2.0, 6.0), Vec2::splat(px));
                    let (glyph, name, marked) = match row {
                        Row::Up => ("⮪", "..".to_string(), false),
                        Row::Entry(i) => {
                            let e = &self.entries[i];
                            let marked = self.marked.contains(&e.name);
                            let mut drawn = false;
                            let path = match self.hit(&e.name) {
                                Some(h) => fs.join(&h.dir, &h.entry.name),
                                None => fs.join(&self.path, &e.name),
                            };
                            if !e.is_dir_like()
                                && thumbs::refusal(&e.name, e.size, remote, view.remote_thumbs, production, view.remote_max).is_none()
                            {
                                match view.thumbs.get(&fs, path, e, view.thumb_px, &cancel) {
                                    Thumb::Ready(tex, size) => {
                                        let r = Rect::from_center_size(img_rect.center(), size.min(Vec2::splat(px)));
                                        p.image(tex, r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                        drawn = true;
                                    }
                                    Thumb::Loading => {
                                        p.rect_stroke(img_rect.shrink(px * 0.2), 4.0, Stroke::new(crate::fonts::hairline(), c.weak.gamma_multiply(0.5)), StrokeKind::Inside);
                                        p.text(img_rect.center(), Align2::CENTER_CENTER, "⏳", FontId::proportional(px * 0.25), c.weak);
                                        drawn = true;
                                    }
                                    Thumb::Failed => {}
                                }
                            }
                            if !drawn {
                                let k = FileKind::of(e);
                                p.text(img_rect.center(), Align2::CENTER_CENTER, k.glyph(), icons::font(px * 0.5), c.of(k));
                            }
                            ("", e.name.clone(), marked)
                        }
                    };
                    if !glyph.is_empty() {
                        p.text(img_rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(px * 0.45), c.text);
                    }
                    let color = if marked { c.marked } else { c.text };
                    let label = if marked { format!("✔ {name}") } else { name };
                    let lr = Rect::from_min_size(egui::pos2(rect.left() + 4.0, img_rect.bottom() + 2.0), Vec2::new(tile.x - 8.0, label_h));
                    if let Row::Entry(i) = row {
                        if self.renaming_entry(i) {
                            self.inline_editor(ui, lr.expand2(Vec2::new(0.0, 2.0)), events);
                            self.interact_item(ui, &resp, idx, row, events);
                            continue;
                        }
                    }
                    let font = crate::fonts::prop(12.5);
                    let galley = ui.fonts_mut(|f| {
                        f.layout_job(egui::text::LayoutJob {
                            wrap: egui::text::TextWrapping::truncate_at_width(lr.width()),
                            ..egui::text::LayoutJob::simple_singleline(label, font, color)
                        })
                    });
                    let x = lr.center().x - galley.size().x / 2.0;
                    p.galley(egui::pos2(x, lr.top()), galley, color);
                    self.interact_item(ui, &resp, idx, row, events);
                }
            }
        });
        self.scroll_offset = out.state.clamped_offset().y;
    }
}
