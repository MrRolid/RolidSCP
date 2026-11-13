//! The toolbar under the menu, listing styles, folder history (Back / Forward),
//! marking a group by mask and the settings dialog (view and thumbnails).

use eframe::egui::{self, Color32, Vec2};
use rolid_config::{ThumbSize, ViewStyle};

use super::{Action, App, Dialog, PaneRef};
use crate::dialog;
use crate::icons;
use crate::look::Role;
use crate::pane::kinds::{parse_hex, to_hex, FileKind};

/// "Select group" / "Unselect group".
pub struct MarkForm {
    pub mark: bool,
    pub mask: String,
    pub dirs: bool,
    pub focus: bool,
}

/// Settings dialog being edited (applied on Save).
pub struct SettingsForm {
    pub toolbar: bool,
    pub brackets: bool,
    pub mask_dirs: bool,
    pub thumb_size: ThumbSize,
    pub remote: bool,
    pub production: bool,
    pub remote_max_kib: String,
    pub type_colors: bool,
    /// Color of each category (light theme), as edited.
    pub colors: Vec<(FileKind, Color32)>,
    /// Archive limits as typed: GB, items, ratio, MB, compression threads.
    pub arch: [String; 5],
    /// Font: from the system (fontconfig) instead of the built-in one.
    pub font_system: bool,
    pub font_family: String,
    /// Text size (whole points).
    pub font_size: f32,
    pub font_hinting: rolid_config::FontHinting,
    pub font_contrast: rolid_config::FontContrast,
    /// Language code, `None` = from the system.
    pub language: Option<String>,
    pub date_iso: bool,
    /// Languages to choose from (built in and the user's .po files).
    pub languages: Vec<String>,
    /// Viewer: syntax highlighting on.
    pub viewer_hl: bool,
    /// Viewer limits as typed: highlighting MB, images from a server MB.
    pub viewer: [String; 2],
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
        let dirs = self.settings.mask_dirs;
        self.dialog = Dialog::MarkGroup(Box::new(MarkForm { mark, mask, dirs, focus: true }));
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
        self.save_settings();
    }

    pub(super) fn open_settings(&mut self) {
        let s = &self.settings;
        self.dialog = Dialog::Settings(Box::new(SettingsForm {
            toolbar: !s.hide_toolbar,
            brackets: !s.dir_brackets_off,
            mask_dirs: s.mask_dirs,
            thumb_size: s.thumb_size.unwrap_or_default(),
            remote: s.thumbs_on_servers(),
            production: s.thumbs_production,
            remote_max_kib: (s.thumbs_remote_max() / 1024).to_string(),
            type_colors: !s.type_colors_off,
            colors: FileKind::COLORED
                .iter()
                .filter_map(|&k| {
                    let c = s.type_colors.get(k.key()).and_then(|h| parse_hex(h)).or(k.default_color(false))?;
                    Some((k, c))
                })
                .collect(),
            arch: [
                (s.unpack_max_bytes() / 1_000_000_000).to_string(),
                s.unpack_items().to_string(),
                s.unpack_ratio().to_string(),
                (s.remote_archive_max() / 1_000_000).to_string(),
                s.pack_threads().to_string(),
            ],
            font_system: s.font_system,
            font_family: s.font_family.clone().unwrap_or_default(),
            font_size: crate::fonts::clamp_size(s.font_size.unwrap_or(crate::fonts::DEFAULT_SIZE)),
            font_hinting: s.font_hinting.unwrap_or_default(),
            font_contrast: s.font_contrast.unwrap_or_default(),
            language: s.language.clone(),
            date_iso: s.date_iso,
            languages: rolid_i18n::available(Some(&Self::lang_dir())),
            viewer_hl: !s.viewer_highlight_off,
            viewer: [(s.viewer_highlight_max() >> 20).to_string(), (s.viewer_image_max() >> 20).to_string()],
        }));
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
                ui.add_enabled(on, btn).on_hover_text(tip).on_disabled_hover_text("").clicked()
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
        let thumb_mem = self.thumbs.memory();
        let font_desc = self.font_desc.clone();
        match &mut self.dialog {
            Dialog::MarkGroup(f) => {
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
                    ui.checkbox(&mut f.dirs, tr!("Include folders"))
                        .on_hover_text(tr!("The default is set in Settings → Selecting by mask includes folders too."));
                    let ok_on = !f.mask.trim().is_empty();
                    let text = if f.mark { tr!("Select") } else { tr!("Unselect") };
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, text).default(true).enabled(ok_on), dialog::cancel()]);
                    if b[0] && ok_on {
                        let form = MarkForm { mark: f.mark, mask: f.mask.clone(), dirs: f.dirs, focus: false };
                        act = Some(Box::new(move |a: &mut Self| a.apply_mark_group(form)));
                        close = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            Dialog::Settings(f) => {
                dialog::window("settings", tr!("Settings")).icon(Role::Edit).min_width(540.0).show(ctx, |ui| {
                    let max_h = (ctx.content_rect().height() - 180.0).max(200.0);
                    egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                        let heading = tr!("Language");
                        dialog::group(ui, &heading, |ui| {
                            dialog::form(ui, "language", |ui| {
                                ui.label(tr!("Language:"));
                                ui.horizontal(|ui| {
                                    let auto = tr!("Automatic (from the system)");
                                    let name = |code: &str| rolid_i18n::language_name(code);
                                    let shown = f.language.as_deref().map(name).unwrap_or_else(|| auto.clone());
                                    egui::ComboBox::from_id_salt("language-combo").selected_text(shown).height(400.0).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut f.language, None, auto.as_str());
                                        for code in &f.languages {
                                            ui.selectable_value(&mut f.language, Some(code.clone()), name(code));
                                        }
                                    });
                                    dialog::help(ui, tr!("The language changes at once. Your own translations: {dir}/<language>.po", dir = Self::lang_dir()));
                                });
                                ui.end_row();
                                ui.label(tr!("Date format:"));
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut f.date_iso, false, tr!("By language"));
                                    ui.radio_value(&mut f.date_iso, true, tr!("ISO (2026-11-08 21:08)"));
                                });
                                ui.end_row();
                            });
                        });
                        let heading = tr!("Font");
                        dialog::group(ui, &heading, |ui| {
                            dialog::form(ui, "font-render", |ui| {
                                ui.label(tr!("Font:"));
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut f.font_system, false, tr!("Built-in ({font})", font = crate::fonts::NOTO_NAME));
                                    ui.radio_value(&mut f.font_system, true, tr!("System (fontconfig)"));
                                });
                                ui.end_row();
                                ui.add_enabled(f.font_system, egui::Label::new(tr!("Font name:")));
                                ui.add_enabled(
                                    f.font_system,
                                    egui::TextEdit::singleline(&mut f.font_family)
                                        .hint_text(tr!("empty = default sans-serif"))
                                        .desired_width(200.0),
                                );
                                ui.end_row();
                                ui.label(tr!("Text size in the list:"));
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("font-size")
                                        .selected_text(format!("{}", f.font_size))
                                        .width(50.0)
                                        .show_ui(ui, |ui| {
                                            for v in crate::fonts::MIN_SIZE as u32..=crate::fonts::MAX_SIZE as u32 {
                                                ui.selectable_value(&mut f.font_size, v as f32, v.to_string());
                                            }
                                        });
                                    ui.label(tr!("points (default {size})", size = crate::fonts::DEFAULT_SIZE as u32));
                                });
                                ui.end_row();
                                ui.label(tr!("Hinting (sharpness):"));
                                ui.horizontal(|ui| {
                                    for h in rolid_config::FontHinting::ALL {
                                        ui.radio_value(&mut f.font_hinting, h, rolid_i18n::tc_str("hinting", h.label()));
                                    }
                                    dialog::help(
                                        ui,
                                        tr!(
                                            "How strongly the letter shapes snap to the pixel grid. Strong (default) = vertical \
                                             strokes on whole pixels, the sharpest; medium = following the font's instructions \
                                             (vertical only); off = shapes exactly as the font was designed, but softer."
                                        ),
                                    );
                                });
                                ui.end_row();
                                ui.label(tr!("Font contrast:"));
                                ui.horizontal(|ui| {
                                    for c in rolid_config::FontContrast::ALL {
                                        ui.radio_value(&mut f.font_contrast, c, rolid_i18n::tc_str("contrast", c.label()));
                                    }
                                    dialog::help(
                                        ui,
                                        tr!(
                                            "How dark the edges of the letters are drawn (how light in the dark theme). \
                                             Higher (default) balances the thinner strokes of strong hinting; highest = the font looks bolder."
                                        ),
                                    );
                                });
                                ui.end_row();
                            });
                            dialog::note(ui, tr!("Now: {font}", font = font_desc.as_str()));
                        });
                        let heading = tr!("View");
                        dialog::group(ui, &heading, |ui| {
                            ui.checkbox(&mut f.toolbar, tr!("Toolbar under the menu"));
                            ui.checkbox(&mut f.brackets, tr!("Folders in [square brackets] (full and brief listing)"));
                            ui.checkbox(&mut f.mask_dirs, tr!("Selecting by mask includes folders too (also Invert selection)"));
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut f.type_colors, tr!("Name colors by file type"));
                                dialog::help(ui, tr!("The colors apply to the light theme; the dark theme uses lighter shades."));
                            });
                            ui.add_enabled_ui(f.type_colors, |ui| {
                                egui::Grid::new("type-colors").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                                    for (i, (k, c)) in f.colors.iter_mut().enumerate() {
                                        ui.horizontal(|ui| {
                                            ui.color_edit_button_srgba(c);
                                            ui.label(icons::icon(k.glyph(), *c, 16.0));
                                            ui.label(k.label());
                                        });
                                        if i % 2 == 1 {
                                            ui.end_row();
                                        }
                                    }
                                });
                                if ui.small_button(tr!("Default colors")).clicked() {
                                    for (k, c) in f.colors.iter_mut() {
                                        if let Some(d) = k.default_color(false) {
                                            *c = d;
                                        }
                                    }
                                }
                            });
                        });
                        let heading = tr!("Thumbnails");
                        dialog::group(ui, &heading, |ui| {
                            dialog::form(ui, "thumb-size", |ui| {
                                ui.label(trc!("setting", "Size:"));
                                ui.horizontal(|ui| {
                                    for s in ThumbSize::ALL {
                                        ui.radio_value(&mut f.thumb_size, s, rolid_i18n::tc_str("thumbnail", s.label()));
                                    }
                                });
                                ui.end_row();
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut f.remote, tr!("Image previews on servers too"));
                                dialog::help(
                                    ui,
                                    tr!("At most 2 previews are downloaded from a server at the same time; images larger than {px} px are not processed.", px = 10_000),
                                );
                            });
                            ui.add_enabled_ui(f.remote, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(tr!("Download only files up to"));
                                    ui.add(egui::TextEdit::singleline(&mut f.remote_max_kib).desired_width(70.0));
                                    ui.label(trc!("unit", "KiB"));
                                });
                                if parse_max_kib(&f.remote_max_kib).is_none() {
                                    dialog::warn(ui, tr!("Enter a number from {lo} to {hi}.", lo = 16, hi = 1_048_576), true);
                                }
                                ui.checkbox(&mut f.production, tr!("Also on production servers"));
                            });
                            dialog::note(ui, tr!("Previews in memory: {size} (at most 48 MB).", size = rolid_i18n::size(thumb_mem as u64)));
                        });
                        let heading = tr!("Viewer (F3)");
                        dialog::group(ui, &heading, |ui| {
                            ui.checkbox(&mut f.viewer_hl, tr!("Syntax highlighting"));
                            dialog::form(ui, "viewer-limits", |ui| {
                                for (i, label) in [tr!("Highlight files up to"), tr!("Download images from a server up to")].into_iter().enumerate() {
                                    ui.label(label);
                                    ui.horizontal(|ui| {
                                        ui.add_sized([90.0, 20.0], egui::TextEdit::singleline(&mut f.viewer[i]));
                                        if parse_viewer(i, &f.viewer[i]).is_some() {
                                            ui.label(trc!("unit", "MB"));
                                        } else {
                                            let (lo, hi) = VIEWER_RANGES[i];
                                            dialog::warn(ui, tr!("number {lo} to {hi}", lo = lo, hi = hi), true);
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                        });
                        let heading = tr!("Archives");
                        dialog::group(ui, &heading, |ui| {
                            let threads_hint = tr!("(default {threads} = number of cores − 1)", threads = rolid_config::Settings::default_pack_threads());
                            dialog::form(ui, "arch-limits", |ui| {
                                for (i, (label, unit)) in [
                                    (tr!("Extract at most"), tr!("GB (and always leave 1 GB free)")),
                                    (tr!("At most items in an archive"), String::new()),
                                    (tr!("Ask at a compression ratio above"), ": 1".to_string()),
                                    (tr!("Download and open a remote archive up to"), trc!("unit", "MB")),
                                    (tr!("Number of compression threads"), threads_hint.clone()),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    ui.label(label);
                                    ui.horizontal(|ui| {
                                        ui.add_sized([90.0, 20.0], egui::TextEdit::singleline(&mut f.arch[i]));
                                        if parse_arch(i, &f.arch[i]).is_some() {
                                            ui.label(unit);
                                        } else {
                                            let (lo, hi) = ARCH_RANGES[i];
                                            dialog::warn(ui, tr!("number {lo} to {hi}", lo = lo, hi = hi), true);
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                        });
                    });
                    let arch_ok = (0..5).all(|i| parse_arch(i, &f.arch[i]).is_some());
                    let viewer_ok = (0..2).all(|i| parse_viewer(i, &f.viewer[i]).is_some());
                    let valid = (!f.remote || parse_max_kib(&f.remote_max_kib).is_some()) && arch_ok && viewer_ok;
                    let b = dialog::buttons(ui, [dialog::btn(Role::Save, tr!("Save")).default(true).enabled(valid), dialog::cancel()]);
                    if b[0] && valid {
                        let toolbar = f.toolbar;
                        let (brackets, mask_dirs) = (f.brackets, f.mask_dirs);
                        let size = f.thumb_size;
                        let remote = f.remote;
                        let production = f.production;
                        let max = parse_max_kib(&f.remote_max_kib);
                        let type_colors = f.type_colors;
                        let font_system = f.font_system;
                        let family = f.font_family.trim().to_string();
                        let font_size = crate::fonts::clamp_size(f.font_size);
                        let (font_hinting, font_contrast) = (f.font_hinting, f.font_contrast);
                        let (language, date_iso) = (f.language.clone(), f.date_iso);
                        let arch: Vec<u64> = (0..5).map(|i| parse_arch(i, &f.arch[i]).unwrap_or(0)).collect();
                        let viewer_hl = f.viewer_hl;
                        let viewer: Vec<u64> = (0..2).map(|i| parse_viewer(i, &f.viewer[i]).unwrap_or(0)).collect();
                        let custom: std::collections::BTreeMap<String, String> = f
                            .colors
                            .iter()
                            .filter(|(k, c)| k.default_color(false) != Some(*c))
                            .map(|(k, c)| (k.key().to_string(), to_hex(*c)))
                            .collect();
                        act = Some(Box::new(move |a: &mut Self| {
                            let s = &mut a.settings;
                            s.hide_toolbar = !toolbar;
                            s.dir_brackets_off = !brackets;
                            s.mask_dirs = mask_dirs;
                            s.thumb_size = (size != ThumbSize::default()).then_some(size);
                            s.thumbs_remote = (!remote).then_some(false);
                            s.thumbs_production = production;
                            s.type_colors_off = !type_colors;
                            s.type_colors = custom;
                            if let Some(m) = max {
                                s.thumbs_remote_max_kib = (m != 2048).then_some(m);
                            }
                            s.unpack_max_gb = (arch[0] != 20).then_some(arch[0]);
                            s.unpack_max_items = (arch[1] != 1_000_000).then_some(arch[1]);
                            s.unpack_max_ratio = (arch[2] != 1000).then_some(arch[2]);
                            s.remote_archive_max_mb = (arch[3] != 500).then_some(arch[3]);
                            s.pack_threads = (arch[4] != rolid_config::Settings::default_pack_threads()).then_some(arch[4]);
                            s.viewer_highlight_off = !viewer_hl;
                            s.viewer_highlight_max_mb = (viewer[0] != 20).then_some(viewer[0]);
                            s.viewer_image_max_mb = (viewer[1] != 50).then_some(viewer[1]);
                            let before = crate::fonts::Spec::from_settings(s);
                            s.font_system = font_system;
                            s.font_family = (!family.is_empty()).then_some(family);
                            s.font_size = (font_size != crate::fonts::DEFAULT_SIZE).then_some(font_size);
                            s.font_hinting = (font_hinting != Default::default()).then_some(font_hinting);
                            s.font_contrast = (font_contrast != Default::default()).then_some(font_contrast);
                            let lang_changed = s.language != language || s.date_iso != date_iso;
                            s.language = language;
                            s.date_iso = date_iso;
                            if crate::fonts::Spec::from_settings(&a.settings) != before {
                                a.apply_font();
                            }
                            if lang_changed {
                                a.apply_language();
                            }
                            a.save_settings();
                        }));
                        close = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            _ => {}
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
