//! "Browse…" in the copy and move dialogs: choose the target folder on the
//! disk or server of the target panel, in the app's own look.

use eframe::egui::{self, Key, Sense, Vec2};
use rolid_i18n::tr;

use super::{App, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::icons;
use crate::look::Role;

pub struct FolderPick {
    /// The panel whose disk or server is browsed.
    pub at: PaneRef,
    /// The folder shown.
    pub path: String,
    /// The path field (Enter goes there).
    pub edit: String,
    /// Subfolders of `path` (None while loading).
    pub dirs: Option<Result<Vec<String>, String>>,
    pub req: u64,
    /// Highlighted subfolder.
    pub cursor: Option<usize>,
    pub disk: String,
}

impl App {
    /// Open the picker on `start` (a folder of the panel `at`).
    pub(super) fn open_folder_pick(&mut self, at: PaneRef, start: String) {
        let Some(p) = self.pane(at) else { return };
        let disk = p.display_label();
        let req = self.folder_pick.as_ref().map_or(0, |f| f.req);
        self.folder_pick = Some(FolderPick { at, path: start.clone(), edit: start.clone(), dirs: None, req, cursor: None, disk });
        self.pick_go(start);
    }

    fn pick_go(&mut self, path: String) {
        let Some(fs) = self.folder_pick.as_ref().and_then(|f| self.pane(f.at)).map(|p| p.fs.clone()) else { return };
        let Some(f) = self.folder_pick.as_mut() else { return };
        f.req += 1;
        f.path = path.clone();
        f.edit = path.clone();
        f.dirs = None;
        f.cursor = None;
        let req = f.req;
        self.spawn(async move {
            let result = match fs.list(&path).await {
                Ok(list) => {
                    let mut d: Vec<String> = list.into_iter().filter(|e| e.is_dir_like()).map(|e| e.name).collect();
                    d.sort_by_key(|n| n.to_lowercase());
                    Ok(d)
                }
                Err(e) => Err(e.to_string()),
            };
            Msg::PickListed { req, path, result }
        });
    }

    pub(super) fn on_pick_listed(&mut self, req: u64, path: String, result: Result<Vec<String>, String>) {
        if let Some(f) = self.folder_pick.as_mut() {
            if f.req == req && f.path == path {
                f.dirs = Some(result);
            }
        }
    }

    /// The chosen folder goes into the dialog below.
    fn pick_chosen(&mut self, path: String) {
        self.folder_pick = None;
        if let Dialog::Move(f) = &self.dialog {
            let full = match (f.items.len() == 1, self.pane(f.to)) {
                (true, Some(p)) => p.fs.join(&path, &f.items[0].name),
                _ => path.clone(),
            };
            if let Dialog::Move(f) = &mut self.dialog {
                f.target = full;
                f.error = None;
            }
            return;
        }
        if let Dialog::Copy { dest, .. } = &mut self.dialog {
            *dest = path;
            self.recount_copy();
        }
    }

    pub(super) fn folder_pick_ui(&mut self, ctx: &egui::Context) {
        let Some(fs) = self.folder_pick.as_ref().and_then(|f| self.pane(f.at)).map(|p| p.fs.clone()) else {
            self.folder_pick = None;
            return;
        };
        let Some(f) = self.folder_pick.as_mut() else { return };
        let parent = fs.parent(&f.path);
        let mut go: Option<String> = None;
        let mut chosen: Option<String> = None;
        let mut cancel = false;
        let n = f.dirs.as_ref().and_then(|d| d.as_ref().ok()).map_or(0, |d| d.len());
        let field_focus = egui::Id::unique("pick-path");
        let typing = ctx.memory(|m| m.has_focus(field_focus));
        if !typing && n > 0 {
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    f.cursor = Some(f.cursor.map_or(0, |c| (c + 1).min(n - 1)));
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    f.cursor = Some(f.cursor.map_or(0, |c| c.saturating_sub(1)));
                }
            });
        }
        if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Backspace)) {
            go = parent.clone();
        }
        if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowRight)) {
            if let (Some(c), Some(Ok(d))) = (f.cursor, &f.dirs) {
                go = d.get(c).map(|name| fs.join(&f.path, name));
            }
        }
        dialog::window("folder-pick", tr!("Choose a folder")).icon(Role::Folder).min_width(500.0).show(ctx, |ui| {
            dialog::lead(ui, tr!("Folder on {disk}:", disk = f.disk.as_str()));
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut f.edit).id(field_focus).desired_width(dialog::width() - 40.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && f.edit != f.path {
                    go = Some(f.edit.trim().to_string());
                }
                let up = ui.add_enabled(parent.is_some(), egui::Button::new(icons::icon(icons::ARROW_UP, ui.visuals().text_color(), 16.0)));
                if up.on_hover_text(tr!("Up one level (Backspace)")).clicked() {
                    go = parent.clone();
                }
            });
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
            egui::Frame::NONE.stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(4.0).show(ui, |ui| {
                ui.set_min_width(dialog::width() - 10.0);
                egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match &f.dirs {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(tr!("Loading…"));
                            });
                        }
                        Some(Err(e)) => dialog::warn(ui, e.as_str(), true),
                        Some(Ok(d)) if d.is_empty() => dialog::note(ui, tr!("No subfolders.")),
                        Some(Ok(d)) => {
                            for (i, name) in d.iter().enumerate() {
                                let sel = f.cursor == Some(i);
                                let color = ui.visuals().text_color();
                                let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
                                if sel {
                                    ui.painter().rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                                } else if r.hovered() {
                                    ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                                }
                                ui.painter().text(rect.left_center() + Vec2::new(4.0, 0.0), egui::Align2::LEFT_CENTER, icons::FOLDER, icons::font(15.0), color);
                                ui.painter().text(
                                    rect.left_center() + Vec2::new(26.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    name,
                                    egui::TextStyle::Body.resolve(ui.style()),
                                    color,
                                );
                                if sel {
                                    r.scroll_to_me(None);
                                }
                                if r.clicked() {
                                    f.cursor = Some(i);
                                }
                                if r.double_clicked() {
                                    go = Some(fs.join(&f.path, name));
                                }
                            }
                        }
                    }
                });
            });
            dialog::note(ui, tr!("Double-click or → opens a folder, Backspace goes up."));
            let target = match (f.cursor, &f.dirs) {
                (Some(c), Some(Ok(d))) => d.get(c).map(|n| fs.join(&f.path, n)).unwrap_or_else(|| f.path.clone()),
                _ => f.path.clone(),
            };
            let b = dialog::buttons(ui, [
                dialog::btn(Role::Ok, tr!("Select")).default(!typing).tooltip(target.clone()),
                dialog::cancel(),
            ]);
            if b[0] {
                chosen = Some(target);
            }
            cancel |= b[1];
        });
        if cancel {
            self.folder_pick = None;
        } else if let Some(p) = chosen {
            self.pick_chosen(p);
        } else if let Some(p) = go {
            self.pick_go(p);
        }
    }
}
