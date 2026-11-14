//! Classic menu bar (File, Settings, View, Tools, Help), the
//! keyboard-shortcut overview (F1) and the About dialog.

use eframe::egui::{self, RichText};
use rolid_i18n::rolid_text::{n, nc};
use rolid_i18n::{t_str, tc_str};

use super::{Action, App, Dialog};
use crate::pane::SortKey;

pub use crate::version::VERSION;

/// Menu item with a right-aligned shortcut (key names); returns true when clicked.
fn keyed(ui: &mut egui::Ui, label: String, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
}

fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let prog = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(prog).arg(path).spawn().map(|_| ())
}

impl App {
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let mut act: Option<Action> = None;
        let mut set = |a: Action| act = Some(a);
        let in_search = self.pane(self.active_ref()).is_some_and(|p| p.search.is_some());
        let remote = self.pane(self.active_ref()).is_some_and(|p| p.remote.is_some());
        let speed = self.settings.speed_limit_kib;
        let (drop_preset, copy_preset) = (self.settings.drop_preset(), self.settings.copy_preset());
        let vault_exists = self.vault.exists();
        let vault_open = self.vault.open.as_ref().map(|v| v.len());
        let lock_after = self.settings.vault_lock_after();
        let legacy = self.legacy_profile_ids().len();
        let edits = self.edits.sessions.len();
        let style = self.active_style();
        let toolbar = !self.settings.hide_toolbar;
        let nav_bar = !self.settings.hide_nav_bar;
        let (ext_column, seconds) = (self.settings.ext_column, self.settings.seconds);
        let brackets = !self.settings.dir_brackets_off;
        let (can_back, can_fwd) = self
            .pane(self.active_ref())
            .map(|p| (p.history.can_back() && p.search.is_none(), p.history.can_forward() && p.search.is_none()))
            .unwrap_or((false, false));

        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(trc!("menu", "File"), |ui| {
                if keyed(ui, tr!("New tab…"), "Ctrl+T") {
                    set(Box::new(|a| a.open_new_tab_menu(a.active_side)));
                }
                if keyed(ui, tr!("Duplicate tab"), "") {
                    set(Box::new(|a| a.duplicate_tab(a.active_side, a.sides[a.active_side].active)));
                }
                if keyed(ui, tr!("Close tab"), "Ctrl+W") {
                    set(Box::new(|a| a.close_active_tab()));
                }
                ui.separator();
                if keyed(ui, tr!("Add to / remove from favorites"), "Ctrl+D") {
                    set(Box::new(|a| a.toggle_favorite(a.active_ref())));
                }
                if keyed(ui, tr!("Favorite folders…"), "") {
                    set(Box::new(|a| a.open_favorites()));
                }
                ui.separator();
                if keyed(ui, tr!("Connect to server…"), "Ctrl+O") {
                    set(Box::new(|a| a.open_connect()));
                }
                if remote && keyed(ui, tr!("Reconnect / refresh"), "Ctrl+R") {
                    set(Box::new(|a| a.refresh_or_rerun()));
                }
                if keyed(ui, tr!("Local disk in this panel"), "Ctrl+L") {
                    set(Box::new(|a| a.set_local(a.active_ref())));
                }
                ui.separator();
                if keyed(ui, trc!("verb", "Open"), "Enter") {
                    set(Box::new(|a| a.activate_entry(a.active_ref())));
                }
                if keyed(ui, tr!("Open with…"), "Shift+Enter") {
                    set(Box::new(|a| a.ask_open_with(a.active_ref())));
                }
                if keyed(ui, trc!("verb", "View"), "F3") {
                    set(Box::new(|a| a.view_active()));
                }
                if keyed(ui, tr!("Edit in external editor"), "F4") {
                    set(Box::new(|a| a.start_edit_active()));
                }
                if ui
                    .add_enabled(!in_search, egui::Button::new(tr!("New file…")).shortcut_text("Shift+F4"))
                    .clicked()
                {
                    set(Box::new(|a| a.ask_new_file()));
                }
                if keyed(ui, tr!("Copy to other panel"), "F5") {
                    set(Box::new(|a| a.ask_copy()));
                }
                if keyed(ui, tr!("Move to other panel"), "F6") {
                    set(Box::new(|a| a.ask_move()));
                }
                if ui
                    .add_enabled(!in_search, egui::Button::new(tr!("Rename")).shortcut_text("F2 / Shift+F6"))
                    .clicked()
                {
                    set(Box::new(|a| a.start_rename_active()));
                }
                if ui
                    .add_enabled(!in_search, egui::Button::new(tr!("New folder")).shortcut_text("F7"))
                    .clicked()
                {
                    set(Box::new(|a| a.ask_mkdir()));
                }
                if keyed(ui, tr!("Delete"), "F8") {
                    set(Box::new(|a| a.ask_delete()));
                }
                if keyed(ui, tr!("Permissions…"), "Ctrl+Shift+A") {
                    set(Box::new(|a| a.ask_perms(a.active_ref())));
                }
                ui.separator();
                if keyed(ui, tr!("Pack…"), "Alt+F5") {
                    set(Box::new(|a| a.ask_pack()));
                }
                if keyed(ui, tr!("Extract…"), "Alt+F9") {
                    set(Box::new(|a| a.ask_archive(super::unpack_ui::Purpose::Extract)));
                }
                if keyed(ui, tr!("Test archive"), "Alt+Shift+F9") {
                    set(Box::new(|a| a.ask_archive(super::unpack_ui::Purpose::Test)));
                }
                ui.separator();
                if keyed(ui, tr!("Calculate size of all folders"), "Alt+Shift+Enter") {
                    set(Box::new(|a| a.size_all_dirs()));
                }
                if keyed(ui, tr!("Calculate size of selected folders"), "Alt+Space / Shift+Space") {
                    set(Box::new(|a| a.size_selected_dirs()));
                }
                if keyed(ui, tr!("Calculate checksums…"), "") {
                    set(Box::new(|a| a.ask_checksums()));
                }
                ui.separator();
                ui.menu_button(tr!("Copy to clipboard"), |ui| {
                    use super::ctx_ui::ClipKind;
                    for (t, k, kind, on) in [
                        (tr!("Copy name"), "Ctrl+Shift+N", ClipKind::Names, true),
                        (tr!("Copy full path"), "Ctrl+Shift+P", ClipKind::Paths, true),
                        (tr!("Copy as sftp://…"), "Ctrl+Shift+S", ClipKind::Urls, remote),
                    ] {
                        if ui.add_enabled(on, egui::Button::new(t).shortcut_text(k)).clicked() {
                            set(Box::new(move |a| a.copy_to_clipboard(a.active_ref(), kind)));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if keyed(ui, tr!("Quit"), "Ctrl+Q") {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button(trc!("menu", "Settings"), |ui| {
                if keyed(ui, tr!("Settings…"), "") {
                    set(Box::new(|a| a.open_settings()));
                }
                if keyed(ui, tr!("Server manager…"), "Ctrl+O") {
                    set(Box::new(|a| a.open_connect()));
                }
                if keyed(ui, tr!("Rename tab"), &tr!("double-click the tab")) {
                    set(Box::new(|a| a.start_rename_tab()));
                }
                ui.add_enabled(false, egui::Button::new(tr!("Tab color")).shortcut_text(tr!("right-click the tab")));
                ui.separator();
                if keyed(ui, tr!("Open settings folder"), "") {
                    set(Box::new(|a| {
                        let dir = rolid_config::config_dir();
                        let _ = std::fs::create_dir_all(&dir);
                        if let Err(e) = open_in_file_manager(&dir) {
                            a.last_error = Some(format!("{}: {e}", dir.display()));
                        }
                    }));
                }
                if keyed(ui, tr!("Clear search history"), "") {
                    set(Box::new(|a| a.clear_search_history()));
                }
                ui.separator();
                ui.menu_button(tr!("Transfer speed limit: {speed}", speed = super::fmt_speed(speed)), |ui| {
                    for p in super::SPEED_PRESETS {
                        if ui.radio(speed == p, super::fmt_speed(p)).clicked() {
                            set(Box::new(move |a| a.set_speed_limit(p)));
                        }
                    }
                });
                ui.menu_button(tr!("Default choice on conflict"), |ui| {
                    ui.weak(tr!("What is preselected (Enter) when the file already exists in the target."));
                    ui.menu_button(tr!("Dragging from a system window: {choice}", choice = t_str(drop_preset.label())), |ui| {
                        for p in rolid_config::ConflictPreset::ALL {
                            if ui.radio(drop_preset == p, t_str(p.label())).clicked() {
                                set(Box::new(move |a| a.set_conflict_preset(true, p)));
                            }
                        }
                    });
                    ui.menu_button(tr!("Copying with F5: {choice}", choice = t_str(copy_preset.label())), |ui| {
                        for p in rolid_config::ConflictPreset::ALL {
                            if ui.radio(copy_preset == p, t_str(p.label())).clicked() {
                                set(Box::new(move |a| a.set_conflict_preset(false, p)));
                            }
                        }
                    });
                });
                ui.separator();
                if keyed(ui, tr!("External editor (F4)…"), "") {
                    set(Box::new(|a| a.ask_editor_command()));
                }
                ui.menu_button(tr!("Master password (saved passwords)"), |ui| {
                    let state = match (vault_exists, vault_open) {
                        (false, _) => tr!("Status: master password not created yet"),
                        (true, None) => tr!("Status: 🔒 locked"),
                        (true, Some(n)) => trn!("Status: 🔓 unlocked ({n} saved password)", "Status: 🔓 unlocked ({n} saved passwords)", n),
                    };
                    ui.add_enabled(false, egui::Button::new(state));
                    ui.separator();
                    if !vault_exists && keyed(ui, tr!("Create master password…"), "") {
                        set(Box::new(|a| a.ask_create()));
                    }
                    if vault_exists && vault_open.is_none() && keyed(ui, tr!("Unlock…"), "") {
                        set(Box::new(|a| a.ask_unlock()));
                    }
                    if ui
                        .add_enabled(vault_open.is_some(), egui::Button::new(tr!("Lock now")))
                        .clicked()
                    {
                        set(Box::new(|a| a.lock_vault("manually from the menu")));
                    }
                    if ui.add_enabled(vault_exists, egui::Button::new(tr!("Change master password…"))).clicked() {
                        set(Box::new(|a| a.ask_change()));
                    }
                    ui.menu_button(tr!("Lock automatically: {when}", when = super::vault_ui::lock_label(lock_after)), |ui| {
                        for s in super::vault_ui::LOCK_CHOICES {
                            if ui.radio(lock_after == s, super::vault_ui::lock_label(s)).clicked() {
                                set(Box::new(move |a| a.set_vault_lock(s)));
                            }
                        }
                    });
                    if legacy > 0 && keyed(ui, tr!("Move passwords from the system keyring ({count})…", count = legacy), "") {
                        set(Box::new(|a| a.ask_migrate()));
                    }
                    ui.separator();
                    if ui
                        .add_enabled(vault_exists, egui::Button::new(tr!("Delete saved passwords and start over…")))
                        .clicked()
                    {
                        set(Box::new(|a| a.ask_reset()));
                    }
                });
            });

            ui.menu_button(trc!("menu", "View"), |ui| {
                if keyed(ui, tr!("Refresh"), "Ctrl+R") {
                    set(Box::new(|a| a.refresh_or_rerun()));
                }
                ui.separator();
                for st in super::view_ui::STYLES {
                    use super::view_ui::ViewStyleExt;
                    let b = egui::Button::selectable(style == st, st.label()).shortcut_text(st.shortcut());
                    if ui.add(b).clicked() {
                        set(Box::new(move |a| a.set_view_style(st)));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.add_enabled(can_back, egui::Button::new(tr!("Back")).shortcut_text("Alt+←")).clicked() {
                    set(Box::new(|a| a.history_go(false)));
                }
                if ui.add_enabled(can_fwd, egui::Button::new(tr!("Forward")).shortcut_text("Alt+→")).clicked() {
                    set(Box::new(|a| a.history_go(true)));
                }
                if keyed(ui, tr!("Folders on the path and disks…"), "Alt+↓") {
                    set(Box::new(|a| a.open_path_tree()));
                }
                if keyed(ui, tr!("Root folder"), "Ctrl+\\") {
                    set(Box::new(|a| a.goto(a.active_ref(), a.pane(a.active_ref()).map(|p| p.root_path()).unwrap_or_else(|| "/".into()))));
                }
                if keyed(ui, tr!("Home folder"), "Ctrl+H") {
                    set(Box::new(|a| a.goto(a.active_ref(), "~".into())));
                }
                ui.separator();
                let mut tb = toolbar;
                if ui.checkbox(&mut tb, tr!("Toolbar")).clicked() {
                    set(Box::new(|a| a.toggle_toolbar()));
                    ui.close();
                }
                let mut nb = nav_bar;
                if ui.checkbox(&mut nb, tr!("Bar above the panels (path tree, navigation)")).clicked() {
                    set(Box::new(|a| a.toggle_nav_bar()));
                    ui.close();
                }
                let mut ext = ext_column;
                if ui.checkbox(&mut ext, tr!("Extension column")).clicked() {
                    set(Box::new(|a| a.toggle_ext_column()));
                    ui.close();
                }
                let mut br = brackets;
                if ui.checkbox(&mut br, tr!("Folders in [square brackets]")).clicked() {
                    set(Box::new(|a| a.toggle_brackets()));
                    ui.close();
                }
                let mut sec = seconds;
                if ui.checkbox(&mut sec, tr!("Modified time with seconds")).clicked() {
                    set(Box::new(|a| a.toggle_seconds()));
                    ui.close();
                }
                let custom = !self.settings.columns.is_empty();
                if ui
                    .add_enabled(custom, egui::Button::new(tr!("Reset column widths")))
                    .on_hover_text(tr!("Column widths are set by dragging the lines between the column titles; a double click fits the column to its content."))
                    .on_disabled_hover_text(tr!("Column widths are set by dragging the lines between the column titles; a double click fits the column to its content."))
                    .clicked()
                {
                    set(Box::new(|a| a.reset_column_widths()));
                    ui.close();
                }
                let secs = self.refresh_secs();
                ui.menu_button(tr!("Automatic refresh"), |ui| {
                    ui.set_max_width(240.0);
                    ui.add(egui::Label::new(egui::RichText::new(tr!("Server folders while the window is active; local folders follow changes at once.")).weak()).wrap());
                    for s in super::refresh::CHOICES {
                        let label = match s {
                            0 => trc!("refresh", "Off"),
                            s if s < 60 => tr!("Every {n} s", n = s),
                            s => tr!("Every {n} min", n = s / 60),
                        };
                        if ui.radio(secs == s, label).clicked() {
                            set(Box::new(move |a| a.set_refresh_secs(s)));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let column_click = tr!("click the column header");
                for (key, label) in [
                    (SortKey::Name, tr!("Sort by name")),
                    (SortKey::Size, tr!("Sort by size")),
                    (SortKey::Modified, tr!("Sort by modified date")),
                    (SortKey::Ext, tr!("Sort by extension")),
                ] {
                    if keyed(ui, label, &column_click) {
                        set(Box::new(move |a| {
                            let at = a.active_ref();
                            if let Some(p) = a.pane_mut(at) {
                                p.set_sort(key);
                            }
                        }));
                    }
                }
                ui.separator();
                if keyed(ui, tr!("Switch left / right panel"), "Tab") {
                    set(Box::new(|a| a.switch_pane()));
                }
                if keyed(ui, tr!("Next tab"), "Ctrl+PgDn") {
                    set(Box::new(|a| a.step_tab(1)));
                }
                if keyed(ui, tr!("Previous tab"), "Ctrl+PgUp") {
                    set(Box::new(|a| a.step_tab(-1)));
                }
                if keyed(ui, tr!("Tab and server switcher…"), "Ctrl+K") {
                    set(Box::new(|a| a.open_switcher()));
                }
            });

            ui.menu_button(trc!("menu", "Tools"), |ui| {
                if keyed(ui, tr!("Find files…"), "Ctrl+F7 / Ctrl+F") {
                    set(Box::new(|a| a.open_search_dialog()));
                }
                if ui
                    .add_enabled(in_search, egui::Button::new(tr!("Search again")).shortcut_text("Ctrl+R"))
                    .clicked()
                {
                    set(Box::new(|a| a.rerun_search(a.active_ref())));
                }
                ui.separator();
                if keyed(ui, tr!("Files being edited ({count})…", count = edits), "") {
                    set(Box::new(|a| a.edits.show_list = true));
                }
                ui.separator();
                if keyed(ui, tr!("Open terminal here"), "Ctrl+P") {
                    set(Box::new(|a| a.open_terminal()));
                }
                if keyed(ui, tr!("Go to path…"), "Ctrl+G") {
                    set(Box::new(|a| {
                        let at = a.active_ref();
                        if let Some(p) = a.pane_mut(at) {
                            p.path_edit = Some(p.path.clone());
                        }
                    }));
                }
                if keyed(ui, tr!("Open folder in left panel"), "Ctrl+←") {
                    set(Box::new(|a| a.open_in_other(0)));
                }
                if keyed(ui, tr!("Open folder in right panel"), "Ctrl+→") {
                    set(Box::new(|a| a.open_in_other(1)));
                }
                if keyed(ui, tr!("Select group…"), super::view_ui::MARK_KEYS) {
                    set(Box::new(|a| a.ask_mark_group(true)));
                }
                if keyed(ui, tr!("Deselect group…"), super::view_ui::UNMARK_KEYS) {
                    set(Box::new(|a| a.ask_mark_group(false)));
                }
                if keyed(ui, tr!("Invert selection"), super::view_ui::INVERT_KEYS) {
                    set(Box::new(|a| a.invert_selection()));
                }
                if keyed(ui, tr!("Select all / none"), "Ctrl+A") {
                    set(Box::new(|a| {
                        let at = a.active_ref();
                        if let Some(p) = a.pane_mut(at) {
                            p.mark_all();
                        }
                    }));
                }
            });

            ui.menu_button(trc!("menu", "Help"), |ui| {
                if keyed(ui, tr!("Help"), "F1") {
                    set(Box::new(|a| a.dialog = Dialog::Help(false)));
                }
                if keyed(ui, tr!("Keyboard shortcuts"), "") {
                    set(Box::new(|a| a.dialog = Dialog::Help(true)));
                }
                if keyed(ui, tr!("Open log (login diagnostics)"), "") {
                    set(Box::new(|a| {
                        let log = rolid_config::log_path();
                        if let Err(e) = open_in_file_manager(&log) {
                            a.last_error = Some(format!("{}: {e}", log.display()));
                        }
                    }));
                }
                if keyed(ui, tr!("About RolidSCP"), "") {
                    set(Box::new(|a| a.dialog = Dialog::About));
                }
            });

        });
        if let Some(a) = act {
            a(self);
        }
    }
}

/// The F1 overview: section, rows of (keys, what they do). Texts are marked
/// with `n!` and translated when drawn; keys with words are marked with the
/// context "keys", plain key names stay as they are.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        n!("Panels"),
        &[
            ("↑ ↓ PgUp PgDn Home End", n!("move the cursor")),
            ("← →", n!("by column (brief view) / by tile (thumbnails)")),
            ("Ctrl+F1 / Ctrl+F2 / Ctrl+Shift+F1", n!("brief / detailed view / thumbnails (separately for each tab)")),
            ("Alt+← / Alt+→", n!("back / forward in the tab's folder history")),
            ("Alt+↓", n!("folders on the path and disks (the drop-down of the bar above the panel; arrows, Enter, Esc)")),
            ("Ctrl+\\", n!("root folder")),
            ("Ctrl+H", n!("home folder")),
            (nc!("keys", "Enter / double-click"), n!("open: a folder, an archive as a folder, a file with the default (or remembered) application; a program or script asks first")),
            ("Backspace", n!("one level up")),
            ("Tab", n!("switch left / right panel")),
            (nc!("keys", "typing"), n!("quick filter (Esc clears it)")),
            (nc!("keys", "Insert / Space / Ctrl+click"), n!("select an item (Space only selects; folder size is Alt+Space)")),
            ("Ctrl+A", n!("select all / none")),
            ("Num+ / Alt+=", n!("select a group by mask (*.jpg;*.png)")),
            ("Alt+−", n!("deselect a group (Num− and Num* cannot be told apart from the regular keys)")),
            ("Alt+I", n!("invert the file selection (folders too, depending on Settings)")),
            (nc!("keys", "Ctrl+G / click the path"), n!("enter a path")),
            (nc!("keys", "/  ..  ~  * at the right of the path bar"), n!("root, one level up, home, recent folders")),
            ("Ctrl+← / Ctrl+→", n!("open folder in the left / right panel")),
            ("Ctrl+R", n!("refresh")),
            (nc!("keys", "right-click a file"), n!("context menu")),
        ],
    ),
    (
        n!("Files"),
        &[
            ("F3", n!("view the file: text, hex or image; a server file is not downloaded whole")),
            ("F4", n!("edit in an external editor (a server file is uploaded back after saving)")),
            ("Shift+F4", n!("new file in the external editor (on a server it is created by the first save)")),
            ("F2 / Shift+F6", n!("rename right in the list (Enter confirms, Esc or a click elsewhere cancels)")),
            ("F5", n!("copy to the other panel")),
            ("F6", n!("move to the other panel (the target and the new name can be edited)")),
            ("Alt+F5", n!("pack the selection (zip, 7z, tar.gz, tar.zst)")),
            ("Alt+F9", n!("extract the selected archives")),
            ("Alt+Shift+F9", n!("test an archive (writes nothing)")),
            (nc!("keys", "Enter on an archive"), n!("open the archive as a read-only folder (.. at its root = back); F5 copies out, an archive on a server is downloaded first")),
            ("Shift+Enter", n!("Open with… (choose an application or a custom command)")),
            ("Alt+Shift+Enter", n!("calculate the size of all folders (valid until Ctrl+R or leaving the folder)")),
            ("Alt+Space / Shift+Space", n!("calculate the size of the folder under the cursor, or of all selected ones (Shift+Space when the desktop takes Alt+Space, e.g. Cinnamon)")),
            (nc!("keys", "right-click → Open with"), n!("choose an application; Other application… = a custom command, remembered for the extension")),
            ("Ctrl+Shift+A", n!("permissions (rwx, octal, also recursive)")),
            ("Ctrl+Shift+N / P / S", n!("to clipboard: name / full path / sftp:// address (without password)")),
            (nc!("keys", "drag from a file manager"), n!("upload / copy into the panel (onto a folder row = into it)")),
            ("F7", n!("new folder")),
            ("F8 / Delete", n!("delete")),
            ("Ctrl+F7 / Ctrl+F", n!("find files")),
        ],
    ),
    (
        n!("Tabs and servers"),
        &[
            ("Ctrl+O", n!("server manager")),
            ("Ctrl+L", n!("local disk in the panel")),
            ("Ctrl+T", n!("new tab: home folder, favorites, servers, recent places")),
            ("Ctrl+W", n!("close the tab")),
            ("Ctrl+D", n!("add the folder to the favorites or remove it")),
            ("Ctrl+Tab", n!("tabs of the active panel by last use")),
            ("Ctrl+PgUp / PgDn", n!("previous / next tab")),
            ("Ctrl+1 … 9", n!("jump to a tab in the active panel")),
            ("Ctrl+K", n!("find a tab (in both panels) or a server")),
            ("Ctrl+P", n!("terminal in the current folder (on a server: ssh into it, with the saved password)")),
            (nc!("keys", "double-click a tab"), n!("rename the tab")),
            (nc!("keys", "right-click a tab"), n!("tab color, Rename tab")),
        ],
    ),
    (
        n!("Viewer (F3)"),
        &[
            ("Esc / Q", n!("close the viewer")),
            ("1 / 2 / 3", n!("text / hexadecimal / image")),
            ("Ctrl+F / F7", n!("find in the file (text, case, whole words, regular expression)")),
            ("F3 / Shift+F3", n!("next / previous hit")),
            ("Ctrl+G", n!("go to a line, a percentage (50 %) or an offset (0x1F00)")),
            ("Ctrl+Home / Ctrl+End", n!("beginning / end of the file")),
            ("W / N / F", n!("wrap lines / line numbers / follow the end of the file")),
            ("Ctrl+C", n!("copy the selection (Shift+arrows or the mouse select)")),
            ("Ctrl++ / Ctrl+− / Ctrl+wheel", n!("larger / smaller text")),
            ("Ctrl+R", n!("reload the file")),
            ("F4", n!("edit the file (as F4 in the panel)")),
            ("← → PgUp PgDn", n!("image: previous / next image in the folder")),
            ("F / 0 / + / −", n!("image: fit to window / 100 % / zoom in / zoom out (also the wheel; drag moves)")),
            ("R / L", n!("image: rotate right / left (only the view)")),
        ],
    ),
    (
        n!("In search results"),
        &[
            ("Enter", n!("open the folder of the result")),
            ("Esc", n!("stop the search / back to the folder")),
            ("Ctrl+R", n!("search again")),
        ],
    ),
];

/// Returns true when the dialog should close.
/// `keys`: the keyboard shortcuts page (else the guide from `help/<lang>.md`).
pub fn help_ui(ctx: &egui::Context, keys: &mut bool) -> bool {
    let mut close = false;
    let page = crate::help::text(&rolid_i18n::current_language());
    let max_h = (ctx.content_rect().height() - 260.0).clamp(200.0, 620.0);
    let w = 700.0_f32.min(ctx.content_rect().width() - 60.0);
    let role = crate::look::Role::Info;
    crate::dialog::window("help-modal", tr!("Help")).icon(role).min_width(w).max_width(w).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(keys, false, RichText::new(crate::help::title(page).unwrap_or_else(|| tr!("Help"))).heading());
            ui.selectable_value(keys, true, RichText::new(tr!("Keyboard shortcuts")).heading());
        });
        ui.separator();
        if !*keys {
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| crate::help::show(ui, page));
        } else {
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            for (section, rows) in SHORTCUTS {
                ui.add_space(6.0);
                ui.label(RichText::new(t_str(section)).strong());
                egui::Grid::new(*section).num_columns(2).min_col_width(250.0).spacing([24.0, 3.0]).show(ui, |ui| {
                    for (k, v) in *rows {
                        // Plain key names are not marked: no translation in "keys".
                        ui.monospace(tc_str("keys", k));
                        ui.label(t_str(v));
                        ui.end_row();
                    }
                });
            }
        });
        }
        let b = crate::dialog::buttons(ui, [crate::dialog::close().default(true)]);
        close = b[0];
    });
    close
}

/// Bitcoin donation addresses; one of them is picked at random on first start
/// and remembered in `settings.toml`.
pub const DONATE_BTC: [&str; 4] = [
    "bc1q62vpce2flvycd93s96ldgzquug56rq0m5ja7yr",
    "bc1qgjgvvuee828zq9ppepah47ckt2skrv722n3n6r",
    "bc1qp552e5mg7edpdkq9pvmyvhclnau0gl2n4xn6ga",
    "bc1qw8y8clyq54uz9nk4v34jn7zrd33644uw9q9q0l",
];

/// The address to show: the stored one if it is in [`DONATE_BTC`]; a random one
/// when none is stored yet; the first one when the stored value is unknown
/// (e.g. edited by hand). Returns the address and whether the setting changed.
pub fn pick_donate_address(stored: Option<&str>) -> (&'static str, bool) {
    match stored {
        Some(s) => match DONATE_BTC.iter().find(|a| **a == s.trim()) {
            Some(a) => (a, s != *a),
            None => (DONATE_BTC[0], true),
        },
        None => {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0),
            );
            (DONATE_BTC[(h.finish() % DONATE_BTC.len() as u64) as usize], true)
        }
    }
}

/// Draw a QR code for `data` (black modules on a white quiet zone).
fn qr_code(ui: &mut egui::Ui, data: &str, size: f32) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        return;
    };
    let n = code.width();
    let colors = code.to_colors();
    let quiet = 2;
    let cell = size / (n + 2 * quiet) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, egui::Color32::WHITE);
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = ((i % n + quiet) as f32, (i / n + quiet) as f32);
            let min = rect.min + egui::vec2(x * cell, y * cell);
            // Slight overlap avoids hairline gaps between modules.
            p.rect_filled(
                egui::Rect::from_min_size(min, egui::Vec2::splat(cell + 0.5)),
                0.0,
                egui::Color32::BLACK,
            );
        }
    }
}

/// Public repository (GitHub).
pub const REPO_URL: &str = "https://github.com/MrRolid/RolidSCP";

/// The text of the license.
const GPL_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// Returns true when the dialog should close.
pub fn about_ui(ctx: &egui::Context, btc: &str, font_desc: &str) -> bool {
    let mut close = false;
    let title = format!("RolidSCP {}", crate::version::full_version());
    crate::dialog::window("about-modal", title).icon(crate::look::Role::Info).min_width(460.0).show(ctx, |ui| {
        crate::dialog::lead(ui, tr!("Dual-panel file manager for the local disk and SSH/SFTP."));
        ui.label(tr!("Built for {os} / {arch}", os = std::env::consts::OS, arch = std::env::consts::ARCH));
        ui.label(tr!("Configuration: {path}", path = rolid_config::config_dir()));
        ui.label(tr!(
            "Saved passwords: {path} (encrypted with the master password, Argon2id + XChaCha20-Poly1305)",
            path = rolid_config::vault::Vault::default_path()
        ));
        ui.horizontal(|ui| {
            let log = rolid_config::log_path();
            ui.label(tr!("Log: {path}", path = &log));
            if ui.small_button("📋").on_hover_text(tr!("Copy the log path")).clicked() {
                ui.ctx().copy_text(log.display().to_string());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // TRANSLATORS: followed by the link "license text" and "; in the package …".
            ui.label(tr!("© 2026 Martin Vorel (martin@vorel.eu). License: GNU GPL version 3 or later (GPL-3.0-or-later), "));
            // TRANSLATORS: a link to the GPL text, between "…(GPL-3.0-or-later), " and "; in the package …".
            ui.hyperlink_to(tr!("license text"), GPL_URL);
            // TRANSLATORS: follows the link "license text".
            ui.label(tr!("; in the package /usr/share/doc/rolidscp/copyright. No warranty of any kind."));
        });
        ui.horizontal(|ui| {
            ui.label(tr!("Source code:"));
            ui.hyperlink(REPO_URL);
        });
        ui.label(tr!("Icons: {set} – phosphoricons.com, built into the program", set = crate::icons::SET));
        ui.label(tr!(
            "Font: {font} (© 2015 Google LLC, {license}) and the monospace Hack 3.003 (MIT and Bitstream Vera), \
             built into the program; in use now: {current}",
            font = crate::fonts::NOTO_NAME,
            license = crate::fonts::NOTO_LICENSE,
            current = font_desc
        ));
        // News of all versions: scrolls so the dialog fits on small screens.
        egui::ScrollArea::vertical().max_height(crate::fonts::fs(36.0)).id_salt("about-news").show(ui, |ui| {
            ui.weak(RichText::new(tr!("What's new")).strong());
            for news in [
                trc!(
                    "news",
                    "0.8.14 data protection: on a production server copying, uploading, moving and a new folder \
                     never start with Overwrite and Enter does not confirm them; a move deletes the source only when \
                     it did not change meanwhile, and to a server only after the checksums match; an interrupted \
                     transfer is resumed only from a part file of the same source; a file replaced on a server \
                     without atomic rename is repaired at the next connection after a cut-off; F4 copies are kept \
                     on disk, a failed upload is tried again, the time of the upload has seconds; starts without \
                     system fonts; slow server shells no longer hold up search and checksums."
                ),
                trc!(
                    "news",
                    "0.8.13 listings refresh by themselves: local folders at once, server folders when you come \
                     back to them and every 30 s while the window is active (View → Automatic refresh), the cursor \
                     and the marks stay; thin lines between the columns, drag them to change the width (a double \
                     click fits the content), kept for each panel; names that do not fit end with “…”; the disk \
                     health badge shows the reason; dragging files into Thunar works; hovering changes only colors."
                ),
                trc!(
                    "news",
                    "0.8.12 a bar above each panel: the folders on the path and the disks in a drop-down (Alt+↓), \
                     back, forward, up, root, home, refresh and favorites; the free space, file system and disk \
                     health moved into the path row; a right click on the empty space of a panel offers a new file, \
                     a new folder, a terminal and the disk properties."
                ),
                trc!(
                    "news",
                    "0.8.11 files and folders can be dragged out of the window to other applications (X11), from a \
                     server or an archive they are downloaded first; Shift+F4 creates a new file in the external \
                     editor; favorite folders can be added, renamed and removed in the new tab menu; stricter disk \
                     temperature and wear limits, the temperature is checked every 5 minutes."
                ),
                trc!(
                    "news",
                    "0.8.10 the free-space row shows the file system type and the disk health (SMART) with a detail \
                     in plain language; Ctrl+P opens a terminal in the panel's folder, on a server ssh with the saved \
                     password; the tab and server switcher moved to Ctrl+K."
                ),
                trc!(
                    "news",
                    "0.8.9 built-in viewer (F3) in a window of its own: files of any size, also on a server without \
                     downloading them whole, syntax highlighting, encodings, hex, following a growing log, searching \
                     with the ripgrep engine (on a server with grep over SSH) and images (JPEG, PNG, animated GIF, \
                     WebP, BMP) with zoom, rotation and next / previous image."
                ),
                trc!(
                    "news",
                    "0.8.8 new tab menu (Ctrl+T: home folder, favorites, servers, recent places), favorite folders \
                     (Ctrl+D), tab menu (duplicate, color, close others), dragging files inside the app (Shift moves, \
                     Esc cancels), saved passwords lock again after a set time, one look for all dialogs and a target \
                     history with Browse… when copying and moving."
                ),
                trc!(
                    "news",
                    "0.8.7 in eight languages: English, Czech, German, French, Polish, Spanish, Norwegian and Danish \
                     (by the system or in Settings, switches at once; your own translation in ~/.config/rolidscp/lang), \
                     numbers and dates written as the language does (or ISO), help on F1."
                ),
                trc!(
                    "news",
                    "0.8.6 sharper text: strong hinting puts the vertical strokes of letters on whole pixels \
                     (the default, together with higher contrast); table lines stay sharp at 125% and 150% scaling too."
                ),
                trc!(
                    "news",
                    "0.8.5 rename right in the list (F2, Shift+F6), move to the other panel (F6), also between disks \
                     and servers, rename a tab by double-clicking it, font size in whole points (default 14), font \
                     Hinting and Contrast options in Settings, the bottom bar adapts to the window width."
                ),
                trc!(
                    "news",
                    "0.8.4 GPL-3.0-or-later license and source code on GitHub, sharper Noto Sans font (system font \
                     and text size can be chosen in Settings), size of the folder under the cursor (Alt+Space or \
                     Shift+Space), more readable colors on the cursor row."
                ),
                trc!(
                    "news",
                    "0.8.3 Enter opens a file in the default application (for a program or script it asks: editor / \
                     Run), more precise drop target when dragging, compression on all cores (a No compression level; \
                     already compressed files are only stored), estimated time remaining, folders in [square brackets], \
                     selecting by mask also for folders, folder sizes (Alt+Shift+Enter, on a server via du), \
                     MD5/SHA/BLAKE3/CRC32 checksums saved in a format compatible with sha256sum -c."
                ),
                trc!(
                    "news",
                    "0.8.2 archives: pack (Alt+F5: zip, 7z, tar, tar.gz, tar.zst, AES-256 password for zip and 7z), \
                     extract (Alt+F9) with protection against zip-slip and zip bombs, archive test (Alt+Shift+F9), \
                     Enter opens an archive as a read-only folder (F5 copies out, also from a server)."
                ),
                trc!(
                    "news",
                    "0.8.1 toolbar, brief view and image thumbnails, folder history (Alt+←/→), selecting a group by \
                     mask, a distinct header for the active panel, free space, selection summary, file type icons and \
                     colors, exact drop target highlighting while dragging on X11."
                ),
                trc!(
                    "news",
                    "0.8.0 context menu (Open with, permissions, copying the path to the clipboard), default choice \
                     on conflict, consistent dialog look with icons, the queue shows “file 3 of 120”."
                ),
            ] {
                ui.weak(news);
            }
        });
        ui.separator();
        ui.label(RichText::new(tr!("❤ Support development")).strong().size(crate::fonts::fs(16.0)));
        ui.label(tr!(
            "RolidSCP is free and has no ads. If it saves you time, you can support its further development \
             with a bitcoin donation of any amount. Thank you!"
        ));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            qr_code(ui, &format!("bitcoin:{btc}"), 170.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(tr!("Bitcoin (BTC):")).strong());
                ui.add(egui::Label::new(RichText::new(btc).monospace().size(crate::fonts::fs(12.0))).selectable(true).wrap());
                ui.add_space(4.0);
                if crate::look::btn(crate::look::Role::Copy, tr!("Copy address")).ui(ui).clicked() {
                    ui.ctx().copy_text(btc.to_string());
                }
                ui.hyperlink_to(tr!("₿ Open in wallet"), format!("bitcoin:{btc}"));
                ui.hyperlink_to(
                    tr!("🔎 Address on mempool.space"),
                    format!("https://mempool.space/address/{btc}"),
                );
                ui.add_space(4.0);
                ui.weak(tr!("Scan the QR code with a mobile wallet."));
            });
        });
        let b = crate::dialog::buttons(ui, [crate::dialog::close().default(true)]);
        close = b[0];
    });
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donate_address_selection() {
        // Stored valid address is kept.
        assert_eq!(pick_donate_address(Some(DONATE_BTC[2])), (DONATE_BTC[2], false));
        // Unknown / tampered address falls back to the first one.
        assert_eq!(pick_donate_address(Some("bc1qattacker")), (DONATE_BTC[0], true));
        // Nothing stored: a random one from the list, to be saved.
        let (a, changed) = pick_donate_address(None);
        assert!(DONATE_BTC.contains(&a) && changed);
    }
}
