//! Settings in a window of their own: a tree of sections on the left, the
//! settings of the chosen section on the right and a filter over all of
//! them at the top.
//!
//! The window edits a form; Save (or Apply) writes what the form changed
//! into the settings with a three-way merge against what they were when
//! the window opened, so that a change made meanwhile elsewhere (a menu, the
//! viewer, another running copy) is not undone by values the user did not
//! touch here.

use std::time::Instant;

use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use rolid_config::{ConflictPreset, Settings, ThumbSize};
use rolid_i18n::rolid_text::{n, nc};
use rolid_i18n::{t_str, tc_str};

use super::view_ui::{parse_arch, parse_max_kib, parse_viewer, ARCH_RANGES, VIEWER_RANGES};
use super::App;
use crate::dialog;
use crate::icons;
use crate::look::Role;
use crate::pane::kinds::{parse_hex, to_hex, FileKind};

/// A leaf of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sec {
    Language,
    Font,
    Panels,
    Colors,
    Thumbs,
    Copying,
    Archives,
    Dragging,
    Viewer,
    Editor,
    Servers,
    Passwords,
}

impl Sec {
    pub const ALL: [Sec; 12] = [
        Sec::Language,
        Sec::Font,
        Sec::Panels,
        Sec::Colors,
        Sec::Thumbs,
        Sec::Copying,
        Sec::Archives,
        Sec::Dragging,
        Sec::Viewer,
        Sec::Editor,
        Sec::Servers,
        Sec::Passwords,
    ];

    /// The English name (translated when drawn).
    fn name(self) -> &'static str {
        match self {
            Sec::Language => n!("Language and dates"),
            Sec::Font => nc!("settings", "Font"),
            Sec::Panels => nc!("settings", "Panels"),
            Sec::Colors => n!("Colors by file type"),
            Sec::Thumbs => nc!("settings", "Thumbnails"),
            Sec::Copying => n!("Copying and moving"),
            Sec::Archives => nc!("settings", "Archives"),
            Sec::Dragging => nc!("settings", "Drag and drop"),
            Sec::Viewer => n!("Viewer (F3)"),
            Sec::Editor => n!("External editor (F4)"),
            Sec::Servers => n!("Server connections"),
            Sec::Passwords => n!("Saved passwords"),
        }
    }

    fn ctx(self) -> Option<&'static str> {
        matches!(self, Sec::Font | Sec::Panels | Sec::Thumbs | Sec::Archives | Sec::Dragging).then_some("settings")
    }

    pub fn label(self) -> String {
        match self.ctx() {
            Some(c) => tc_str(c, self.name()).to_string(),
            None => t_str(self.name()).to_string(),
        }
    }
}

/// The branches of the tree.
const GROUPS: &[(&str, &[Sec])] = &[
    (n!("Appearance"), &[Sec::Language, Sec::Font, Sec::Panels, Sec::Colors, Sec::Thumbs]),
    (n!("Files"), &[Sec::Copying, Sec::Archives, Sec::Dragging]),
    (n!("Tools"), &[Sec::Viewer, Sec::Editor]),
    (n!("Servers and security"), &[Sec::Servers, Sec::Passwords]),
];

/// One setting: what the filter searches in, and what [`draw_row`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowId {
    Language,
    DateFormat,
    FontKind,
    FontName,
    FontSize,
    Hinting,
    Contrast,
    Toolbar,
    NavBar,
    Brackets,
    ExtColumn,
    Seconds,
    MaskDirs,
    Refresh,
    TypeColors,
    ThumbSize,
    ThumbRemote,
    ThumbMax,
    ThumbProduction,
    SpeedLimit,
    DropPreset,
    CopyPreset,
    Arch(usize),
    Highlight,
    ViewerMax(usize),
    EditorCmd,
    ClockWarning,
    VaultLock,
    X11OnWayland,
}

/// Every setting: (section, row, English label, more words to search in).
/// The label is searched in English and in the language of the window.
const ROWS: &[(Sec, RowId, &str, &str)] = &[
    (Sec::Language, RowId::Language, n!("Language:"), ""),
    (Sec::Language, RowId::DateFormat, n!("Date format:"), n!("ISO (2026-11-08 21:08)")),
    (Sec::Font, RowId::FontKind, n!("Font:"), n!("System (fontconfig)")),
    (Sec::Font, RowId::FontName, n!("Font name:"), ""),
    (Sec::Font, RowId::FontSize, n!("Text size in the list:"), ""),
    (Sec::Font, RowId::Hinting, n!("Hinting (sharpness):"), ""),
    (Sec::Font, RowId::Contrast, n!("Font contrast:"), ""),
    (Sec::Panels, RowId::Toolbar, n!("Toolbar under the menu"), ""),
    (Sec::Panels, RowId::NavBar, n!("Bar above the panels (path tree, navigation)"), ""),
    (Sec::Panels, RowId::Brackets, n!("Folders in [square brackets] (full and brief listing)"), ""),
    (Sec::Panels, RowId::ExtColumn, n!("Extension column"), ""),
    (Sec::Panels, RowId::Seconds, n!("Modified time with seconds"), ""),
    (Sec::Panels, RowId::MaskDirs, n!("Selecting by mask includes folders too (also Invert selection)"), ""),
    (Sec::Panels, RowId::Refresh, n!("Automatic refresh:"), n!("Server folders while the window is active; local folders follow changes at once.")),
    (Sec::Colors, RowId::TypeColors, n!("Name colors by file type"), ""),
    (Sec::Thumbs, RowId::ThumbSize, n!("Thumbnail size:"), ""),
    (Sec::Thumbs, RowId::ThumbRemote, n!("Image previews on servers too"), ""),
    (Sec::Thumbs, RowId::ThumbMax, n!("Download only files up to"), ""),
    (Sec::Thumbs, RowId::ThumbProduction, n!("Also on production servers"), ""),
    (Sec::Copying, RowId::SpeedLimit, n!("Transfer speed limit:"), ""),
    (Sec::Copying, RowId::DropPreset, n!("Dragging from a system window:"), n!("What is preselected (Enter) when the file already exists in the target.")),
    (Sec::Copying, RowId::CopyPreset, n!("Copying with F5:"), n!("What is preselected (Enter) when the file already exists in the target.")),
    (Sec::Archives, RowId::Arch(0), n!("Extract at most"), ""),
    (Sec::Archives, RowId::Arch(1), n!("At most items in an archive"), ""),
    (Sec::Archives, RowId::Arch(2), n!("Ask at a compression ratio above"), ""),
    (Sec::Archives, RowId::Arch(3), n!("Download and open a remote archive up to"), ""),
    (Sec::Archives, RowId::Arch(4), n!("Number of compression threads"), ""),
    (Sec::Dragging, RowId::X11OnWayland, n!("On Wayland, use X11 (XWayland)"), n!("Dragging files to and from other programs. Takes effect after the program is restarted.")),
    (Sec::Viewer, RowId::Highlight, n!("Syntax highlighting"), ""),
    (Sec::Viewer, RowId::ViewerMax(0), n!("Highlight files up to"), ""),
    (Sec::Viewer, RowId::ViewerMax(1), n!("Download images from a server up to"), ""),
    (Sec::Editor, RowId::EditorCmd, n!("Editor command:"), n!("The command that opens the file. {{file}} is replaced by the path to the file.")),
    (Sec::Servers, RowId::ClockWarning, n!("Warn when a server's clock is off by more than a minute"), n!("Overwrite only older")),
    (Sec::Passwords, RowId::VaultLock, n!("Lock automatically:"), n!("Master password (saved passwords)")),
];

/// The settings as edited in the window (applied on Save or Apply).
#[derive(Clone)]
pub struct SettingsForm {
    pub toolbar: bool,
    pub nav_bar: bool,
    pub brackets: bool,
    pub ext_column: bool,
    pub seconds: bool,
    pub mask_dirs: bool,
    pub refresh_secs: u32,
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
    /// Viewer: syntax highlighting on.
    pub viewer_hl: bool,
    /// Viewer limits as typed: highlighting MB, images from a server MB.
    pub viewer: [String; 2],
    pub speed_kib: Option<u64>,
    pub drop_preset: ConflictPreset,
    pub copy_preset: ConflictPreset,
    pub editor_cmd: String,
    pub clock_warning: bool,
    pub vault_lock: Option<u32>,
    pub x11_on_wayland: bool,
}

impl SettingsForm {
    pub fn from_settings(s: &Settings) -> Self {
        SettingsForm {
            toolbar: !s.hide_toolbar,
            nav_bar: !s.hide_nav_bar,
            brackets: !s.dir_brackets_off,
            ext_column: s.ext_column,
            seconds: s.seconds,
            mask_dirs: s.mask_dirs,
            refresh_secs: s.auto_refresh_secs.unwrap_or(super::refresh::DEFAULT_SECS),
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
            viewer_hl: !s.viewer_highlight_off,
            viewer: [(s.viewer_highlight_max() >> 20).to_string(), (s.viewer_image_max() >> 20).to_string()],
            speed_kib: s.speed_limit_kib,
            drop_preset: s.drop_preset(),
            copy_preset: s.copy_preset(),
            editor_cmd: s.editor_command.clone().unwrap_or_default(),
            clock_warning: !s.clock_warning_off,
            vault_lock: s.vault_lock_after(),
            x11_on_wayland: s.x11_on_wayland,
        }
    }

    /// Every typed number is in its range and the editor command can run.
    pub fn valid(&self) -> bool {
        (!self.remote || parse_max_kib(&self.remote_max_kib).is_some())
            && (0..5).all(|i| parse_arch(i, &self.arch[i]).is_some())
            && (0..2).all(|i| parse_viewer(i, &self.viewer[i]).is_some())
            && editor_check(&self.editor_cmd).is_ok()
    }

    /// Write the form into `s`. A value equal to what the form was opened
    /// with is written back unchanged, so opening and saving the window
    /// changes nothing (see the tests).
    pub fn apply(&self, s: &mut Settings) {
        s.hide_toolbar = !self.toolbar;
        s.hide_nav_bar = !self.nav_bar;
        s.dir_brackets_off = !self.brackets;
        s.ext_column = self.ext_column;
        s.seconds = self.seconds;
        if s.mask_dirs != self.mask_dirs {
            // A new default: the dialogs start from it again.
            s.mark_group_dirs = None;
            s.unmark_group_dirs = None;
        }
        s.mask_dirs = self.mask_dirs;
        if s.auto_refresh_secs.unwrap_or(super::refresh::DEFAULT_SECS) != self.refresh_secs {
            s.auto_refresh_secs = (self.refresh_secs != super::refresh::DEFAULT_SECS).then_some(self.refresh_secs);
        }
        if s.thumb_size.unwrap_or_default() != self.thumb_size {
            s.thumb_size = (self.thumb_size != ThumbSize::default()).then_some(self.thumb_size);
        }
        if s.thumbs_on_servers() != self.remote {
            s.thumbs_remote = (!self.remote).then_some(false);
        }
        s.thumbs_production = self.production;
        if let Some(m) = parse_max_kib(&self.remote_max_kib) {
            if s.thumbs_remote_max() / 1024 != m {
                s.thumbs_remote_max_kib = (m != 2048).then_some(m);
            }
        }
        s.type_colors_off = !self.type_colors;
        let custom: std::collections::BTreeMap<String, String> = self
            .colors
            .iter()
            .filter(|(k, c)| k.default_color(false) != Some(*c))
            .map(|(k, c)| (k.key().to_string(), to_hex(*c)))
            .collect();
        // Colors of kinds this version does not show stay as they are.
        let shown: Vec<&str> = self.colors.iter().map(|(k, _)| k.key()).collect();
        s.type_colors.retain(|k, _| !shown.contains(&k.as_str()));
        s.type_colors.extend(custom);
        let arch: Vec<Option<u64>> = (0..5).map(|i| parse_arch(i, &self.arch[i])).collect();
        let before = Self::from_settings(s);
        let changed = |i: usize| arch[i].filter(|_| before.arch[i].trim() != self.arch[i].trim());
        if let Some(v) = changed(0) {
            s.unpack_max_gb = (v != 20).then_some(v);
        }
        if let Some(v) = changed(1) {
            s.unpack_max_items = (v != 1_000_000).then_some(v);
        }
        if let Some(v) = changed(2) {
            s.unpack_max_ratio = (v != 1000).then_some(v);
        }
        if let Some(v) = changed(3) {
            s.remote_archive_max_mb = (v != 500).then_some(v);
        }
        if let Some(v) = changed(4) {
            s.pack_threads = (v != Settings::default_pack_threads()).then_some(v);
        }
        s.viewer_highlight_off = !self.viewer_hl;
        for i in 0..2 {
            let Some(v) = parse_viewer(i, &self.viewer[i]).filter(|_| before.viewer[i].trim() != self.viewer[i].trim()) else { continue };
            match i {
                0 => s.viewer_highlight_max_mb = (v != 20).then_some(v),
                _ => s.viewer_image_max_mb = (v != 50).then_some(v),
            }
        }
        s.font_system = self.font_system;
        let family = self.font_family.trim().to_string();
        s.font_family = (!family.is_empty()).then_some(family);
        let size = crate::fonts::clamp_size(self.font_size);
        if before.font_size != size {
            s.font_size = (size != crate::fonts::DEFAULT_SIZE).then_some(size);
        }
        if before.font_hinting != self.font_hinting {
            s.font_hinting = (self.font_hinting != Default::default()).then_some(self.font_hinting);
        }
        if before.font_contrast != self.font_contrast {
            s.font_contrast = (self.font_contrast != Default::default()).then_some(self.font_contrast);
        }
        s.language = self.language.clone();
        s.date_iso = self.date_iso;
        s.speed_limit_kib = self.speed_kib;
        if before.drop_preset != self.drop_preset {
            s.drop_conflict = Some(self.drop_preset);
        }
        if before.copy_preset != self.copy_preset {
            s.copy_conflict = Some(self.copy_preset);
        }
        let cmd = self.editor_cmd.trim().to_string();
        if before.editor_cmd.trim() != cmd {
            s.editor_command = (!cmd.is_empty()).then_some(cmd);
        }
        s.clock_warning_off = !self.clock_warning;
        s.x11_on_wayland = self.x11_on_wayland;
        if before.vault_lock != self.vault_lock {
            s.set_vault_lock_after(self.vault_lock);
        }
    }
}

/// Can the editor command run? (empty = automatic)
fn editor_check(cmd: &str) -> Result<(), rolid_i18n::rolid_text::Text> {
    if cmd.trim().is_empty() {
        Ok(())
    } else {
        rolid_edit::editor::build(cmd, std::path::Path::new("/tmp/x")).map(|_| ())
    }
}

/// Fold a text for the filter: lower case, letters without accents.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ą' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ç' | 'č' | 'ć' => out.push('c'),
            'ď' => out.push('d'),
            'é' | 'è' | 'ê' | 'ë' | 'ě' | 'ę' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ľ' | 'ĺ' | 'ł' => out.push('l'),
            'ñ' | 'ň' | 'ń' => out.push('n'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => out.push('o'),
            'œ' => out.push_str("oe"),
            'ř' | 'ŕ' => out.push('r'),
            'š' | 'ś' => out.push('s'),
            'ß' => out.push_str("ss"),
            'ť' => out.push('t'),
            'ú' | 'ù' | 'û' | 'ü' | 'ů' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            c => out.push(c),
        }
    }
    out
}

/// Does a row match the filter? Every word of the filter must be found in
/// the label (English or translated), the extra words or the section name.
pub fn row_matches(filter: &str, sec: Sec, label: &'static str, extra: &'static str) -> bool {
    let words: Vec<String> = fold(filter).split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        return true;
    }
    let mut hay = String::new();
    let (tl, te) = (t_str(label), if extra.is_empty() { String::new() } else { t_str(extra).to_string() });
    for t in [label, &*tl, extra, &te, sec.name()] {
        hay.push_str(&fold(t));
        hay.push('\n');
    }
    hay.push_str(&fold(&sec.label()));
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// The rows the filter leaves, in order.
pub fn matching(filter: &str) -> Vec<(Sec, RowId)> {
    ROWS.iter().filter(|(s, _, l, e)| row_matches(filter, *s, l, e)).map(|(s, r, _, _)| (*s, *r)).collect()
}

/// The settings window.
pub struct SettingsWin {
    pub form: SettingsForm,
    /// The settings the form was made from (the base of the merge on Save).
    orig: Settings,
    pub section: Sec,
    pub filter: String,
    focus_filter: bool,
    /// Bring the window to the front in the next frame.
    fresh: bool,
    pub close: bool,
    /// Languages to choose from (built in and the user's .po files).
    languages: Vec<String>,
    /// The editor used when the command is empty.
    editor_default: String,
    /// "Saved." after Apply, for a few seconds.
    note: Option<Instant>,
}

impl SettingsWin {
    pub fn new(s: &Settings, section: Sec, languages: Vec<String>) -> Self {
        SettingsWin {
            form: SettingsForm::from_settings(s),
            orig: s.clone(),
            section,
            filter: String::new(),
            focus_filter: true,
            fresh: true,
            close: false,
            languages,
            editor_default: rolid_edit::editor::detect_default(),
            note: None,
        }
    }

    /// The form differs from the settings it was opened with.
    pub fn dirty(&self) -> bool {
        let mut s = self.orig.clone();
        self.form.apply(&mut s);
        s != self.orig
    }

    /// The new settings: what the form changed, merged into `current`.
    pub fn merged(&self, current: &Settings) -> Settings {
        let mut ours = self.orig.clone();
        self.form.apply(&mut ours);
        rolid_config::merge_edits(&self.orig, &ours, current)
    }
}

/// What the window needs from the application to draw.
struct Env {
    font_desc: String,
    thumb_mem: usize,
    lang_dir: String,
}

enum Press {
    Save,
    Apply,
    Cancel,
}

impl App {
    /// Settings → Settings… (Ctrl+,): the window, at `section`.
    pub(super) fn open_settings_at(&mut self, section: Sec) {
        match &mut self.settings_win {
            Some(w) => {
                w.section = section;
                w.filter.clear();
                w.fresh = true;
            }
            None => {
                let langs = rolid_i18n::available(Some(&Self::lang_dir()));
                self.settings_win = Some(Box::new(SettingsWin::new(&self.settings, section, langs)));
            }
        }
    }

    pub(super) fn open_settings(&mut self) {
        let section = self.settings_win.as_ref().map(|w| w.section).unwrap_or(Sec::Language);
        self.open_settings_at(section);
    }

    /// Save what the window changed (the window stays open on Apply).
    fn save_settings_window(&mut self) {
        let Some(w) = &mut self.settings_win else { return };
        if !w.form.valid() {
            return;
        }
        let before = self.settings.clone();
        let merged = w.merged(&self.settings);
        self.settings = merged;
        self.save_settings();
        let lock_changed = before.vault_lock_after() != self.settings.vault_lock_after();
        self.settings_changed(&before);
        if lock_changed {
            self.vault.touch();
        }
        tracing::info!("settings saved from the settings window");
        if let Some(w) = &mut self.settings_win {
            // Go on from what is saved now (Apply, then more changes).
            w.orig = self.settings.clone();
            let keep = std::mem::replace(&mut w.form, SettingsForm::from_settings(&self.settings));
            // Typed text that means the same value stays as typed.
            w.form.remote_max_kib = keep.remote_max_kib;
            w.note = Some(Instant::now());
        }
    }

    /// Draw the settings window (its own window, or inside the main one
    /// where the system cannot open more windows).
    pub(super) fn settings_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.settings_win.take() else { return };
        let env = Env { font_desc: self.font_desc.clone(), thumb_mem: self.thumbs.memory(), lang_dir: Self::lang_dir().display().to_string() };
        let mut press: Option<Press> = None;
        let id = egui::ViewportId::from_hash_of("settings-window");
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{} – RolidSCP", tr!("Settings"))) // i18n-ok: separator and product name
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([620.0, 420.0])
            .with_app_id("rolidscp");
        ctx.show_viewport_immediate(id, builder, |ui, class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested()) {
                press = Some(Press::Cancel);
            }
            if std::mem::take(&mut w.fresh) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                w.focus_filter = true;
            }
            if class == egui::ViewportClass::EmbeddedWindow {
                ui.set_min_size(egui::vec2(820.0, 560.0));
            }
            if let Some(p) = window_body(ui, &mut w, &env) {
                press = Some(p);
            }
        });
        match press {
            Some(Press::Cancel) => w.close = true,
            Some(p) => {
                self.settings_win = Some(w);
                self.save_settings_window();
                w = self.settings_win.take().expect("settings window");
                w.close |= matches!(p, Press::Save);
            }
            None => {}
        }
        if w.close {
            self.ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        } else {
            self.settings_win = Some(w);
        }
    }
}

/// The content of the window; returns a pressed button.
fn window_body(ui: &mut egui::Ui, w: &mut SettingsWin, env: &Env) -> Option<Press> {
    let ctx = ui.ctx().clone();
    // A text field had the focus: Enter only leaves it (it does not save).
    let typing = ctx.egui_wants_keyboard_input();
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        w.focus_filter = true;
    }
    let mut press = None;
    egui::Panel::top("settings-filter").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(icons::icon(icons::SEARCH, ui.visuals().text_color(), 16.0));
            let r = ui.add(
                egui::TextEdit::singleline(&mut w.filter)
                    .hint_text(tr!("Find a setting (Ctrl+F)"))
                    .desired_width(320.0),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr!("Find a setting (Ctrl+F)")));
            if std::mem::take(&mut w.focus_filter) {
                r.request_focus();
            }
            // Esc in the filter clears it first.
            if r.has_focus() && !w.filter.is_empty() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                w.filter.clear();
            }
            if !w.filter.is_empty() && ui.button(tr!("Clear")).clicked() {
                w.filter.clear();
            }
        });
        ui.add_space(4.0);
    });
    let found = matching(&w.filter);
    egui::Panel::bottom("settings-buttons").show(ui, |ui| {
        ui.add_space(6.0);
        let valid = w.form.valid();
        let dirty = w.dirty();
        ui.horizontal(|ui| {
            if !valid {
                dialog::warn(ui, tr!("Some values are out of range (marked in red)."), true);
            } else if dirty {
                dialog::note(ui, tr!("Changes are not saved yet."));
            } else if w.note.is_some_and(|t| t.elapsed().as_secs() < 4) {
                ui.colored_label(crate::look::GREEN, tr!("Saved."));
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        });
        if typing {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        }
        let width = ui.available_width();
        let b = dialog::with_width(width, || {
            dialog::buttons(ui, [
                dialog::btn(Role::Save, tr!("Save")).default(true).enabled(valid),
                dialog::btn(Role::Ok, tr!("Apply")).enabled(valid && dirty),
                dialog::cancel(),
            ])
        });
        if b[0] && valid {
            press = Some(Press::Save);
        } else if b[1] && valid {
            press = Some(Press::Apply);
        } else if b[2] {
            press = Some(Press::Cancel);
        }
        ui.add_space(4.0);
    });
    egui::Panel::left("settings-tree").resizable(false).exact_size(230.0).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(6.0);
            for (group, secs) in GROUPS {
                let shown: Vec<Sec> = secs.iter().copied().filter(|s| w.filter.trim().is_empty() || found.iter().any(|(fs, _)| fs == s)).collect();
                if shown.is_empty() {
                    continue;
                }
                ui.label(RichText::new(t_str(group)).strong());
                for s in shown {
                    let count = found.iter().filter(|(fs, _)| *fs == s).count();
                    let text = if w.filter.trim().is_empty() { s.label() } else { format!("{} ({count})", s.label()) };
                    let sel = w.filter.trim().is_empty() && w.section == s;
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let r = ui.selectable_label(sel, text);
                        if r.clicked() {
                            w.section = s;
                            w.filter.clear();
                        }
                    });
                }
                ui.add_space(6.0);
            }
            if found.is_empty() {
                dialog::note(ui, tr!("No setting matches the filter."));
            }
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            let secs: Vec<Sec> = if w.filter.trim().is_empty() {
                vec![w.section]
            } else {
                Sec::ALL.iter().copied().filter(|s| found.iter().any(|(fs, _)| fs == s)).collect()
            };
            for s in secs {
                ui.heading(s.label());
                ui.add_space(4.0);
                let rows: Vec<RowId> = found.iter().filter(|(fs, _)| *fs == s).map(|(_, r)| *r).collect();
                egui::Grid::new(("settings-grid", s as u8)).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    for r in rows {
                        draw_row(ui, r, w, env);
                        ui.end_row();
                    }
                });
                section_footer(ui, s, w, env);
                ui.add_space(14.0);
            }
        });
    });
    press
}

/// A checkbox row: no label in the first column.
fn check(ui: &mut egui::Ui, on: &mut bool, label: &'static str) -> egui::Response {
    ui.label("");
    ui.checkbox(on, t_str(label))
}

fn label_of(r: RowId) -> &'static str {
    ROWS.iter().find(|(_, id, _, _)| *id == r).map(|(_, _, l, _)| *l).unwrap_or("")
}

/// A number field with its unit, or the allowed range in red.
fn number(ui: &mut egui::Ui, text: &mut String, ok: bool, unit: String, range: (u64, u64)) {
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 20.0], egui::TextEdit::singleline(text));
        if ok {
            ui.label(unit);
        } else {
            dialog::warn(ui, tr!("number {lo} to {hi}", lo = range.0, hi = range.1), true);
        }
    });
}

fn draw_row(ui: &mut egui::Ui, r: RowId, w: &mut SettingsWin, env: &Env) {
    let label = t_str(label_of(r));
    let f = &mut w.form;
    match r {
        RowId::Language => {
            ui.label(label);
            ui.horizontal(|ui| {
                let auto = tr!("Automatic (from the system)");
                let name = |code: &str| rolid_i18n::language_name(code);
                let shown = f.language.as_deref().map(name).unwrap_or_else(|| auto.clone());
                egui::ComboBox::from_id_salt("settings-language").selected_text(shown).height(400.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.language, None, auto.as_str());
                    for code in &w.languages {
                        ui.selectable_value(&mut f.language, Some(code.clone()), name(code));
                    }
                });
                dialog::help(ui, tr!("The language changes at once. Your own translations: {dir}/<language>.po", dir = env.lang_dir.as_str()));
            });
        }
        RowId::DateFormat => {
            ui.label(label);
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.date_iso, false, tr!("By language"));
                ui.radio_value(&mut f.date_iso, true, tr!("ISO (2026-11-08 21:08)"));
            });
        }
        RowId::FontKind => {
            ui.label(label);
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.font_system, false, tr!("Built-in ({font})", font = crate::fonts::NOTO_NAME));
                ui.radio_value(&mut f.font_system, true, tr!("System (fontconfig)"));
            });
        }
        RowId::FontName => {
            ui.add_enabled(f.font_system, egui::Label::new(label));
            ui.add_enabled(f.font_system, egui::TextEdit::singleline(&mut f.font_family).hint_text(tr!("empty = default sans-serif")).desired_width(220.0));
        }
        RowId::FontSize => {
            ui.label(label);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("settings-font-size").selected_text(format!("{}", f.font_size)).width(50.0).show_ui(ui, |ui| {
                    for v in crate::fonts::MIN_SIZE as u32..=crate::fonts::MAX_SIZE as u32 {
                        ui.selectable_value(&mut f.font_size, v as f32, v.to_string());
                    }
                });
                ui.label(tr!("points (default {size})", size = crate::fonts::DEFAULT_SIZE as u32));
            });
        }
        RowId::Hinting => {
            ui.label(label);
            ui.horizontal_wrapped(|ui| {
                for h in rolid_config::FontHinting::ALL {
                    ui.radio_value(&mut f.font_hinting, h, tc_str("hinting", h.label()));
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
        }
        RowId::Contrast => {
            ui.label(label);
            ui.horizontal_wrapped(|ui| {
                for c in rolid_config::FontContrast::ALL {
                    ui.radio_value(&mut f.font_contrast, c, tc_str("contrast", c.label()));
                }
                dialog::help(
                    ui,
                    tr!(
                        "How dark the edges of the letters are drawn (how light in the dark theme). \
                         Higher (default) balances the thinner strokes of strong hinting; highest = the font looks bolder."
                    ),
                );
            });
        }
        RowId::Toolbar => {
            check(ui, &mut f.toolbar, label_of(r));
        }
        RowId::NavBar => {
            check(ui, &mut f.nav_bar, label_of(r));
        }
        RowId::Brackets => {
            check(ui, &mut f.brackets, label_of(r));
        }
        RowId::ExtColumn => {
            check(ui, &mut f.ext_column, label_of(r));
        }
        RowId::Seconds => {
            check(ui, &mut f.seconds, label_of(r));
        }
        RowId::MaskDirs => {
            check(ui, &mut f.mask_dirs, label_of(r));
        }
        RowId::Refresh => {
            ui.label(label);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    for s in super::refresh::CHOICES {
                        let text = match s {
                            0 => trc!("refresh", "Off"),
                            s if s < 60 => tr!("Every {n} s", n = s),
                            s => tr!("Every {n} min", n = s / 60),
                        };
                        ui.radio_value(&mut f.refresh_secs, s, text);
                    }
                });
                dialog::note(ui, tr!("Server folders while the window is active; local folders follow changes at once."));
            });
        }
        RowId::TypeColors => {
            ui.label("");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.type_colors, label);
                    dialog::help(ui, tr!("The colors apply to the light theme; the dark theme uses lighter shades."));
                });
                ui.add_enabled_ui(f.type_colors, |ui| {
                    egui::Grid::new("settings-type-colors").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
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
        }
        RowId::ThumbSize => {
            ui.label(label);
            ui.horizontal(|ui| {
                for s in ThumbSize::ALL {
                    ui.radio_value(&mut f.thumb_size, s, tc_str("thumbnail", s.label()));
                }
            });
        }
        RowId::ThumbRemote => {
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.remote, label);
                dialog::help(
                    ui,
                    tr!("At most 2 previews are downloaded from a server at the same time; images larger than {px} px are not processed.", px = 10_000),
                );
            });
        }
        RowId::ThumbMax => {
            ui.add_enabled(f.remote, egui::Label::new(label));
            ui.add_enabled_ui(f.remote, |ui| {
                let ok = !f.remote || parse_max_kib(&f.remote_max_kib).is_some();
                number(ui, &mut f.remote_max_kib, ok, trc!("unit", "KiB"), (16, 1_048_576));
            });
        }
        RowId::ThumbProduction => {
            ui.label("");
            ui.add_enabled(f.remote, egui::Checkbox::new(&mut f.production, label));
        }
        RowId::SpeedLimit => {
            ui.label(label);
            ui.horizontal(|ui| {
                super::speed_limit_combo(ui, "settings-speed", &mut f.speed_kib);
            });
        }
        RowId::DropPreset | RowId::CopyPreset => {
            ui.label(label);
            let p = if r == RowId::DropPreset { &mut f.drop_preset } else { &mut f.copy_preset };
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("settings-preset", r == RowId::DropPreset)).selected_text(t_str(p.label())).show_ui(ui, |ui| {
                    for c in ConflictPreset::ALL {
                        ui.selectable_value(p, c, t_str(c.label()));
                    }
                });
                dialog::help(ui, tr!("What is preselected (Enter) when the file already exists in the target."));
            });
        }
        RowId::Arch(i) => {
            ui.label(label);
            let threads_hint = tr!("(default {threads} = number of cores − 1)", threads = Settings::default_pack_threads());
            let unit = match i {
                0 => tr!("GB (and always leave 1 GB free)"),
                1 => String::new(),
                2 => ": 1".to_string(),
                3 => trc!("unit", "MB"),
                _ => threads_hint,
            };
            let ok = parse_arch(i, &f.arch[i]).is_some();
            number(ui, &mut f.arch[i], ok, unit, ARCH_RANGES[i]);
        }
        RowId::Highlight => {
            check(ui, &mut f.viewer_hl, label_of(r));
        }
        RowId::ViewerMax(i) => {
            ui.label(label);
            let ok = parse_viewer(i, &f.viewer[i]).is_some();
            number(ui, &mut f.viewer[i], ok, trc!("unit", "MB"), VIEWER_RANGES[i]);
        }
        RowId::EditorCmd => {
            ui.label(label);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.editor_cmd).desired_width(360.0).hint_text(w.editor_default.as_str()));
                    dialog::help(
                        ui,
                        format!(
                            "{}\n\n{}",
                            tr!("Empty = automatic (now: {command}).", command = w.editor_default.as_str()),
                            tr!("A terminal editor only through a terminal, e.g.: x-terminal-emulator -e nano {{file}}")
                        ),
                    );
                });
                dialog::note(ui, tr!("The command that opens the file. {{file}} is replaced by the path to the file."));
                if let Err(e) = editor_check(&f.editor_cmd) {
                    dialog::warn(ui, rolid_i18n::t(&e), true);
                }
            });
        }
        RowId::ClockWarning => {
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.clock_warning, label);
                dialog::help(
                    ui,
                    tr!(
                        "After connecting, the program asks the server for its time (with the date command, \
                         only where the server has a shell). When it is off by more than a minute, the line at \
                         the bottom of the window says so: \"Overwrite only older\" compares the times of files, \
                         and a file changed on that server gets a wrong time."
                    ),
                );
            });
        }
        RowId::X11OnWayland => {
            ui.label("");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.x11_on_wayland, label);
                    dialog::help(
                        ui,
                        tr!(
                            "On Wayland the window library of the program cannot drag files to and from other \
                             programs yet. Through XWayland the window is an X11 one, and dragging works where \
                             the desktop passes it on between X11 and Wayland programs (GNOME does)."
                        ),
                    );
                });
                dialog::note(ui, tr!("Dragging files to and from other programs. Takes effect after the program is restarted."));
                dialog::warn(
                    ui,
                    tr!(
                        "What you lose: text is less sharp with fractional scaling (for example 125 % or 150 %), \
                         and other X11 programs can read what you type in this window, the master password too. \
                         A Wayland window keeps programs apart."
                    ),
                    false,
                );
            });
        }
        RowId::VaultLock => {
            ui.label(label);
            egui::ComboBox::from_id_salt("settings-vault-lock").selected_text(super::vault_ui::lock_label(f.vault_lock)).show_ui(ui, |ui| {
                for s in super::vault_ui::LOCK_CHOICES {
                    ui.selectable_value(&mut f.vault_lock, s, super::vault_ui::lock_label(s));
                }
            });
        }
    }
}

/// What the window runs on now, and whether the setting `chosen` differs.
fn dragging_now(chosen: bool) -> String {
    use super::x11_drag::{self, Backend};
    let now = x11_drag::backend();
    if !x11_drag::x11_can_be_chosen() {
        return match now {
            Backend::Wayland => tr!("No X11 display (XWayland) is available here: the window stays a Wayland one."),
            _ => tr!("This is an X11 session: dragging works, the setting matters only on Wayland."),
        };
    }
    let running = now == Backend::XWayland;
    let mut out = if running {
        tr!("Now: X11 through XWayland.")
    } else {
        tr!("Now: Wayland.")
    };
    if chosen != running {
        out.push(' ');
        out.push_str(&tr!("The change takes effect after the program is restarted."));
    }
    out
}

/// What a section shows under its settings (not searched).
fn section_footer(ui: &mut egui::Ui, s: Sec, w: &SettingsWin, env: &Env) {
    match s {
        Sec::Font => dialog::note(ui, tr!("Now: {font}", font = env.font_desc.as_str())),
        Sec::Thumbs => dialog::note(ui, tr!("Previews in memory: {size} (at most 48 MB).", size = rolid_i18n::size(env.thumb_mem as u64))),
        Sec::Dragging => dialog::note(ui, dragging_now(w.form.x11_on_wayland)),
        Sec::Passwords => dialog::note(
            ui,
            tr!("Creating, unlocking and changing the master password: Settings → Master password (saved passwords) in the menu bar."),
        ),
        _ => {}
    }
    let _ = w;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customized() -> Settings {
        let mut s = Settings {
            hide_toolbar: true,
            hide_nav_bar: true,
            dir_brackets_off: true,
            ext_column: true,
            seconds: true,
            mask_dirs: true,
            mark_group_dirs: Some(false),
            auto_refresh_secs: Some(0),
            thumb_size: Some(ThumbSize::ALL[0]),
            thumbs_remote: Some(false),
            thumbs_production: true,
            thumbs_remote_max_kib: Some(4096),
            type_colors_off: true,
            unpack_max_gb: Some(7),
            unpack_max_items: Some(12),
            unpack_max_ratio: Some(55),
            remote_archive_max_mb: Some(9),
            pack_threads: Some(3),
            font_system: true,
            font_family: Some("DejaVu Sans".into()),
            font_size: Some(17.0),
            language: Some("cs".into()),
            date_iso: true,
            viewer_highlight_off: true,
            viewer_highlight_max_mb: Some(3),
            viewer_image_max_mb: Some(4),
            speed_limit_kib: Some(512),
            drop_conflict: Some(ConflictPreset::ALL[1]),
            copy_conflict: Some(ConflictPreset::ALL[2]),
            editor_command: Some("gedit {file}".into()),
            clock_warning_off: true,
            x11_on_wayland: true,
            ..Default::default()
        };
        s.type_colors.insert("image".into(), "#112233".into());
        // A color of a kind this version may not know.
        s.type_colors.insert("hologram".into(), "#445566".into());
        s.set_vault_lock_after(Some(300));
        s
    }

    #[test]
    fn opening_and_saving_changes_nothing() {
        for s in [Settings::default(), customized()] {
            let f = SettingsForm::from_settings(&s);
            let mut out = s.clone();
            f.apply(&mut out);
            assert_eq!(out, s);
            let mut w = SettingsWin::new(&s, Sec::Language, vec![]);
            w.editor_default = "x".into();
            assert!(!w.dirty());
            assert_eq!(w.merged(&s), s);
        }
    }

    #[test]
    fn saving_keeps_changes_made_meanwhile_elsewhere() {
        let s = Settings::default();
        let mut w = SettingsWin::new(&s, Sec::Panels, vec![]);
        w.form.seconds = true;
        assert!(w.dirty());
        // Meanwhile the toolbar was hidden from the View menu and another
        // copy set a speed limit.
        let mut now = s.clone();
        now.hide_toolbar = true;
        now.speed_limit_kib = Some(1024);
        let m = w.merged(&now);
        assert!(m.seconds, "the change made in the window");
        assert!(m.hide_toolbar, "the toolbar stays hidden");
        assert_eq!(m.speed_limit_kib, Some(1024), "the other copy's limit stays");
    }

    #[test]
    fn values_out_of_range_cannot_be_saved() {
        let mut f = SettingsForm::from_settings(&Settings::default());
        assert!(f.valid());
        f.arch[0] = "0".into();
        assert!(!f.valid());
        f.arch[0] = "20".into();
        f.viewer[1] = "abc".into();
        assert!(!f.valid());
        f.viewer[1] = "50".into();
        f.remote_max_kib = "1".into();
        assert!(!f.valid(), "the thumbnail limit is checked while previews on servers are on");
        f.remote = false;
        assert!(f.valid());
        f.editor_cmd = "\"unclosed {file}".into();
        assert!(!f.valid(), "an editor command that cannot be split");
    }

    #[test]
    fn a_changed_number_is_written_and_the_default_removes_the_key() {
        let s = customized();
        let mut f = SettingsForm::from_settings(&s);
        f.arch[0] = "20".into(); // the default
        f.arch[1] = "77".into();
        f.speed_kib = None;
        f.clock_warning = true;
        let mut out = s.clone();
        f.apply(&mut out);
        assert_eq!(out.unpack_max_gb, None);
        assert_eq!(out.unpack_max_items, Some(77));
        assert_eq!(out.speed_limit_kib, None);
        assert!(!out.clock_warning_off);
        assert_eq!(out.type_colors.get("hologram").map(String::as_str), Some("#445566"), "unknown kinds stay");
    }

    #[test]
    fn the_filter_finds_settings_by_words_without_accents() {
        assert_eq!(fold("Písmo ŽLUŤOUČKÝ Straße"), "pismo zlutoucky strasse");
        let all = matching("");
        assert_eq!(all.len(), ROWS.len());
        let r = matching("speed");
        assert_eq!(r, vec![(Sec::Copying, RowId::SpeedLimit)]);
        // Several words: all must be found.
        assert_eq!(matching("font contrast"), vec![(Sec::Font, RowId::Contrast)]);
        // The section name matches all its rows.
        assert_eq!(matching("archives").len(), 5);
        assert!(matching("no such setting at all").is_empty());
        // The X11 choice is found by what it is for.
        assert_eq!(matching("wayland"), vec![(Sec::Dragging, RowId::X11OnWayland)]);
        assert_eq!(matching("xwayland restart"), vec![(Sec::Dragging, RowId::X11OnWayland)]);
        assert_eq!(matching("drag other programs"), vec![(Sec::Dragging, RowId::X11OnWayland)]);
        // Every row is in a section of the tree, and every section has rows.
        for (s, _, _, _) in ROWS {
            assert!(GROUPS.iter().any(|(_, secs)| secs.contains(s)));
        }
        for s in Sec::ALL {
            assert!(ROWS.iter().any(|(rs, _, _, _)| *rs == s), "{s:?} has no rows");
        }
    }
}
