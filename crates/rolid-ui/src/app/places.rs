//! Places: the "New tab" menu (+ and Ctrl+T), favorite folders (Ctrl+D),
//! recent places, the list picker shared with the Ctrl+K switcher, and the
//! tab menu and tooltip (duplicate, close others, the real server address).

use std::sync::Arc;

use eframe::egui::{self, Color32, Key, Modifiers, Stroke, Vec2};
use rolid_config::{Location, Places, Profiles, RemoteTarget};
use rolid_core::color::Rgb;
use rolid_core::filter;
use rolid_local::LocalFs;

use super::session::offline_pane;
use super::{rgb32, Action, App, Dialog, PaneRef, SwitchTo, Tab, TabId};
use crate::dialog;
use crate::look::Role;
use crate::pane::{text_on, Pane};

// ------------------------------------------------------------------ picker

/// One row of a [`picker`] list.
pub(super) struct PickRow {
    /// A heading shown above the row (the first row of a group).
    pub heading: Option<String>,
    pub label: String,
    /// Background (a tab or server color); `None` = a plain button.
    pub color: Option<Rgb>,
    pub kind: RowKind,
    /// A favorite's own name (prefilled when renaming).
    pub name: Option<String>,
}

/// What a row of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowKind {
    Plain,
    /// Favorite folder `n` (with Rename and Remove buttons).
    Favorite(usize),
}

/// Renaming or removing a favorite right in the list.
#[derive(Debug, Default)]
pub(super) struct FavEdit {
    /// The favorite being renamed, the typed name, and "focus the field".
    pub rename: Option<(usize, String, bool)>,
    /// The favorite whose removal waits for a second click (or Back).
    pub confirm: Option<usize>,
}

impl FavEdit {
    fn active(&self) -> bool {
        self.rename.is_some() || self.confirm.is_some()
    }
}

/// A change of the favorites made in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FavAction {
    Remove(usize),
    Rename(usize, String),
}

pub(super) enum Picked {
    Nothing,
    Row(usize),
    Fav(FavAction),
    Close,
}

/// Width of the rows of a [`picker`] (fixed, so the window never changes its size).
const PICKER_ROW_W: f32 = 540.0;

/// A searchable list in a modal window: typing filters (the caller builds
/// the rows from `query`), ↑ ↓ move, Enter picks, Esc or the ✕ closes. Used by the Ctrl+K switcher and the "New tab" menu.
/// Favorite rows can be renamed and removed in place (`fav`); then Enter
/// confirms and Esc goes back to the list.
#[allow(clippy::too_many_arguments)]
pub(super) fn picker(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    query: &mut String,
    cursor: &mut usize,
    focus: &mut bool,
    rows: &[PickRow],
    fav: &mut FavEdit,
) -> Picked {
    let editing = fav.active();
    let (down, up) = if editing {
        (false, false)
    } else {
        (ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)), ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)))
    };
    // Esc while renaming or asking: back to the list, the window stays.
    if editing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        *fav = FavEdit::default();
        *focus = true;
        return Picked::Nothing;
    }
    let enter = ctx.input(|i| i.key_pressed(Key::Enter));
    if down {
        *cursor = (*cursor + 1).min(rows.len().saturating_sub(1));
    }
    if up {
        *cursor = cursor.saturating_sub(1);
    }
    *cursor = (*cursor).min(rows.len().saturating_sub(1));
    let mut out = Picked::Nothing;
    let max_h = (ctx.content_rect().height() - 220.0).max(200.0);
    let cancel = dialog::window(("picker", id), title).icon(Role::Search).min_width(560.0).max_width(560.0).show(ctx, |ui| {
        let r = ui.add_enabled(!editing, egui::TextEdit::singleline(query).hint_text(tr!("type to search…")).desired_width(PICKER_ROW_W));
        if *focus && !editing {
            r.request_focus();
            *focus = false;
        }
        // One width for every row: the window keeps its size when a row turns
        // into a field or a question.
        let row_w = PICKER_ROW_W;
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            // Long labels are cut, never widen the window.
            ui.set_max_width(row_w);
            for (n, row) in rows.iter().enumerate() {
                if let Some(h) = &row.heading {
                    ui.add_space(4.0);
                    ui.weak(h);
                }
                let sel = n == *cursor;
                let (fill, fg) = match row.color {
                    Some(c) => (rgb32(c), text_on(rgb32(c))),
                    None => (ui.visuals().widgets.inactive.weak_bg_fill, ui.visuals().text_color()),
                };
                let stroke = if sel { Stroke::new(2.0, ui.visuals().strong_text_color()) } else { Stroke::NONE };
                let fav_n = match row.kind {
                    RowKind::Favorite(i) => Some(i),
                    RowKind::Plain => None,
                };
                if let Some(i) = fav_n.filter(|i| fav.rename.as_ref().is_some_and(|(r, _, _)| r == i)) {
                    // Renaming: a field with Save / Back.
                    ui.horizontal(|ui| {
                        let Some((_, name, focus_field)) = fav.rename.as_mut() else { return };
                        let w = (row_w - 150.0).max(120.0);
                        let r = ui.add(egui::TextEdit::singleline(name).hint_text(tr!("(name)")).desired_width(w));
                        if *focus_field {
                            r.request_focus();
                            *focus_field = false;
                        }
                        let save = ui.button(tr!("Save")).clicked() || (enter && r.lost_focus());
                        if save {
                            out = Picked::Fav(FavAction::Rename(i, name.clone()));
                        } else if ui.button(tr!("Back")).clicked() {
                            *fav = FavEdit::default();
                        }
                    });
                    continue;
                }
                if let Some(i) = fav_n.filter(|i| fav.confirm == Some(*i)) {
                    // The second click of removing.
                    ui.horizontal(|ui| {
                        let q = tr!("Remove “{place}” from the favorites?", place = row.label.as_str());
                        ui.allocate_ui_with_layout(Vec2::new((row_w - 160.0).max(120.0), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width((row_w - 160.0).max(120.0));
                            ui.add(egui::Label::new(&q).truncate()).on_hover_text(&q);
                        });
                        if ui.button(tr!("Remove")).clicked() || enter {
                            out = Picked::Fav(FavAction::Remove(i));
                        } else if ui.button(tr!("Back")).clicked() {
                            *fav = FavEdit::default();
                        }
                    });
                    continue;
                }
                let side_buttons = if fav_n.is_some() { 2.0 * 30.0 } else { 0.0 };
                ui.horizontal(|ui| {
                    let b = egui::Button::new(egui::RichText::new(&row.label).color(fg))
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .fill(fill)
                        .stroke(stroke)
                        .min_size(Vec2::new(row_w - side_buttons, 24.0));
                    // An exact size: a long label is cut, the row never grows.
                    let resp = ui.add_enabled_ui(!editing, |ui| ui.add_sized([row_w - side_buttons, 24.0], b)).inner.on_hover_text(&row.label);
                    if sel && (up || down) {
                        resp.scroll_to_me(None);
                    }
                    if resp.clicked() {
                        out = Picked::Row(n);
                    }
                    if let Some(i) = fav_n {
                        let small = |t: &str| egui::Button::new(t).min_size(Vec2::new(26.0, 24.0));
                        if ui.add_enabled(!editing, small("✏")).on_hover_text(tr!("Rename")).clicked() {
                            let name = row.name.clone().unwrap_or_default();
                            *fav = FavEdit { rename: Some((i, name, true)), confirm: None };
                        }
                        if ui.add_enabled(!editing, small("🗑")).on_hover_text(tr!("Remove from favorites")).clicked() {
                            *fav = FavEdit { rename: None, confirm: Some(i) };
                        }
                    }
                });
            }
            if rows.is_empty() {
                ui.weak(tr!("Nothing matches."));
            }
        });
        dialog::cancel_pressed(ui)
    });
    if editing {
        // Enter belongs to the field or the question, never to the list.
        return if cancel { Picked::Close } else { out };
    }
    if enter && !rows.is_empty() && matches!(out, Picked::Nothing) {
        out = Picked::Row(*cursor);
    }
    if matches!(out, Picked::Nothing) && (cancel || enter) {
        out = Picked::Close;
    }
    out
}

// ------------------------------------------------------- new tab menu items

/// What an item of the "New tab" menu opens.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum NewTabTo {
    Place(Location),
    Profile(String),
    /// The server manager (Ctrl+O).
    Servers,
    /// Add the active panel's folder to the favorites.
    AddFavorite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Home,
    Favorites,
    Servers,
    Recent,
    Manager,
}

#[derive(Debug, Clone)]
pub(super) struct NewTabItem {
    pub group: Group,
    pub label: String,
    pub color: Option<Rgb>,
    pub to: NewTabTo,
    /// Favorite folder number (rename and remove in the list).
    pub fav: Option<usize>,
    pub name: Option<String>,
}

/// How a place is shown: a local path, or "server: path".
pub(super) fn place_label(loc: &Location, profiles: &Profiles) -> String {
    match loc {
        Location::Local { path } => tilde(path, &App::home_dir()),
        Location::Remote { target, path } => {
            let server = target
                .profile
                .as_deref()
                .and_then(|id| profiles.get(id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("{}@{}", target.user, target.host));
            format!("🖧 {server}: {path}")
        }
    }
}

/// A local path with the home folder written as "~".
pub(super) fn tilde(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some(rest) if home.len() > 1 && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The color of the server of a place (from its profile).
pub(super) fn place_color(loc: &Location, profiles: &Profiles) -> Option<Rgb> {
    match loc {
        Location::Remote { target, .. } => target
            .profile
            .as_deref()
            .and_then(|id| profiles.get(id))
            .and_then(|p| p.color.as_deref())
            .and_then(Rgb::from_hex),
        Location::Local { .. } => None,
    }
}

/// The items of the "New tab" menu matching `query`, in groups: the home
/// folder, favorite folders (always there, with "Add the current folder"
/// last), saved servers, recent places (without the favorites) and the
/// server manager last (always there).
pub(super) fn new_tab_items(home: &str, places: &Places, profiles: &Profiles, query: &str) -> Vec<NewTabItem> {
    let mut groups: Vec<Vec<(i32, NewTabItem)>> = Vec::new();
    let mut add = |score: i32, item: NewTabItem| match groups.last_mut() {
        Some(g) if g[0].1.group == item.group => g.push((score, item)),
        _ => groups.push(vec![(score, item)]),
    };
    let mut push = |group: Group, hay: String, label: String, color: Option<Rgb>, to: NewTabTo, fav: Option<(usize, Option<String>)>| {
        let shown = match &to {
            // Always offered, whatever is typed.
            NewTabTo::AddFavorite => Some(i32::MIN),
            _ => filter::score(&hay, query),
        };
        if let Some(score) = shown {
            let (fav, name) = match fav {
                Some((n, name)) => (Some(n), name),
                None => (None, None),
            };
            add(score, NewTabItem { group, label, color, to, fav, name });
        }
    };
    let home_loc = Location::Local { path: home.to_string() };
    push(Group::Home, format!("{} {home}", tr!("Home folder")), tr!("⌂ Home folder ({path})", path = home), None, NewTabTo::Place(home_loc), None);
    for (i, f) in places.favorites.iter().enumerate() {
        let path = place_label(&f.location, profiles);
        let label = match &f.name {
            Some(n) => format!("★ {n}   ({path})"),
            None => format!("★ {path}"),
        };
        let hay = format!("{} {path} {}", f.name.as_deref().unwrap_or(""), f.location.path());
        push(Group::Favorites, hay, label, place_color(&f.location, profiles), NewTabTo::Place(f.location.clone()), Some((i, f.name.clone())));
    }
    push(Group::Favorites, String::new(), tr!("＋ Add the current folder"), None, NewTabTo::AddFavorite, None);
    for p in &profiles.list {
        let hay = format!("{} {}@{}", p.name, p.user, p.host);
        let label = format!("🖧 {}   ({}@{})", p.name, p.user, p.host);
        let color = Some(p.color.as_deref().and_then(Rgb::from_hex).unwrap_or(Rgb(0xE8, 0xE6, 0xE1)));
        push(Group::Servers, hay, label, color, NewTabTo::Profile(p.id.clone()), None);
    }
    for l in places.recent.iter().filter(|l| !places.favorites.iter().any(|f| f.location.same_place(l))) {
        let label = place_label(l, profiles);
        push(Group::Recent, format!("{label} {}", l.path()), format!("↺ {label}"), place_color(l, profiles), NewTabTo::Place(l.clone()), None);
    }
    let mut out: Vec<NewTabItem> = Vec::new();
    for mut g in groups {
        g.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        out.extend(g.into_iter().map(|(_, i)| i));
    }
    out.push(NewTabItem { group: Group::Manager, label: tr!("🖧 Server manager…"), color: None, to: NewTabTo::Servers, fav: None, name: None });
    out
}

fn group_heading(g: Group) -> String {
    match g {
        Group::Favorites => tr!("Favorite folders"),
        Group::Servers => tr!("Saved servers"),
        Group::Recent => tr!("Recently used"),
        Group::Home | Group::Manager => String::new(),
    }
}

/// Rows for the picker, with a heading above the first item of each group.
fn rows(items: &[NewTabItem]) -> Vec<PickRow> {
    let mut last = None;
    items
        .iter()
        .map(|i| {
            let heading = (last != Some(i.group)).then(|| group_heading(i.group)).filter(|h| !h.is_empty());
            last = Some(i.group);
            let kind = i.fav.map_or(RowKind::Plain, RowKind::Favorite);
            PickRow { heading, label: i.label.clone(), color: i.color, kind, name: i.name.clone() }
        })
        .collect()
}

/// Rename or remove a favorite (from the "New tab" menu). The "*" list and
/// Ctrl+D read the same list, so they show the change at once.
pub(super) fn apply_fav_action(places: &mut Places, a: FavAction) {
    match a {
        FavAction::Remove(i) => {
            places.remove_favorite(i);
        }
        FavAction::Rename(i, name) => places.rename_favorite(i, &name),
    }
}

impl Tab {
    /// A copy of the tab for "Duplicate tab": the same place, color, name and
    /// listing style (inside an archive: the folder containing it).
    pub fn duplicate(&self, id: TabId) -> Tab {
        let (fs, path, remote, offline) = self.pane.real_location();
        let mut pane = Pane::new(fs, path);
        pane.remote = remote;
        pane.offline = offline;
        pane.style = self.pane.style;
        Tab { id, name: self.name.clone(), color: self.color, pane }
    }
}

// ---------------------------------------------------------------- the app

/// The favorites dialog: the row whose name is being edited.
#[derive(Default)]
pub(super) struct FavoritesForm {
    pub focus: Option<usize>,
}

impl App {
    /// "+" or Ctrl+T: the "New tab" menu for panel `side`.
    pub(super) fn open_new_tab_menu(&mut self, side: usize) {
        self.active_side = side;
        self.dialog = Dialog::NewTab { side, query: String::new(), cursor: 0, focus: true, fav: FavEdit::default() };
    }

    pub(super) fn new_tab_menu_ui(&mut self, ctx: &egui::Context) {
        let Dialog::NewTab { side, query, cursor, focus, fav } = &mut self.dialog else { return };
        let side = *side;
        let items = new_tab_items(&App::home_dir(), &self.places, &self.profiles, query);
        let rows = rows(&items);
        let title = if side == 0 { tr!("New tab in the left panel") } else { tr!("New tab in the right panel") };
        match picker(ctx, "new-tab", &title, query, cursor, focus, &rows, fav) {
            Picked::Nothing => {}
            Picked::Close => self.dialog = Dialog::None,
            Picked::Fav(a) => {
                *fav = FavEdit::default();
                *focus = true;
                apply_fav_action(&mut self.places, a);
            }
            Picked::Row(n) if items[n].to == NewTabTo::AddFavorite => {
                // The window stays: the new favorite shows up in it.
                self.add_current_favorite();
            }
            Picked::Row(n) => {
                self.dialog = Dialog::None;
                let to = items[n].to.clone();
                match to {
                    NewTabTo::Place(loc) => self.open_place_tab(side, loc),
                    NewTabTo::Profile(id) => self.open_profile_in(&id, side, false),
                    NewTabTo::Servers => {
                        self.active_side = side;
                        self.open_connect();
                    }
                    NewTabTo::AddFavorite => {}
                }
            }
        }
    }

    /// Ctrl+K: tabs of both panels first, then saved servers (opening one
    /// reuses its tab if open).
    pub(super) fn switcher_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Switcher { query, cursor, focus } = &mut self.dialog else { return };
        let mut items: Vec<(i32, SwitchTo, PickRow)> = self
            .sides
            .iter()
            .enumerate()
            .flat_map(|(side, s)| s.tabs.iter().enumerate().map(move |(i, t)| (side, i, t)))
            .filter_map(|(side, i, t)| {
                let title = t.title();
                let hay = format!("{title} {}", t.pane.path);
                let label = if side == 0 { tr!("◧ left: {title}", title = title.as_str()) } else { tr!("◨ right: {title}", title = title.as_str()) };
                filter::score(&hay, query).map(|s| (s + 10, SwitchTo::Tab(side, i), PickRow { heading: None, label, color: Some(t.color), kind: RowKind::Plain, name: None }))
            })
            .chain(self.profiles.list.iter().filter_map(|p| {
                let hay = format!("{} {}@{}", p.name, p.user, p.host);
                let color = p.color.as_deref().and_then(Rgb::from_hex).unwrap_or(Rgb(0xE8, 0xE6, 0xE1));
                filter::score(&hay, query).map(|s| {
                    (s, SwitchTo::Profile(p.id.clone()), PickRow { heading: None, label: format!("🖧 {}   ({}@{})", p.name, p.user, p.host), color: Some(color), kind: RowKind::Plain, name: None })
                })
            }))
            .collect();
        items.sort_by_key(|it| std::cmp::Reverse(it.0));
        let (tos, rows): (Vec<SwitchTo>, Vec<PickRow>) = items.into_iter().map(|(_, to, row)| (to, row)).unzip();
        match picker(ctx, "switcher", &tr!("Switch to a tab or open a saved server"), query, cursor, focus, &rows, &mut FavEdit::default()) {
            Picked::Nothing | Picked::Fav(_) => {}
            Picked::Close => self.dialog = Dialog::None,
            Picked::Row(n) => {
                self.dialog = Dialog::None;
                self.switch_to(tos[n].clone());
            }
        }
    }

    /// Open a place as a new tab in panel `side` (a server connects at once;
    /// a remote place follows its profile when the server's address changed).
    pub(super) fn open_place_tab(&mut self, side: usize, loc: Location) {
        let (pane, color) = match loc {
            Location::Local { path } => {
                let path = if std::path::Path::new(&path).is_dir() { path } else { App::home_dir() };
                (Pane::new(Arc::new(LocalFs), path), None)
            }
            Location::Remote { mut target, path } => {
                let prof = target.profile.as_deref().and_then(|id| self.profiles.get(id)).cloned();
                if let Some(p) = &prof {
                    target = RemoteTarget {
                        profile: Some(p.id.clone()),
                        host: p.host.clone(),
                        port: p.port,
                        user: p.user.clone(),
                        key_path: p.key_path.clone(),
                    };
                }
                let color = prof.as_ref().and_then(|p| p.color.as_deref()).and_then(Rgb::from_hex);
                let pane = offline_pane(target, prof.as_ref().map(|p| p.name.clone()), prof.as_ref().is_some_and(|p| p.production), path);
                (pane, color)
            }
        };
        self.push_tab(side, pane, color, None);
        // Showing the tab connects a server tab.
        self.activate_tab(side, self.sides[side].active);
    }

    /// The favorites with their labels, for the "*" list.
    pub(super) fn favorite_labels(&self) -> Vec<(String, Location)> {
        self.places
            .favorites
            .iter()
            .map(|f| {
                let path = place_label(&f.location, &self.profiles);
                let label = match &f.name {
                    Some(n) => format!("{n}   ({path})"),
                    None => path,
                };
                (label, f.location.clone())
            })
            .collect()
    }

    /// Ctrl+D: the folder of panel `at` in or out of the favorites.
    pub(super) fn toggle_favorite(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        if p.search.is_some() {
            self.last_error = Some(tr!("Search results cannot be a favorite folder."));
            return;
        }
        let loc = p.location();
        let label = place_label(&loc, &self.profiles);
        let text = if self.places.toggle_favorite(loc) {
            tr!("Added to the favorite folders: {place}", place = label)
        } else {
            tr!("Removed from the favorite folders: {place}", place = label)
        };
        self.notice = Some((text, std::time::Instant::now()));
    }

    /// "Add the current folder" in the "New tab" menu: the active panel's
    /// folder (local or on a server) joins the favorites; never removes it.
    pub(super) fn add_current_favorite(&mut self) {
        let Some(p) = self.pane(self.active_ref()) else { return };
        if p.search.is_some() {
            self.last_error = Some(tr!("Search results cannot be a favorite folder."));
            return;
        }
        let loc = p.location();
        let label = place_label(&loc, &self.profiles);
        let text = if self.places.add_favorite(loc) {
            tr!("Added to the favorite folders: {place}", place = label)
        } else {
            tr!("Already among the favorite folders: {place}", place = label)
        };
        self.notice = Some((text, std::time::Instant::now()));
    }

    /// A favorite chosen in the "*" list of `at`: shown in that panel when it
    /// is on the same disk or server, otherwise opened as a new tab.
    pub(super) fn go_favorite(&mut self, at: PaneRef, n: usize) {
        let Some(f) = self.places.favorites.get(n).cloned() else { return };
        let Some(p) = self.pane(at) else { return };
        let here = p.location();
        let same_disk = p.archive.is_none() && p.search.is_none() && !p.offline && here.same_disk(&f.location);
        if same_disk {
            self.goto(at, f.location.path().to_string());
        } else {
            self.open_place_tab(at.side, f.location);
        }
    }

    /// A listing arrived: remember the place among the recent ones.
    pub(super) fn remember_place(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        if p.search.is_some() || p.offline {
            return;
        }
        let loc = p.location();
        self.places.remember(loc);
    }

    pub(super) fn open_favorites(&mut self) {
        self.dialog = Dialog::Favorites(FavoritesForm::default());
    }

    /// The favorites: rename, reorder, delete.
    pub(super) fn favorites_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Favorites(form) = &mut self.dialog else { return };
        let mut close = false;
        let (mut up, mut down, mut remove, mut open) = (None, None, None, None);
        let labels: Vec<String> = self.places.favorites.iter().map(|f| place_label(&f.location, &self.profiles)).collect();
        dialog::window("favorites", tr!("Favorite folders")).icon(Role::Folder).min_width(560.0).show(ctx, |ui| {
            dialog::note(ui, tr!("Ctrl+D adds or removes the folder of the active panel. An empty name shows the path."));
            if self.places.favorites.is_empty() {
                dialog::lead(ui, tr!("No favorite folders yet."));
            }
            let n = self.places.favorites.len();
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() * 0.6).max(200.0)).show(ui, |ui| {
                for (i, f) in self.places.favorites.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let mut name = f.name.clone().unwrap_or_default();
                        let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("(name)")).desired_width(160.0));
                        if form.focus == Some(i) {
                            r.request_focus();
                            form.focus = None;
                        }
                        if r.changed() {
                            f.name = (!name.trim().is_empty()).then_some(name);
                        }
                        let w = (ui.available_width() - 170.0).max(120.0);
                        ui.allocate_ui_with_layout(Vec2::new(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(w);
                            ui.add(egui::Label::new(&labels[i]).truncate()).on_hover_text(&labels[i]);
                        });
                        if ui.add_enabled(i > 0, egui::Button::new("⏶")).on_hover_text(tr!("Move up")).clicked() {
                            up = Some(i);
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("⏷")).on_hover_text(tr!("Move down")).clicked() {
                            down = Some(i);
                        }
                        if ui.button("🗑").on_hover_text(tr!("Remove from favorites")).clicked() {
                            remove = Some(i);
                        }
                        if ui.button(trc!("verb", "Open")).on_hover_text(tr!("Open in a new tab")).clicked() {
                            open = Some(i);
                        }
                    });
                }
            });
            let b = dialog::buttons(ui, [dialog::close()]);
            close |= b[0];
        });
        if let Some(i) = up {
            self.places.move_favorite(i, -1);
        }
        if let Some(i) = down {
            self.places.move_favorite(i, 1);
        }
        if let Some(i) = remove {
            self.places.favorites.remove(i);
        }
        if let Some(i) = open {
            let loc = self.places.favorites[i].location.clone();
            self.dialog = Dialog::None;
            self.open_place_tab(self.active_side, loc);
            return;
        }
        if close {
            self.dialog = Dialog::None;
        }
    }

    // ------------------------------------------------------------ tabs

    /// "Duplicate tab": a copy right after it, with the same color.
    pub(super) fn duplicate_tab(&mut self, side: usize, idx: usize) {
        let Some(t) = self.sides[side].tabs.get(idx) else { return };
        let id = self.next_id + 1;
        let tab = t.duplicate(id);
        self.next_id = id;
        let at = self.insert_tab(side, idx + 1, tab);
        self.activate_tab(side, self.sides[side].index(at.tab).unwrap_or(0));
    }

    /// "Close other tabs" of panel `side` (edited files may keep some open).
    pub(super) fn close_other_tabs(&mut self, side: usize, idx: usize) {
        let Some(keep) = self.sides[side].tabs.get(idx).map(|t| t.id) else { return };
        let others: Vec<TabId> = self.sides[side].tabs.iter().map(|t| t.id).filter(|id| *id != keep).collect();
        for id in others {
            if let Some(i) = self.sides[side].index(id) {
                self.close_tab(side, i);
            }
        }
        if let Some(i) = self.sides[side].index(keep) {
            self.activate_tab(side, i);
        }
    }

    /// The tooltip of a tab: the full path; for a server also the user, the
    /// server name, the address really connected to and the connection state.
    pub(super) fn tab_tooltip(&self, side: usize, t: &Tab) -> String {
        let p = &t.pane;
        let hint = tr!("Right button: menu · double-click: rename · middle button: close");
        let Some(r) = &p.remote else {
            let path = match &p.archive {
                Some(_) => format!("{} → {}", p.real_location().1, p.path),
                None => p.path.clone(),
            };
            return format!("{path}\n\n{hint}");
        };
        let at = PaneRef { tab: t.id, side };
        let link = self.pane_link(at);
        let peer = link.as_ref().and_then(|l| l.current()).and_then(|(_, c)| c.peer);
        let address = match peer {
            Some(a) => a.to_string(),
            None => tr!("{host}:{port} (not connected)", host = r.target.host.as_str(), port = r.target.port),
        };
        let server = r.name.clone().unwrap_or_else(|| r.target.host.clone());
        let state = if p.connecting {
            tr!("connecting…")
        } else if p.offline {
            tr!("not connected")
        } else {
            match &link {
                Some(l) => rolid_i18n::t(&l.state().text()),
                None => tr!("not connected"),
            }
        };
        let mut lines = vec![
            tr!("Server: {name}", name = server),
            tr!("User: {user}", user = r.target.user.as_str()),
            tr!("Address: {address}", address = address),
            tr!("Path: {path}", path = p.path.as_str()),
            tr!("State: {state}", state = state),
        ];
        if r.production {
            lines.push(tr!("Production server"));
        }
        format!("{}\n\n{hint}", lines.join("\n"))
    }

    /// The tab menu (right button on a tab). Returns the action chosen.
    pub(super) fn tab_menu(ui: &mut egui::Ui, side: usize, idx: usize, tab: &mut Tab, renaming: &mut Option<(TabId, String, bool)>) -> Option<Action> {
        let mut act: Option<Action> = None;
        if ui.button(tr!("Duplicate tab")).clicked() {
            act = Some(Box::new(move |a: &mut App| a.duplicate_tab(side, idx)));
            ui.close();
        }
        if ui.button(tr!("Rename tab")).clicked() {
            *renaming = Some((tab.id, tab.title(), true));
            ui.close();
        }
        if tab.name.is_some() && ui.button(tr!("Automatic name")).clicked() {
            tab.name = None;
            ui.close();
        }
        ui.menu_button(tr!("Change color"), |ui| color_menu(ui, tab));
        ui.separator();
        if ui.add(egui::Button::new(tr!("Close")).shortcut_text("Ctrl+W")).clicked() {
            act = Some(Box::new(move |a: &mut App| a.close_tab(side, idx)));
            ui.close();
        }
        if ui.button(tr!("Close other tabs")).clicked() {
            act = Some(Box::new(move |a: &mut App| a.close_other_tabs(side, idx)));
            ui.close();
        }
        act
    }
}

/// The color palette of a tab (only this tab changes).
fn color_menu(ui: &mut egui::Ui, tab: &mut Tab) {
    use rolid_core::color::PASTEL_PRESETS;
    egui::Grid::new(("palette", tab.id)).spacing([4.0, 4.0]).show(ui, |ui| {
        for (i, (name, c)) in PASTEL_PRESETS.iter().enumerate() {
            let sw = egui::Button::new("")
                .fill(rgb32(*c))
                .stroke(if *c == tab.color { Stroke::new(2.0, Color32::BLACK) } else { Stroke::new(1.0, Color32::GRAY) })
                .min_size(Vec2::new(24.0, 24.0));
            if ui.add(sw).on_hover_text(rolid_i18n::t_str(name)).clicked() {
                tab.color = *c;
            }
            if i % 8 == 7 {
                ui.end_row();
            }
        }
    });
    ui.collapsing(tr!("Custom color (16M shades)"), |ui| {
        let mut c = rgb32(tab.color);
        if egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque) {
            tab.color = Rgb(c.r(), c.g(), c.b());
        }
        ui.horizontal(|ui| {
            ui.label(tr!("Hex:"));
            // Keep the typed text while editing (partial input is not a valid color yet).
            let id = egui::Id::unique(("hex-edit", tab.id));
            let mut hex = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| tab.color.to_hex());
            let r = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(80.0));
            if r.changed() {
                if let Some(c) = Rgb::from_hex(&hex) {
                    tab.color = c;
                }
            }
            if r.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, hex));
            } else {
                ui.data_mut(|d| d.remove::<String>(id));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolid_config::{Favorite, Profile};

    fn profiles() -> Profiles {
        let mut p = Profiles::default();
        p.upsert(Profile {
            id: "p1".into(),
            name: "Web".into(),
            host: "web.example".into(),
            port: 22,
            user: "deploy".into(),
            color: Some("#FFD1DC".into()),
            ..Default::default()
        });
        p
    }

    fn remote(path: &str) -> Location {
        Location::Remote {
            target: RemoteTarget { profile: Some("p1".into()), host: "web.example".into(), port: 22, user: "deploy".into(), key_path: None },
            path: path.into(),
        }
    }

    fn places() -> Places {
        Places {
            favorites: vec![
                Favorite { name: Some("Projects".into()), location: Location::Local { path: "/home/u/projects".into() } },
                Favorite { name: None, location: remote("/var/www") },
            ],
            recent: vec![
                Location::Local { path: "/tmp".into() },
                Location::Local { path: "/home/u/projects".into() },
                remote("/etc"),
            ],
            targets: Vec::new(),
        }
    }

    #[test]
    fn new_tab_menu_lists_groups_in_order() {
        let items = new_tab_items("/home/u", &places(), &profiles(), "");
        let groups: Vec<Group> = items.iter().map(|i| i.group).collect();
        assert_eq!(
            groups,
            [Group::Home, Group::Favorites, Group::Favorites, Group::Favorites, Group::Servers, Group::Recent, Group::Recent, Group::Manager],
            "a recent favorite is not listed twice"
        );
        assert_eq!(items[0].to, NewTabTo::Place(Location::Local { path: "/home/u".into() }));
        assert_eq!((items[1].fav, items[2].fav), (Some(0), Some(1)), "favorites can be renamed and removed");
        assert_eq!(items[3].to, NewTabTo::AddFavorite, "“Add the current folder” ends the favorites");
        assert_eq!(items[4].to, NewTabTo::Profile("p1".into()));
        assert_eq!(items[4].color, Rgb::from_hex("#FFD1DC"), "a server has its color");
        assert_eq!(items[2].color, Rgb::from_hex("#FFD1DC"), "a remote favorite has its server's color");
        assert!(items[2].label.contains("Web: /var/www"), "{}", items[2].label);
        assert_eq!(items.last().unwrap().to, NewTabTo::Servers);
        let rows = rows(&items);
        assert!(rows[0].heading.is_none() && rows[1].heading.is_some() && rows[2].heading.is_none());
        assert!(rows.last().unwrap().heading.is_none());
        assert_eq!((rows[1].kind, rows[3].kind), (RowKind::Favorite(0), RowKind::Plain));
        assert_eq!(rows[1].name.as_deref(), Some("Projects"));
    }

    #[test]
    fn favorites_part_is_always_there() {
        // No favorites at all: the heading and "Add the current folder".
        let empty = Places::default();
        let items = new_tab_items("/home/u", &empty, &profiles(), "");
        let favs: Vec<&NewTabItem> = items.iter().filter(|i| i.group == Group::Favorites).collect();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].to, NewTabTo::AddFavorite);
        let rows = rows(&items);
        let at = items.iter().position(|i| i.to == NewTabTo::AddFavorite).unwrap();
        assert!(rows[at].heading.is_some(), "the part has its heading even when empty");
        // Typing something else: still offered.
        let items = new_tab_items("/home/u", &places(), &profiles(), "zzzz");
        assert!(items.iter().any(|i| i.to == NewTabTo::AddFavorite));
    }

    #[test]
    fn rename_and_remove_from_the_menu() {
        let mut p = places();
        let n = p.favorites.len();
        apply_fav_action(&mut p, FavAction::Rename(1, "Web".into()));
        assert_eq!(p.favorites[1].name.as_deref(), Some("Web"));
        let items = new_tab_items("/home/u", &p, &profiles(), "");
        assert!(items.iter().any(|i| i.label.starts_with("★ Web ")), "the menu shows the new name at once");
        apply_fav_action(&mut p, FavAction::Remove(0));
        assert_eq!(p.favorites.len(), n - 1);
        assert_eq!(p.favorites[0].name.as_deref(), Some("Web"));
        let items = new_tab_items("/home/u", &p, &profiles(), "");
        assert!(!items.iter().any(|i| i.label.contains("Projects")), "{items:?}");
        // Added again (as by "Add the current folder"), it is there once.
        assert!(p.add_favorite(Location::Local { path: "/home/u/projects".into() }));
        assert!(!p.add_favorite(Location::Local { path: "/home/u/projects/".into() }));
        assert_eq!(new_tab_items("/home/u", &p, &profiles(), "proj").iter().filter(|i| i.fav.is_some()).count(), 1);
    }

    #[test]
    fn typing_filters_the_new_tab_menu() {
        let items = new_tab_items("/home/u", &places(), &profiles(), "proj");
        assert_eq!(items.len(), 3, "{items:?}");
        assert_eq!(items[0].group, Group::Favorites);
        assert_eq!(items[1].to, NewTabTo::AddFavorite, "adding stays");
        assert_eq!(items[2].to, NewTabTo::Servers, "the server manager stays");
        let items = new_tab_items("/home/u", &places(), &profiles(), "deploy");
        assert!(items.iter().any(|i| i.to == NewTabTo::Profile("p1".into())));
        let items = new_tab_items("/home/u", &places(), &profiles(), "/etc");
        assert!(items.iter().any(|i| i.group == Group::Recent && i.to == NewTabTo::Place(remote("/etc"))), "{items:?}");
        assert!(!items.iter().any(|i| i.to == NewTabTo::Place(Location::Local { path: "/tmp".into() })));
        let items = new_tab_items("/home/u", &places(), &profiles(), "zzzz");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn home_is_shown_as_tilde() {
        assert_eq!(tilde("/home/u/projects", "/home/u"), "~/projects");
        assert_eq!(tilde("/home/u", "/home/u"), "~");
        assert_eq!(tilde("/home/user2", "/home/u"), "/home/user2");
        assert_eq!(tilde("/etc", "/"), "/etc");
    }

    #[test]
    fn duplicate_keeps_place_color_name_and_style() {
        let mut p = Pane::new(Arc::new(LocalFs), "/tmp".into());
        p.style = rolid_config::ViewStyle::Brief;
        let t = Tab { id: 1, name: Some("Logs".into()), color: Rgb(0x12, 0x34, 0x56), pane: p };
        let mut d = t.duplicate(2);
        assert_eq!(d.id, 2);
        assert_eq!(d.color, t.color);
        assert_eq!(d.name.as_deref(), Some("Logs"));
        assert_eq!(d.pane.path, "/tmp");
        assert_eq!(d.pane.style, rolid_config::ViewStyle::Brief);
        // The copy's color is its own.
        d.color = Rgb(1, 2, 3);
        assert_eq!(t.color, Rgb(0x12, 0x34, 0x56));
    }
}
