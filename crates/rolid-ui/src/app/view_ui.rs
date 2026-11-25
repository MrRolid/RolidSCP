//! The toolbar under the menu, listing styles, folder history (Back / Forward),
//! marking a group by mask.

use eframe::egui::{self, Color32, Vec2};
use rolid_config::ViewStyle;

use super::{Action, App, Dialog, PaneRef};
use crate::dialog;
use crate::icons;
use crate::look::Role;

/// "Select group" / "Unselect group".
pub struct MarkForm {
    pub mark: bool,
    pub mask: String,
    pub dirs: bool,
    pub focus: bool,
    /// Opened on a production server: "Include folders" starts off and is
    /// not remembered.
    pub production: bool,
}

/// Ranges of the viewer limits (MB).
pub const VIEWER_RANGES: [(u64, u64); 2] = [(1, 1000), (1, 4000)];

/// The settings key of a panel's column widths: by side and view style.
pub fn columns_key(side: usize, style: ViewStyle) -> String {
    let style = match style {
        ViewStyle::Full => "full",
        ViewStyle::Brief => "brief",
        ViewStyle::Thumbs => "thumbs",
    };
    rolid_config::columns_key(side == 1, style)
}

pub fn parse_viewer(i: usize, s: &str) -> Option<u64> {
    let (lo, hi) = VIEWER_RANGES[i];
    s.trim().replace(' ', "").parse::<u64>().ok().filter(|v| (lo..=hi).contains(v))
}

/// Ranges of the archive limits (GB written, items, ratio, remote archive MB, threads).
pub const ARCH_RANGES: [(u64, u64); 5] = [(1, 100_000), (1, 100_000_000), (10, 1_000_000), (1, 100_000), (1, 256)];

pub fn parse_arch(i: usize, s: &str) -> Option<u64> {
    let (lo, hi) = ARCH_RANGES[i];
    s.trim().replace(' ', "").parse::<u64>().ok().filter(|v| (lo..=hi).contains(v))
}

/// Remote size limit typed in the settings dialog (KiB, 16 – 1 048 576).
pub fn parse_max_kib(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok().filter(|v| (16..=1_048_576).contains(v))
}

impl ViewStyleExt for ViewStyle {
    fn label(self) -> String {
        match self {
            ViewStyle::Brief => tr!("Brief listing"),
            ViewStyle::Full => tr!("Full listing"),
            ViewStyle::Thumbs => tr!("Thumbnails"),
        }
    }
    fn shortcut(self) -> &'static str {
        match self {
            ViewStyle::Brief => "Ctrl+F1",
            ViewStyle::Full => "Ctrl+F2",
            ViewStyle::Thumbs => "Ctrl+Shift+F1",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            ViewStyle::Brief => icons::VIEW_BRIEF,
            ViewStyle::Full => icons::VIEW_FULL,
            ViewStyle::Thumbs => icons::VIEW_THUMBS,
        }
    }
}

pub trait ViewStyleExt {
    /// Translated name of the style.
    fn label(self) -> String;
    fn shortcut(self) -> &'static str;
    fn glyph(self) -> &'static str;
}

pub const STYLES: [ViewStyle; 3] = [ViewStyle::Brief, ViewStyle::Full, ViewStyle::Thumbs];

/// Shortcuts of marking (Num−/Num* collide with the keyboard: see F1).
pub const MARK_KEYS: &str = "Num+ / Alt+=";
pub const UNMARK_KEYS: &str = "Alt+−";
pub const INVERT_KEYS: &str = "Alt+I";

impl App {
    // ----------------------------------------------------------- actions

    pub(super) fn set_view_style(&mut self, style: ViewStyle) {
        let at = self.active_ref();
        if let Some(p) = self.pane_mut(at) {
            p.set_style(style);
        }
    }

    pub(super) fn active_style(&self) -> ViewStyle {
        self.pane(self.active_ref()).map(|p| p.style).unwrap_or_default()
    }

    /// Back (`forward` = false) / Forward in the active panel.
    pub(super) fn history_go(&mut self, forward: bool) {
        let at = self.active_ref();
        let Some(p) = self.pane_mut(at) else { return };
        if p.search.is_some() {
            return;
        }
        let cur = p.path.clone();
        let to = if forward { p.history.go_forward(&cur) } else { p.history.go_back(&cur) };
        if let Some(to) = to {
            self.load(at, to, None);
        }
    }

    pub(super) fn ask_mark_group(&mut self, mark: bool) {
        let mask = self.settings.mark_masks.first().cloned().unwrap_or_else(|| "*".into());
        let production = self.pane(self.active_ref()).and_then(|p| p.remote.as_ref()).is_some_and(|r| r.production);
        let dirs = self.settings.mark_group_dirs(mark, production);
        self.dialog = Dialog::MarkGroup(Box::new(MarkForm { mark, mask, dirs, focus: true, production }));
    }

    pub(super) fn invert_selection(&mut self) {
        let at = self.active_ref();
        let dirs = self.settings.mask_dirs;
        if let Some(p) = self.pane_mut(at) {
            p.invert_selection(dirs);
        }
    }

    fn apply_mark_group(&mut self, f: MarkForm) {
        let mask = f.mask.trim().to_string();
        if mask.is_empty() {
            return;
        }
        let at = self.active_ref();
        if let Some(p) = self.pane_mut(at) {
            p.mark_group(&mask, f.mark, f.dirs);
        }
        self.settings.remember_mask(&mask);
        self.settings.remember_mark_group_dirs(f.mark, f.dirs, f.production);
        self.save_settings();
    }

    /// A column separator dragged (saved when the drag ends) or double clicked.
    pub(super) fn set_column_width(&mut self, at: PaneRef, col: crate::pane::columns::Col, width: Option<f32>, done: bool) {
        let Some(style) = self.pane(at).map(|p| p.style) else { return };
        let key = columns_key(at.side, style);
        let w = self.settings.columns.entry(key.clone()).or_default();
        col.set(w, width);
        if w.is_default() {
            self.settings.columns.remove(&key);
        }
        if done {
            self.save_settings();
        }
    }

    /// View → Reset column widths.
    pub(super) fn reset_column_widths(&mut self) {
        self.settings.columns.clear();
        self.save_settings();
    }

    pub(super) fn toggle_ext_column(&mut self) {
        self.settings.ext_column = !self.settings.ext_column;
        self.save_settings();
    }

    pub(super) fn toggle_brackets(&mut self) {
        self.settings.dir_brackets_off = !self.settings.dir_brackets_off;
        self.save_settings();
    }

    pub(super) fn toggle_seconds(&mut self) {
        self.settings.seconds = !self.settings.seconds;
        self.save_settings();
    }

    pub(super) fn toggle_toolbar(&mut self) {
        self.settings.hide_toolbar = !self.settings.hide_toolbar;
        self.save_settings();
    }

    pub(super) fn toggle_nav_bar(&mut self) {
        self.settings.hide_nav_bar = !self.settings.hide_nav_bar;
        self.save_settings();
    }

    /// Alt+↓: the drop-down tree of the active panel's path.
    pub(super) fn open_path_tree(&mut self) {
        let at = self.active_ref();
        let mut ev = Vec::new();
        if let Some(p) = self.pane_mut(at) {
            p.open_tree(&mut ev);
        }
        if ev.iter().any(|e| matches!(e, crate::pane::PaneEvent::WantMounts)) {
            self.read_mounts(at);
        }
    }

    /// The disks of this computer for the drop-down (in the background:
    /// a network mount may answer slowly).
    pub(super) fn read_mounts(&mut self, at: super::PaneRef) {
        self.spawn(async move {
            let text = tokio::task::spawn_blocking(|| std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default()).await.unwrap_or_default();
            let mounts = crate::pane::navbar::user_mounts(&rolid_disk::mounts::parse(&text));
            let fs = rolid_local::LocalFs;
            let mut rows = Vec::new();
            for m in mounts {
                let total = match tokio::time::timeout(std::time::Duration::from_secs(1), rolid_core::Vfs::space(&fs, &m.point)).await {
                    Ok(Ok(Some(s))) => Some(s.total),
                    _ => None,
                };
                rows.push(crate::pane::navbar::MountRow { point: m.point, fstype: m.fstype, total });
            }
            super::Msg::Then {
                job: None,
                f: Box::new(move |a: &mut super::App| {
                    if let Some(p) = a.pane_mut(at) {
                        p.mounts = Some(rows);
                    }
                }),
            }
        });
    }

    // ----------------------------------------------------------- toolbar

    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let green = Role::Ok.icon(dark).1;
        let blue = Role::Connect.icon(dark).1;
        let red = Role::Delete.icon(dark).1;
        let orange = Role::Overwrite.icon(dark).1;
        let plain = ui.visuals().text_color();
        let style = self.active_style();
        let (can_back, can_fwd) = self
            .pane(self.active_ref())
            .map(|p| (p.history.can_back() && p.search.is_none(), p.history.can_forward() && p.search.is_none()))
            .unwrap_or((false, false));
        let mut act: Option<Action> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let b = |ui: &mut egui::Ui, glyph: &str, color: Color32, tip: &str, key: &str, on: bool, sel: bool| -> bool {
                let tip = if key.is_empty() { tip.to_string() } else { format!("{tip} ({key})") };
                let color = if on { color } else { color.gamma_multiply(0.35) };
                let btn = egui::Button::new(icons::icon(glyph, color, 19.0))
                    .min_size(Vec2::splat(30.0))
                    .selected(sel)
                    .frame_when_inactive(sel);
                let r = ui.add_enabled(on, btn);
                // A screen reader says the tooltip, not the icon's character.
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, on, sel, &tip));
                r.on_hover_text(&tip).on_disabled_hover_text("").clicked()
            };
            let sep = |ui: &mut egui::Ui| {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            };
            if b(ui, icons::REFRESH, green, &tr!("Refresh"), "Ctrl+R", true, false) {
                act = Some(Box::new(|a: &mut Self| a.refresh_or_rerun()));
            }
            if b(ui, icons::GEAR, plain, &tr!("Settings"), "", true, false) {
                act = Some(Box::new(|a: &mut Self| a.open_settings()));
            }
            sep(ui);
            for s in STYLES {
                if b(ui, s.glyph(), blue, &s.label(), s.shortcut(), true, style == s) {
                    act = Some(Box::new(move |a: &mut Self| a.set_view_style(s)));
                }
            }
            sep(ui);
            if b(ui, icons::BACK, blue, &tr!("Back"), "Alt+←", can_back, false) {
                act = Some(Box::new(|a: &mut Self| a.history_go(false)));
            }
            if b(ui, icons::FORWARD, blue, &tr!("Forward"), "Alt+→", can_fwd, false) {
                act = Some(Box::new(|a: &mut Self| a.history_go(true)));
            }
            sep(ui);
            if b(ui, icons::SELECT_GROUP, green, &tr!("Select group…"), MARK_KEYS, true, false) {
                act = Some(Box::new(|a: &mut Self| a.ask_mark_group(true)));
            }
            if b(ui, icons::UNSELECT_GROUP, red, &tr!("Unselect group…"), UNMARK_KEYS, true, false) {
                act = Some(Box::new(|a: &mut Self| a.ask_mark_group(false)));
            }
            if b(ui, icons::INVERT_SELECTION, orange, &tr!("Invert selection"), INVERT_KEYS, true, false) {
                act = Some(Box::new(|a: &mut Self| a.invert_selection()));
            }
            sep(ui);
            if b(ui, icons::SERVERS, blue, &tr!("Servers (server manager)"), "Ctrl+O", true, false) {
                act = Some(Box::new(|a: &mut Self| a.open_connect()));
            }
            sep(ui);
            if b(ui, icons::SEARCH, blue, &tr!("Find files"), "Ctrl+F", true, false) {
                act = Some(Box::new(|a: &mut Self| a.open_search_dialog()));
            }
            sep(ui);
            if b(ui, icons::PACKAGE, Role::Pack.icon(dark).1, &tr!("Pack"), "Alt+F5", true, false) {
                act = Some(Box::new(|a: &mut Self| a.ask_pack()));
            }
            if b(ui, icons::UNPACK, Role::Unpack.icon(dark).1, &tr!("Extract"), "Alt+F9", true, false) {
                act = Some(Box::new(|a: &mut Self| a.ask_archive(super::unpack_ui::Purpose::Extract)));
            }
            if b(ui, icons::FOLDER_SIZE, plain, &tr!("Calculate the size of all folders"), "Alt+Shift+Enter", true, false) {
                act = Some(Box::new(|a: &mut Self| a.size_all_dirs()));
            }
            sep(ui);
            if b(ui, icons::CLIPBOARD_TEXT, plain, &tr!("Copy paths to clipboard"), "Ctrl+Shift+P", true, false) {
                act = Some(Box::new(|a: &mut Self| {
                    a.copy_to_clipboard(a.active_ref(), super::ctx_ui::ClipKind::Paths)
                }));
            }
        });
        if let Some(a) = act {
            a(self);
        }
    }

    // ----------------------------------------------------------- dialogs

    pub(super) fn view_dialog_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut act: Option<Action> = None;
        let masks = self.settings.mark_masks.clone();
        if let Dialog::MarkGroup(f) = &mut self.dialog {
            let title = if f.mark { tr!("Select group") } else { tr!("Unselect group") };
            dialog::window("mark-group", title).icon(Role::Ok).show(ctx, |ui| {
                dialog::form(ui, "mark-form", |ui| {
                    ui.label(tr!("Mask:"));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut f.mask).desired_width(260.0));
                        if f.focus {
                            r.request_focus();
                            f.focus = false;
                        }
                        ui.add_enabled_ui(!masks.is_empty(), |ui| {
                            egui::ComboBox::from_id_salt("mark-history")
                                .selected_text(tr!("Recent"))
                                .show_ui(ui, |ui| {
                                    for m in &masks {
                                        if ui.selectable_label(*m == f.mask, m).clicked() {
                                            f.mask = m.clone();
                                        }
                                    }
                                });
                        });
                        dialog::help(
                            ui,
                            format!(
                                "{}\n{}",
                                tr!("Separate several masks with a semicolon, e.g. *.jpg;*.png."),
                                tr!("* = any characters, ? = one character; case does not matter.")
                            ),
                        );
                    });
                    ui.end_row();
                });
                let tip = if f.production {
                    tr!("On a production server this always starts off; your choice here is not remembered.")
                } else {
                    tr!("Remembered for the next time. The default is set in Settings → Selecting by mask includes folders too.")
                };
                ui.checkbox(&mut f.dirs, tr!("Include folders")).on_hover_text(tip);
                let ok_on = !f.mask.trim().is_empty();
                let text = if f.mark { tr!("Select") } else { tr!("Unselect") };
                let b = dialog::buttons(ui, [dialog::btn(Role::Ok, text).default(true).enabled(ok_on), dialog::cancel()]);
                if b[0] && ok_on {
                    let form = MarkForm { mark: f.mark, mask: f.mask.clone(), dirs: f.dirs, focus: false, production: f.production };
                    act = Some(Box::new(move |a: &mut Self| a.apply_mark_group(form)));
                    close = true;
                }
                if b[1] {
                    close = true;
                }
            });
        }
        if close {
            self.dialog = Dialog::None;
        }
        if let Some(a) = act {
            a(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_limit_is_validated() {
        assert_eq!(parse_max_kib(" 2048 "), Some(2048));
        assert_eq!(parse_max_kib("8"), None);
        assert_eq!(parse_max_kib("abc"), None);
        assert_eq!(parse_max_kib("2000000"), None);
    }
}
