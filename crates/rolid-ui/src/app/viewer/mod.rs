//! The F3 viewer: text (with highlighting), hex and images, in a window of
//! its own (several at once) where the system allows it, otherwise inside
//! the main window (egui does that by itself). It never runs anything and
//! never loads anything from outside: HTML, XML and SVG are shown as text.

mod image;
mod text;

use std::sync::Arc;

use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use rolid_core::{Entry, Vfs};
use rolid_i18n::{tr, trc};
use rolid_ssh::Link;
use rolid_view::doc::{Doc, Health, Opts};
use rolid_view::enc::Enc;
use rolid_view::index::LineNo;
use rolid_view::syntax::{self, Off};

use super::{App, PaneRef};
use crate::icons;
use crate::pane::Target;
pub use text::{Jump, TextView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Hex,
    Image,
}

/// What a viewer asks the application to do.
pub enum Act {
    /// F4 on the file shown.
    Edit(PaneRef, Target, Entry),
    SaveSettings(Box<dyn FnOnce(&mut rolid_config::Settings)>),
    /// Show image `index` of the viewer's list in viewer `id`.
    ShowImage(u64, usize),
}

pub struct Viewer {
    id: u64,
    pub name: String,
    /// Where the file is: the server, "this computer", the archive.
    place: String,
    at: PaneRef,
    target: Target,
    entry: Entry,
    remote: bool,
    /// Inside an archive (F4 is not possible).
    archived: bool,
    link: Option<Arc<Link>>,
    pub text: TextView,
    close: bool,
    /// The first frame: the window gets the focus.
    fresh: bool,
    /// Copy finished / failed, shown for a moment.
    note: Option<(String, std::time::Instant)>,
    /// The image mode and its picture.
    image_mode: bool,
    image: Option<image::ImageView>,
    /// The images of the panel's folder (in its order) for next / previous.
    images: Vec<(Target, Entry)>,
    /// Most bytes read for an image; the largest texture side.
    image_limit: u64,
    max_side: u32,
    /// The window title sent last.
    last_title: String,
}

#[derive(Default)]
pub struct Viewers {
    pub list: Vec<Viewer>,
    next: u64,
}


/// How the file to view is reached.
pub struct Source {
    pub fs: Arc<dyn Vfs>,
    pub path: String,
    pub place: String,
    pub remote: bool,
    pub archived: bool,
    pub link: Option<Arc<Link>>,
    /// Images of the folder for next / previous (empty: none).
    pub images: Vec<(Target, Entry)>,
}

impl App {
    /// F3 on the active panel.
    pub(super) fn view_active(&mut self) {
        let at = self.active_ref();
        self.open_viewer(at);
    }

    /// Show the file under the cursor of `at`.
    pub(super) fn open_viewer(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        let Some((target, entry)) = p.current_target() else { return };
        if entry.is_dir_like() {
            self.last_error = Some(tr!("F3 shows files – it does nothing on a folder."));
            return;
        }
        let fs = p.fs.clone();
        if fs.read_only() {
            self.view_from_archive(at, target, entry);
            return;
        }
        let path = fs.join(&target.dir, &target.name);
        let place = p.display_label();
        let remote = fs.is_remote();
        let link = self.pane_link(at);
        let images = p.image_targets();
        self.add_viewer(at, target, entry, Source { fs, path, place, remote, archived: false, link, images });
    }

    /// A file inside an archive: shown from a temporary copy.
    fn view_from_archive(&mut self, at: PaneRef, target: Target, entry: Entry) {
        let Some(st) = self.pane(at).and_then(|p| p.archive.as_ref()) else { return };
        let fs = st.fs.clone();
        let path = fs.join(&target.dir, &target.name);
        if fs.is_encrypted_item(&path) && !fs.has_password() {
            self.ask_dir_password(
                tr!("View {name}", name = target.name.as_str()),
                tr!("The file is encrypted in the archive."),
                false,
                Box::new(move |a: &mut App, pw| {
                    fs.set_password(Some(pw));
                    a.view_from_archive(at, target, entry);
                }),
            );
            return;
        }
        let job_fs = fs.clone();
        let p2 = path.clone();
        self.archive_job(
            tr!("View {name}", name = target.name.as_str()),
            move |ctl| {
                ctl.check()?;
                job_fs.extract_file(&p2)
            },
            move |a: &mut App, res| match res {
                Ok(f) => {
                    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
                    let place = target.dir.clone();
                    let path = f.to_string_lossy().into_owned();
                    a.add_viewer(at, target, entry, Source { fs: local, path, place, remote: false, archived: true, link: None, images: Vec::new() });
                }
                Err(e) if e.is_cancel() => {}
                Err(e @ (rolid_archive::Error::NeedPassword | rolid_archive::Error::BadPassword)) => {
                    fs.set_password(None);
                    a.ask_dir_password(
                        tr!("View {name}", name = target.name.as_str()),
                        tr!("The file is encrypted in the archive."),
                        matches!(e, rolid_archive::Error::BadPassword),
                        Box::new(move |a: &mut App, pw| {
                            fs.set_password(Some(pw));
                            a.view_from_archive(at, target, entry);
                        }),
                    );
                }
                Err(e) => a.last_error = Some(tr!("Cannot show “{name}”: {error}", name = target.name.as_str(), error = e.text())),
            },
        );
    }

    fn add_viewer(&mut self, at: PaneRef, target: Target, entry: Entry, src: Source) {
        let ctx = self.ctx.clone();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let opts = if src.remote { Opts::remote() } else { Opts::local() };
        let doc = Doc::open(&self.rt, src.fs.clone(), src.path.clone(), entry.size, entry.modified, opts, repaint.clone());
        let mut text = TextView::new(doc, target.name.clone(), self.rt.clone(), repaint, &self.settings);
        if src.remote {
            if let Some(l) = &src.link {
                text.set_exec(Some(Arc::new(LinkExec(l.clone()))));
            }
        }
        self.viewers.next += 1;
        let id = self.viewers.next;
        tracing::info!(remote = src.remote, archived = src.archived, size = entry.size, "viewer opened");
        let image_limit = if src.remote { self.settings.viewer_image_max() } else { image::MAX_LOCAL };
        let max_side = self.ctx.input(|i| i.max_texture_side).min(16384) as u32;
        let is_image = crate::pane::thumbs::is_image(&target.name);
        let mut v = Viewer {
            id,
            name: target.name.clone(),
            place: src.place,
            at,
            target,
            entry,
            remote: src.remote,
            archived: src.archived,
            link: src.link,
            text,
            close: false,
            fresh: true,
            note: None,
            image_mode: false,
            image: None,
            images: src.images,
            image_limit,
            max_side,
            last_title: String::new(),
        };
        if is_image {
            v.set_mode(Mode::Image);
        }
        self.viewers.list.push(v);
    }

    /// Next / previous image in a viewer: the viewer shows that file now.
    fn viewer_show_image(&mut self, id: u64, index: usize) {
        let ctx = self.ctx.clone();
        let Some(v) = self.viewers.list.iter_mut().find(|v| v.id == id) else { return };
        let Some((target, entry)) = v.images.get(index).cloned() else { return };
        let fs = v.text.doc.fs().clone();
        let path = fs.join(&target.dir, &target.name);
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let opts = if v.remote { Opts::remote() } else { Opts::local() };
        let doc = Doc::open(&self.rt, fs, path, entry.size, entry.modified, opts, repaint.clone());
        let mut text = TextView::new(doc, target.name.clone(), self.rt.clone(), repaint, &self.settings);
        if let Some(l) = v.link.as_ref().filter(|_| v.remote) {
            text.set_exec(Some(Arc::new(LinkExec(l.clone()))));
        }
        v.text = text;
        v.name = target.name.clone();
        v.target = target;
        v.entry = entry;
        v.image = None;
        v.set_mode(Mode::Image);
    }

    /// Is a server connection used by an open viewer?
    pub(super) fn viewers_use_link(&self, key: &str) -> bool {
        let link = self.links.map.get(key).map(|e| e.link.clone());
        link.is_some_and(|l| self.viewers.list.iter().any(|v| v.link.as_ref().is_some_and(|x| Arc::ptr_eq(x, &l))))
    }

    /// Draw the viewers (each in its own window).
    pub(super) fn viewers_ui(&mut self, ctx: &egui::Context) {
        if self.viewers.list.is_empty() {
            return;
        }
        let mut acts: Vec<Act> = Vec::new();
        for v in &mut self.viewers.list {
            let id = egui::ViewportId::from_hash_of(("viewer", v.id));
            let builder = egui::ViewportBuilder::default()
                .with_title(v.title())
                .with_inner_size([1000.0, 720.0])
                .with_min_inner_size([420.0, 260.0])
                .with_app_id("rolidscp");
            ctx.show_viewport_immediate(id, builder, |ui, class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    v.close = true;
                }
                v.ui(ui, class, &mut acts);
            });
        }
        self.viewers.list.retain(|v| !v.close);
        for a in acts {
            match a {
                Act::Edit(at, target, entry) => self.edit_from_viewer(at, target, entry),
                Act::SaveSettings(f) => {
                    f(&mut self.settings);
                    self.save_settings();
                }
                Act::ShowImage(id, index) => self.viewer_show_image(id, index),
            }
        }
    }

    /// F4 in the viewer: as F4 in the panel, on the file shown.
    fn edit_from_viewer(&mut self, at: PaneRef, target: Target, entry: Entry) {
        if self.pane(at).is_none() {
            self.last_error = Some(tr!("The panel of the file is closed: open the file from a panel."));
            return;
        }
        self.edit_target(at, target, entry);
        self.ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
    }
}

impl Viewer {
    /// The window title: file, place, size.
    fn title(&self) -> String {
        format!("{} — {} — {} – RolidSCP", self.name, self.place, rolid_i18n::size(self.text.doc.size())) // i18n-ok: separators and product name
    }

    fn ui(&mut self, ui: &mut egui::Ui, class: egui::ViewportClass, acts: &mut Vec<Act>) {
        let ctx = ui.ctx().clone();
        if self.fresh {
            self.fresh = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // Inside the main window: a fixed size (the window has its own ✕).
            ui.set_min_size(egui::vec2(820.0, 560.0));
        }
        // Only a changed title is sent (every command repaints the window again).
        let title = self.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if let Some(r) = self.text.take_clip() {
            match r {
                Ok(t) => {
                    let n = t.chars().count();
                    ctx.copy_text(t);
                    self.note = Some((tr!("Copied {n} characters.", n = n), std::time::Instant::now()));
                }
                Err(e) => self.note = Some((tr!("Copying failed: {error}", error = e), std::time::Instant::now())),
            }
        }
        self.keys(&ctx, acts);
        egui::Panel::top(egui::Id::unique(("viewer-menu", self.id))).show(ui, |ui| self.menu(ui, acts));
        egui::Panel::top(egui::Id::unique(("viewer-head", self.id))).frame(egui::Frame::NONE).show(ui, |ui| self.head(ui));
        egui::Panel::bottom(egui::Id::unique(("viewer-status", self.id))).show(ui, |ui| self.status(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color)).show(ui, |ui| {
            match &mut self.image {
                Some(img) if self.image_mode => img.ui(ui),
                _ => self.text.ui(ui),
            }
        });
        if self.text.copying || self.text.searching() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn mode(&self) -> Mode {
        if self.image_mode {
            Mode::Image
        } else if self.text.hex {
            Mode::Hex
        } else {
            Mode::Text
        }
    }

    fn set_mode(&mut self, m: Mode) {
        self.image_mode = m == Mode::Image;
        // Lines are not counted for a picture (on a server that would download it).
        self.text.doc.pause_index(self.image_mode);
        if self.image_mode {
            if self.image.is_none() {
                let (rt, repaint) = self.text.runtime();
                let fs = self.text.doc.fs().clone();
                let path = self.text.doc.path().to_string();
                self.image = Some(image::ImageView::start(&rt, fs, path, self.text.doc.size(), self.image_limit, self.max_side, repaint));
            }
            return;
        }
        self.text.hex = m == Mode::Hex;
        self.text.sel = None;
        let top = self.text.top();
        self.text.jump(Jump::Offset(top));
    }

    /// Where this file is in the folder's images.
    fn image_index(&self) -> Option<usize> {
        self.images.iter().position(|(t, _)| t.dir == self.target.dir && t.name == self.target.name)
    }

    /// The next (`d` = 1) or previous (`d` = -1) image; `None` = first / last.
    fn step_image(&self, d: i64) -> Option<Act> {
        let n = self.images.len();
        if n == 0 {
            return None;
        }
        let i = match (self.image_index(), d) {
            (_, i64::MIN) => 0,
            (_, i64::MAX) => n - 1,
            (Some(i), d) => (i as i64 + d).rem_euclid(n as i64) as usize,
            (None, _) => 0,
        };
        Some(Act::ShowImage(self.id, i))
    }

    fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let mut act: Option<Act> = None;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                if self.text.goto.is_some() {
                    self.text.goto = None;
                } else if self.text.close_find() {
                } else {
                    self.close = true;
                }
            }
            if i.consume_key(Modifiers::NONE, Key::F4) {
                act = self.edit_act();
            }
            if i.consume_key(Modifiers::COMMAND, Key::R) {
                self.text.reload();
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) || i.consume_key(Modifiers::NONE, Key::F7) {
                self.text.open_find();
            }
            if i.consume_key(Modifiers::SHIFT, Key::F3) {
                self.text.find_next(false);
            }
            if i.consume_key(Modifiers::NONE, Key::F3) {
                self.text.find_next(true);
            }
            if typing {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Q) {
                self.close = true;
            }
            if i.consume_key(Modifiers::NONE, Key::Num1) {
                self.set_mode(Mode::Text);
            }
            if i.consume_key(Modifiers::NONE, Key::Num2) {
                self.set_mode(Mode::Hex);
            }
            if i.consume_key(Modifiers::NONE, Key::Num3) {
                self.set_mode(Mode::Image);
            }
            if self.image_mode {
                // Pictures: next / previous, zoom, rotation.
                let step = [(Key::ArrowRight, 1), (Key::PageDown, 1), (Key::Space, 1), (Key::ArrowLeft, -1), (Key::PageUp, -1), (Key::Backspace, -1), (Key::Home, i64::MIN), (Key::End, i64::MAX)];
                for (k, d) in step {
                    if i.consume_key(Modifiers::NONE, k) {
                        act = self.step_image(d);
                    }
                }
                if let Some(img) = &mut self.image {
                    if i.consume_key(Modifiers::NONE, Key::F) {
                        img.fit();
                    }
                    if i.consume_key(Modifiers::NONE, Key::Num0) {
                        img.actual_size();
                    }
                    if i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals) || i.consume_key(Modifiers::SHIFT, Key::Plus) {
                        img.zoom_by(1.25);
                    }
                    if i.consume_key(Modifiers::NONE, Key::Minus) {
                        img.zoom_by(0.8);
                    }
                    if i.consume_key(Modifiers::NONE, Key::R) {
                        img.rotate(true);
                    }
                    if i.consume_key(Modifiers::NONE, Key::L) || i.consume_key(Modifiers::SHIFT, Key::R) {
                        img.rotate(false);
                    }
                }
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::W) {
                self.text.wrap = !self.text.wrap;
            }
            if i.consume_key(Modifiers::NONE, Key::N) {
                self.text.numbers = !self.text.numbers;
            }
            if i.consume_key(Modifiers::NONE, Key::F) {
                let on = !self.text.follow;
                self.text.set_follow(on);
            }
        });
        if let Some(a) = act {
            acts.push(a);
        }
        if !typing && !self.image_mode {
            // The text area's own keys (scrolling, selection, copy).
            self.text.keys(ctx);
        }
    }

    fn edit_act(&mut self) -> Option<Act> {
        if self.archived {
            self.note = Some((tr!("A file in an archive cannot be edited."), std::time::Instant::now()));
            return None;
        }
        Some(Act::Edit(self.at, self.target.clone(), self.entry.clone()))
    }

    fn menu(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let keyed = |ui: &mut egui::Ui, label: String, key: &str| ui.add(egui::Button::new(label).shortcut_text(key)).clicked();
        let check = |ui: &mut egui::Ui, on: bool, label: String, key: &str| ui.add(egui::Button::selectable(on, label).shortcut_text(key)).clicked();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(trc!("menu", "File"), |ui| {
                if keyed(ui, tr!("Edit"), "F4") {
                    if let Some(a) = self.edit_act() {
                        acts.push(a);
                    }
                }
                if keyed(ui, tr!("Reload"), "Ctrl+R") {
                    self.text.reload();
                }
                if keyed(ui, tr!("Copy selection"), "Ctrl+C") {
                    self.text.copy();
                }
                ui.separator();
                if keyed(ui, tr!("Close"), "Esc") {
                    self.close = true;
                }
            });
            ui.menu_button(trc!("menu", "View"), |ui| {
                let m = self.mode();
                if check(ui, m == Mode::Text, tr!("Text"), "1") {
                    self.set_mode(Mode::Text);
                }
                if check(ui, m == Mode::Hex, tr!("Hexadecimal"), "2") {
                    self.set_mode(Mode::Hex);
                }
                if check(ui, m == Mode::Image, tr!("Image"), "3") {
                    self.set_mode(Mode::Image);
                }
                if self.image_mode {
                    ui.separator();
                    if let Some(img) = &mut self.image {
                        if check(ui, img.zoom.is_none(), tr!("Fit to window"), "F") {
                            img.fit();
                        }
                        if check(ui, img.zoom == Some(1.0), tr!("Actual size (100 %)"), "0") {
                            img.actual_size();
                        }
                        if keyed(ui, tr!("Zoom in"), "+") {
                            img.zoom_by(1.25);
                        }
                        if keyed(ui, tr!("Zoom out"), "−") {
                            img.zoom_by(0.8);
                        }
                        ui.separator();
                        if keyed(ui, tr!("Rotate right"), "R") {
                            img.rotate(true);
                        }
                        if keyed(ui, tr!("Rotate left"), "L") {
                            img.rotate(false);
                        }
                    }
                    ui.separator();
                    let many = self.images.len() > 1;
                    if ui.add_enabled(many, egui::Button::new(tr!("Previous image")).shortcut_text("PgUp")).clicked() {
                        if let Some(a) = self.step_image(-1) {
                            acts.push(a);
                        }
                    }
                    if ui.add_enabled(many, egui::Button::new(tr!("Next image")).shortcut_text("PgDn")).clicked() {
                        if let Some(a) = self.step_image(1) {
                            acts.push(a);
                        }
                    }
                } else {
                    ui.separator();
                    if check(ui, self.text.wrap, tr!("Wrap lines"), "W") {
                        self.text.wrap = !self.text.wrap;
                        let on = self.text.wrap;
                        acts.push(Act::SaveSettings(Box::new(move |s| s.viewer_nowrap = !on)));
                    }
                    if check(ui, self.text.numbers, tr!("Line numbers"), "N") {
                        self.text.numbers = !self.text.numbers;
                        let on = self.text.numbers;
                        acts.push(Act::SaveSettings(Box::new(move |s| s.viewer_numbers_off = !on)));
                    }
                    if check(ui, self.text.follow, tr!("Follow the end of the file"), "F") {
                        let on = !self.text.follow;
                        self.text.set_follow(on);
                    }
                    ui.separator();
                    if keyed(ui, tr!("Larger text"), "Ctrl++") {
                        self.text.font = (self.text.font + 1.0).min(32.0);
                        let f = self.text.font;
                        acts.push(Act::SaveSettings(Box::new(move |s| s.viewer_font = Some(f))));
                    }
                    if keyed(ui, tr!("Smaller text"), "Ctrl+−") {
                        self.text.font = (self.text.font - 1.0).max(8.0);
                        let f = self.text.font;
                        acts.push(Act::SaveSettings(Box::new(move |s| s.viewer_font = Some(f))));
                    }
                    ui.separator();
                    if keyed(ui, tr!("Find…"), "Ctrl+F") {
                        self.text.open_find();
                    }
                    if keyed(ui, tr!("Find next"), "F3") {
                        self.text.find_next(true);
                    }
                    if keyed(ui, tr!("Find previous"), "Shift+F3") {
                        self.text.find_next(false);
                    }
                    if keyed(ui, tr!("Go to line…"), "Ctrl+G") {
                        self.text.goto = Some(String::new());
                    }
                    if keyed(ui, tr!("Beginning"), "Ctrl+Home") {
                        self.text.jump(Jump::Offset(0));
                    }
                    if keyed(ui, tr!("End"), "Ctrl+End") {
                        self.text.jump(Jump::End);
                    }
                }
            });
            if self.image_mode {
                return;
            }
            ui.menu_button(tr!("Encoding"), |ui| {
                let cur = self.text.enc;
                for e in Enc::ALL {
                    if ui.add(egui::Button::selectable(cur == Some(e), e.name())).clicked() {
                        self.text.set_enc(e);
                    }
                }
            });
            ui.menu_button(tr!("Highlighting"), |ui| {
                let off = self.text.syntax.is_none() || self.text.hl_disabled;
                if ui.add(egui::Button::selectable(off, tr!("Off"))).clicked() {
                    self.text.set_syntax(None);
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for name in syntax::names() {
                        let on = !self.text.hl_disabled && self.text.syntax.as_deref() == Some(name.as_str());
                        if ui.add(egui::Button::selectable(on, name.as_str())).clicked() {
                            self.text.hl_disabled = false;
                            self.text.set_syntax(Some(name));
                        }
                    }
                });
            });
        });
    }

    /// The bar under the menu: what is shown, and the notices.
    fn head(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let (fill, fg) = crate::dialog::bar_colors(dark, false);
        egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(icons::icon(if self.image_mode { icons::FILE_IMAGE } else { icons::FILE_TEXT }, fg, 16.0));
                ui.label(RichText::new(&self.name).strong().color(fg).size(crate::fonts::fs(14.5)));
                ui.label(RichText::new(format!("— {} — {}", self.place, rolid_i18n::size(self.text.doc.size()))).color(fg)); // i18n-ok: separators
            });
        });
        // Notices.
        let warn_bg = if dark { Color32::from_rgb(0x4a, 0x3a, 0x10) } else { Color32::from_rgb(0xff, 0xf1, 0xc6) };
        let err_bg = if dark { Color32::from_rgb(0x5a, 0x22, 0x22) } else { Color32::from_rgb(0xf6, 0xd0, 0xd0) };
        let strip = |ui: &mut egui::Ui, bg: Color32, body: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| body(ui));
            });
        };
        match self.text.doc.health() {
            Health::Ok => {}
            Health::Lost(e) => {
                let state = self.link.as_ref().map(|l| rolid_i18n::t(&l.state().text())).unwrap_or_else(|| rolid_i18n::t(&e));
                strip(ui, warn_bg, &mut |ui| {
                    ui.label(icons::icon(icons::PLUG, ui.visuals().warn_fg_color, 15.0));
                    ui.label(tr!("{state}. The view continues when the connection is back.", state = state.as_str()));
                });
            }
            Health::Failed(e) => {
                let msg = rolid_i18n::t(&e);
                let mut again = false;
                strip(ui, err_bg, &mut |ui| {
                    ui.label(icons::icon(icons::WARNING, ui.visuals().error_fg_color, 15.0));
                    ui.label(tr!("The file cannot be read: {error}", error = msg.as_str()));
                    again = ui.button(tr!("Try again")).clicked();
                });
                if again {
                    self.text.reload();
                }
            }
        }
        if let Some(c) = self.text.doc.changed() {
            let mut reload = false;
            let mut follow = false;
            let what = if self.remote { tr!("The file has changed on the server ({size}).", size = rolid_i18n::size(c.size)) } else { tr!("The file has changed on the disk ({size}).", size = rolid_i18n::size(c.size)) };
            strip(ui, warn_bg, &mut |ui| {
                ui.label(icons::icon(icons::REFRESH, ui.visuals().warn_fg_color, 15.0));
                ui.label(what.as_str());
                reload = ui.button(tr!("Reload")).clicked();
                if c.size > self.text.doc.size() {
                    follow = ui.button(tr!("Follow the end")).clicked();
                }
            });
            if reload {
                self.text.reload();
            }
            if follow {
                self.text.set_follow(true);
            }
        }
        if self.image_mode {
            if let Some(e) = self.image.as_ref().and_then(|i| i.error()).filter(|e| !e.is_empty()) {
                let mut hex = false;
                strip(ui, err_bg, &mut |ui| {
                    ui.label(icons::icon(icons::WARNING, ui.visuals().error_fg_color, 15.0));
                    ui.label(tr!("The image cannot be shown: {error}", error = e.as_str()));
                    hex = ui.button(tr!("Show as hex")).clicked();
                });
                if hex {
                    self.set_mode(Mode::Hex);
                }
            }
        } else if self.text.binary && !self.text.hex {
            let mut hex = false;
            strip(ui, warn_bg, &mut |ui| {
                ui.label(tr!("The file looks like binary data."));
                hex = ui.button(tr!("Show as hex")).clicked();
            });
            if hex {
                self.set_mode(Mode::Hex);
            }
        } else if self.text.binary && self.text.hex {
            let mut as_text = false;
            strip(ui, warn_bg, &mut |ui| {
                ui.label(tr!("The file looks like binary data: shown as hex."));
                as_text = ui.button(tr!("Show as text")).clicked();
            });
            if as_text {
                self.set_mode(Mode::Text);
            }
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        if self.image_mode {
            self.image_status(ui);
            return;
        }
        let i = &self.text.info;
        ui.horizontal(|ui| {
            let weak = ui.visuals().weak_text_color();
            if let Some(line) = i.line {
                let n = |l: LineNo| match l {
                    LineNo::Exact(n) => rolid_i18n::num(n as i128),
                    LineNo::About(n) => tr!("about {n}", n = n),
                };
                let total = i.total.map(n).unwrap_or_else(|| "?".into());
                let r = ui.label(tr!("Line {line} of {total}", line = n(line), total = total));
                if i.limited {
                    r.on_hover_text(tr!("On a server lines are counted only in the first {size} (the whole file would have to be downloaded): numbers after that are estimates.", size = rolid_i18n::size(self.text.doc.opts().index_limit)));
                }
                ui.separator();
            }
            ui.label(tr!("Offset {offset}", offset = format!("0x{:X}", i.offset)));
            ui.separator();
            ui.label(format!("{} %", rolid_i18n::decimal(i.percent, 0)));
            if let Some(p) = i.indexing {
                ui.separator();
                ui.label(RichText::new(tr!("Counting lines… {pct} %", pct = rolid_i18n::decimal(p * 100.0, 0))).color(weak));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
            }
            if self.text.follow {
                ui.separator();
                ui.label(RichText::new(tr!("Following the end")).strong());
            }
            if let Some((note, at)) = &self.note {
                if at.elapsed().as_secs() < 4 {
                    ui.separator();
                    ui.label(note.as_str());
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(e) = self.text.enc {
                    ui.label(e.name());
                    ui.separator();
                }
                if !self.text.hex {
                    let limit = rolid_i18n::size(self.text.hl_limit);
                    let (hl, why) = match (self.text.hl_off(), &self.text.syntax, self.text.hl_disabled) {
                        (_, None, _) | (_, _, true) => (tr!("No highlighting"), None),
                        (Some(Off::TooBig), Some(_), _) => (tr!("Highlighting off"), Some(tr!("The file is larger than {size}: highlighting is off (Settings → Viewer (F3)).", size = limit))),
                        (Some(Off::LongLines), Some(_), _) => (tr!("Highlighting off"), Some(tr!("The file has extremely long lines: highlighting is off."))),
                        (Some(Off::Failed), Some(_), _) => (tr!("Highlighting off"), Some(tr!("The file could not be read for highlighting."))),
                        (None, Some(s), _) => (s.clone(), None),
                    };
                    match why {
                        Some(w) => {
                            ui.label(RichText::new(format!("ⓘ {hl}")).color(ui.visuals().warn_fg_color)).on_hover_text(w); // i18n-ok: info symbol
                        }
                        None => {
                            ui.label(hl);
                        }
                    }
                }
            });
        });
    }
}

impl Viewer {
    /// The status bar of the image mode: size, format, zoom, place in the folder.
    fn image_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let size = rolid_i18n::size(self.text.doc.size());
            if let Some(img) = &self.image {
                if let Some((w, h, format, frames, scaled, truncated)) = img.info {
                    ui.label(tr!("{w} × {h} px", w = w, h = h));
                    ui.separator();
                    ui.label(format);
                    ui.separator();
                    ui.label(size.as_str());
                    ui.separator();
                    let z = ui.label(format!("{} %", rolid_i18n::decimal(img.scale() as f64 * 100.0, 0))); // i18n-ok: a number
                    if img.zoom.is_none() {
                        z.on_hover_text(tr!("Fit to window"));
                    }
                    if frames > 1 {
                        ui.separator();
                        let f = ui.label(tr!("Animation: {n} frames", n = frames));
                        if truncated {
                            f.on_hover_text(tr!("Only the first frames are played (the animation needs too much memory)."));
                        }
                    }
                    if scaled {
                        ui.separator();
                        ui.label(RichText::new(tr!("shown smaller")).color(ui.visuals().warn_fg_color)).on_hover_text(tr!("The picture is larger than the graphics card can show at once: it is shown smaller."));
                    }
                    if img.turns != 0 {
                        ui.separator();
                        ui.label(tr!("Turned {deg}°", deg = img.turns as u32 * 90));
                    }
                } else if let Some((got, total)) = img.progress() {
                    ui.label(tr!("Reading the image… {got} of {total}", got = rolid_i18n::size(got), total = rolid_i18n::size(total)));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                } else {
                    ui.label(size.as_str());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(i) = self.image_index() {
                    ui.label(tr!("Image {n} of {count}", n = i + 1, count = self.images.len()));
                }
            });
        });
    }
}

/// Searching on the server: grep over the panel's SSH connection.
struct LinkExec(Arc<Link>);

#[async_trait::async_trait]
impl rolid_view::search::Exec for LinkExec {
    async fn run(&self, cmd: &str, cancel: &std::sync::atomic::AtomicBool, out: &mut (dyn for<'b> FnMut(&'b [u8]) -> bool + Send)) -> Result<(Option<u32>, String), String> {
        let Some((_, conn)) = self.0.current() else {
            return Err(rolid_i18n::t(&self.0.state().text()));
        };
        conn.exec_stream(&rolid_search::remote::guarded(cmd), cancel, out).await.map_err(|e| e.to_string())
    }
    async fn probe(&self, cmd: &str, cancel: &std::sync::atomic::AtomicBool, out: &mut (dyn for<'b> FnMut(&'b [u8]) -> bool + Send)) -> Result<(Option<u32>, String), String> {
        let Some((_, conn)) = self.0.current() else {
            return Err(rolid_i18n::t(&self.0.state().text()));
        };
        conn.exec_probe(&rolid_search::remote::guarded(cmd), cancel, out).await.map_err(|e| e.to_string())
    }
}
