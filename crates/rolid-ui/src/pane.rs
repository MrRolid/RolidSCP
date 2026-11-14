//! One file panel: listing, cursor, marks, quick filter, sorting, rendering.

pub mod columns;
pub mod history;
pub mod inline;
pub mod kinds;
pub mod layout;
pub mod marks;
pub mod navbar;
pub mod thumbs;
pub mod views;

pub use views::ViewCtx;
use views::ItemColors;
use columns::{Col, Columns};
use rolid_config::ViewStyle;
use history::History;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Vec2};
use rolid_core::{filter, Entry, EntryKind, Vfs};
use rolid_i18n::rolid_text::txtn;

/// Height of a row in the detailed listing (scales with the text size).
pub fn row_h() -> f32 {
    crate::fonts::row_h()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    /// By extension (the optional Ext column).
    Ext,
}

/// A visible row: the virtual ".." entry or an index into `entries`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Up,
    Entry(usize),
}

pub struct Pane {
    pub fs: Arc<dyn Vfs>,
    pub path: String,
    pub entries: Vec<Entry>,
    pub view: Vec<Row>,
    pub cursor: usize,
    pub marked: Marks,
    pub filter: String,
    /// The filter `view` was made with while `entries` did not change since:
    /// a longer filter only needs to look at those rows.
    narrow: Option<String>,
    /// [`fingerprint`] of `entries` as listed (a refresh with the same one changes nothing).
    listed_print: Option<u64>,
    /// Counted up whenever `entries` or the computed folder sizes change.
    entries_gen: u64,
    sizes_gen: u64,
    /// The footer text of a big folder and what it was made from.
    footer: Option<((u64, u64, u64), String)>,
    /// Sums of all entries (for entries and sizes generation) and of the
    /// marked ones (for entries, marks and sizes generation).
    all_sums: Option<((u64, u64), Sums)>,
    marks_sums: Option<((u64, u64, u64), Sums)>,
    pub sort: SortKey,
    pub sort_asc: bool,
    pub loading: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    /// Incremented for each listing request; stale results are dropped.
    pub req_id: u64,
    pub loaded_at: Option<Instant>,
    /// Path being edited in the header (click on path).
    pub path_edit: Option<String>,
    /// Set for remote panels (also while disconnected).
    pub remote: Option<RemoteInfo>,
    /// The panel shows a remote location that is not connected yet.
    pub offline: bool,
    pub connecting: bool,
    /// Showing search results instead of a directory listing.
    pub search: Option<SearchView>,
    scroll_offset: f32,
    scroll_to_cursor: bool,
    viewport_h: f32,
    /// Folder rows on screen in the last frame (for dropping files onto them).
    pub drop_dirs: Vec<(Rect, String)>,
    /// Listing style of this tab.
    pub style: ViewStyle,
    /// Folders visited before / after the current one (Alt+← / Alt+→).
    pub history: History,
    /// Brief listing: first visible column and rows per column.
    brief_first: usize,
    brief_rows: usize,
    /// Thumbnail grid: tiles per row.
    grid_cols: usize,
    thumb_tile_h: f32,
    /// Set when the folder is left: previews still being made are dropped.
    pub thumb_cancel: Arc<AtomicBool>,
    /// Free / total space of the shown folder's disk (computed in the background).
    pub space: Option<rolid_core::vfs::Space>,
    /// File system type of that disk (`ext4`, `nfs4`…), when known.
    pub fstype: Option<String>,
    /// The physical disks behind the folder, or why their health is not read.
    pub disks: Option<Result<Vec<String>, rolid_disk::physical::NoSmart>>,
    /// The worst SMART state of those disks (when read).
    pub health: Option<crate::app::DiskBadge>,
    /// Inside an archive (read-only virtual folder): where to return.
    pub archive: Option<Box<ArchiveState>>,
    /// After the next listing: put the cursor on the first of these shown (drop).
    pub select_any: Option<Vec<String>>,
    /// Computed folder sizes (until Ctrl+R or leaving the folder).
    pub dir_sizes: HashMap<String, crate::dirsize::DirSize>,
    /// Folders whose size is being computed.
    pub dir_pending: HashSet<String>,
    /// Set when the folder is left or refreshed: the size computation stops.
    pub size_cancel: Arc<AtomicBool>,
    /// Renaming an item right in the list (F2).
    pub rename: Option<inline::InlineRename>,
    /// The drop-down tree of the path is open.
    pub tree_open: Option<navbar::TreeOpen>,
    /// Disks of this computer for the drop-down (`None` = not read yet).
    pub mounts: Option<Vec<navbar::MountRow>>,
}

/// A panel showing the inside of an archive remembers where it came from.
pub struct ArchiveState {
    pub fs: Arc<crate::archive_fs::ArchiveFs>,
    pub back_fs: Arc<dyn Vfs>,
    pub back_remote: Option<RemoteInfo>,
    /// Folder containing the archive and the archive's name in it.
    pub back_path: String,
    pub back_name: String,
    pub back_history: History,
    pub back_offline: bool,
    /// An archive opened from inside another archive returns into it.
    pub back_archive: Option<Box<ArchiveState>>,
}

#[derive(Clone, Debug)]
pub struct RemoteInfo {
    pub target: rolid_config::RemoteTarget,
    /// Saved profile name, if opened from a profile.
    pub name: Option<String>,
    pub production: bool,
}

/// Search results shown in a panel ("feed to listbox"). Entries are named by
/// their path relative to the search root; `hits` maps that name to the hit.
pub struct SearchView {
    pub id: u64,
    pub query: rolid_search::Query,
    pub hits: HashMap<String, rolid_search::Hit>,
    /// Filled by the search task, drained into the panel every frame.
    pub pending: Arc<Mutex<Vec<rolid_search::Hit>>>,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub summary: Option<String>,
}

impl SearchView {
    pub fn describe(&self) -> String {
        let q = &self.query;
        let mut parts = Vec::new();
        if !q.name.trim().is_empty() {
            parts.push(tr!("“{name}”", name = q.name.trim()));
        }
        if !q.content.is_empty() {
            parts.push(tr!("contains “{text}”", text = &q.content));
        }
        if parts.is_empty() {
            parts.push(trc!("search", "everything"));
        }
        parts.join(" + ")
    }
}

/// Something to operate on: an entry in a directory (with display label).
#[derive(Clone, Debug)]
pub struct Target {
    pub dir: String,
    pub name: String,
    pub label: String,
    pub size: u64,
}

pub enum PaneEvent {
    Focus,
    Activate,
    GoTo(String),
    /// ".." button in the path row.
    Up,
    /// Chosen from the row's context menu (right click).
    Command(PaneCmd),
    /// Rename typed in the list: item `from` in folder `dir` gets name `to`.
    Rename { dir: String, from: String, to: String },
    /// Favorite folder `n` chosen in the "*" list.
    Favorite(usize),
    /// "*" list: add this folder to the favorites or remove it (Ctrl+D).
    ToggleFavorite,
    /// "*" list: the favorites dialog.
    OrganizeFavorites,
    /// A row is being dragged (the item under the mouse; when it is marked,
    /// all the marked items).
    DragStart(String),
    /// The free space or the disk badge was clicked: disk health detail.
    DiskInfo,
    /// Back (`false`) / Forward (`true`) in the folder history.
    History(bool),
    Refresh,
    /// Out of the archive, back to the folder it is in.
    LeaveArchive,
    /// The drop-down needs the list of disks.
    WantMounts,
    /// A folder chosen in the drop-down, with the cursor on `select`.
    Jump { path: String, select: Option<String> },
    /// A column separator dragged or double clicked: the column's width at
    /// text scale 1 (`None` = default); `done` when the drag ended (save).
    ColumnWidth { col: Col, width: Option<f32>, done: bool },
}

/// The name of the ".." row among the drop targets (no real file has it).
pub const UP: &str = "..";

/// Context menu commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneCmd {
    Open,
    OpenWith(Opener),
    /// The "Open with – other application" dialog.
    OpenWithDialog,
    /// F3: the viewer.
    View,
    Edit,
    /// Shift+F4: a new file in the editor.
    NewFile,
    Copy,
    Move,
    Rename,
    Delete,
    ToggleMark,
    Perms,
    CopyNames,
    CopyPaths,
    CopyUrls,
    Pack,
    Unpack,
    TestArchive,
    SizeSelected,
    SizeAll,
    Checksums,
    // The menu of the empty space in the list (the folder itself).
    /// F7.
    NewFolder,
    Refresh,
    AddFavorite,
    /// Ctrl+P.
    Terminal,
    /// Free space and SMART of the panel's disk.
    DiskProps,
}

/// What a file is opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opener {
    /// The external text editor (F4).
    Editor,
    /// The system's default application (xdg-open).
    System,
    /// A desktop application (desktop file ID).
    Desktop(String),
    /// A command with `{file}`.
    Command(String),
    /// Windows' own "Open with" chooser.
    WindowsChooser,
}

impl Opener {
    /// For messages: "the system's default application", "Gedit"…
    pub fn describe(&self) -> String {
        match self {
            Opener::Editor => tr!("text editor"),
            Opener::System => tr!("the system's default application"),
            Opener::Desktop(id) => rolid_edit::apps::AppDb::system()
                .app(id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| id.clone()),
            Opener::Command(c) => c.split_whitespace().next().map(str::to_string).unwrap_or_else(|| tr!("command")),
            Opener::WindowsChooser => tr!("the application chosen in Windows"),
        }
    }
}

/// Index of the first row (in display order) whose name is one of `wanted`.
pub fn first_in_view(shown: &[&str], wanted: &[String]) -> Option<usize> {
    shown.iter().position(|n| !n.is_empty() && wanted.iter().any(|w| w == n))
}

impl Pane {
    pub fn new(fs: Arc<dyn Vfs>, path: String) -> Self {
        Self {
            fs,
            path,
            entries: Vec::new(),
            view: Vec::new(),
            cursor: 0,
            marked: Marks::default(),
            filter: String::new(),
            narrow: None,
            listed_print: None,
            entries_gen: 0,
            sizes_gen: 0,
            footer: None,
            all_sums: None,
            marks_sums: None,
            sort: SortKey::Name,
            sort_asc: true,
            loading: false,
            status: None,
            error: None,
            req_id: 0,
            loaded_at: None,
            path_edit: None,
            remote: None,
            offline: false,
            connecting: false,
            search: None,
            scroll_offset: 0.0,
            scroll_to_cursor: true,
            viewport_h: 400.0,
            drop_dirs: Vec::new(),
            style: ViewStyle::Full,
            history: History::default(),
            brief_first: 0,
            brief_rows: 1,
            grid_cols: 1,
            thumb_tile_h: 100.0,
            thumb_cancel: Arc::new(AtomicBool::new(false)),
            dir_sizes: HashMap::new(),
            dir_pending: HashSet::new(),
            size_cancel: Arc::new(AtomicBool::new(false)),
            space: None,
            fstype: None,
            disks: None,
            health: None,
            archive: None,
            select_any: None,
            rename: None,
            tree_open: None,
            mounts: None,
        }
    }

    /// Profile name for remote panels opened from a profile, else the backend label.
    pub fn display_label(&self) -> String {
        match &self.remote {
            Some(RemoteInfo { name: Some(n), .. }) => n.clone(),
            Some(r) => format!("{}@{}", r.target.user, r.target.host),
            None => fs_label(&*self.fs),
        }
    }

    /// The panel switches into or out of an archive: `path` becomes the
    /// current folder without being a visit in the history.
    pub fn jump_to(&mut self, path: String) {
        self.rename = None;
        self.marked.clear();
        self.filter.clear();
        self.scroll_offset = 0.0;
        self.brief_first = 0;
        self.cursor = 0;
        self.space = None;
        self.fstype = None;
        self.thumb_cancel.store(true, Ordering::Relaxed);
        self.thumb_cancel = Arc::new(AtomicBool::new(false));
        self.path = path;
    }

    /// The real location outside any open archive: backend, folder, remote, offline.
    pub fn real_location(&self) -> (Arc<dyn Vfs>, String, Option<RemoteInfo>, bool) {
        let Some(mut a) = self.archive.as_deref() else {
            return (self.fs.clone(), self.path.clone(), self.remote.clone(), self.offline);
        };
        while let Some(b) = a.back_archive.as_deref() {
            a = b;
        }
        (a.back_fs.clone(), a.back_path.clone(), a.back_remote.clone(), a.back_offline)
    }

    /// The place shown, for tabs, favorites and recent places (inside an
    /// archive: the folder containing it).
    pub fn location(&self) -> rolid_config::Location {
        match self.real_location() {
            (_, path, Some(r), _) => rolid_config::Location::Remote { target: r.target, path },
            (_, path, None, _) => rolid_config::Location::Local { path },
        }
    }

    pub fn has_parent(&self) -> bool {
        // In search mode ".." returns to the directory.
        self.search.is_some() || self.archive.is_some() || self.fs.parent(&self.path).is_some()
    }

    // ------------------------------------------------------------ search

    /// Switch to search mode; returns the buffer the search task fills.
    pub fn begin_search(&mut self, id: u64, query: rolid_search::Query, cancel: Arc<AtomicBool>) -> Arc<Mutex<Vec<rolid_search::Hit>>> {
        self.exit_search();
        let pending = Arc::new(Mutex::new(Vec::new()));
        self.path = query.root.clone();
        self.search = Some(SearchView {
            id,
            query,
            hits: HashMap::new(),
            pending: pending.clone(),
            running: true,
            cancel,
            summary: None,
        });
        self.entries.clear();
        self.entries_changed();
        self.listed_print = None;
        self.marked.clear();
        self.filter.clear();
        self.error = None;
        self.loading = false;
        self.cursor = 0;
        self.scroll_offset = 0.0;
        self.rebuild_view();
        pending
    }

    /// Move hits from the task buffer into the panel. Returns true if any arrived.
    pub fn drain_search(&mut self) -> bool {
        let Some(sv) = &mut self.search else { return false };
        let new: Vec<_> = std::mem::take(&mut *sv.pending.lock().unwrap());
        if new.is_empty() {
            return false;
        }
        for h in new {
            let mut e = h.entry.clone();
            e.name = h.rel.clone();
            self.entries.push(e);
            sv.hits.insert(h.rel.clone(), h);
        }
        let keep = self.current_name();
        self.sort_entries();
        self.rebuild_view();
        if let Some(k) = keep {
            self.select_name(&k);
        } else {
            self.cursor = 0;
        }
        true
    }

    pub fn finish_search(&mut self, id: u64, summary: String) {
        self.drain_search();
        if let Some(sv) = &mut self.search {
            if sv.id == id {
                sv.running = false;
                sv.summary = Some(summary);
            }
        }
    }

    /// Leave search mode (cancels a running search). The caller reloads the listing.
    pub fn exit_search(&mut self) {
        if let Some(sv) = self.search.take() {
            sv.cancel.store(true, Ordering::Relaxed);
            self.entries.clear();
            self.entries_changed();
            self.listed_print = None;
            self.marked.clear();
            self.filter.clear();
            self.cursor = 0;
            self.loading = true; // the caller reloads the directory
            self.rebuild_view();
        }
    }

    /// The entry under the cursor as a target (also in search results), with its entry.
    pub fn current_target(&self) -> Option<(Target, Entry)> {
        let e = self.current_entry()?;
        let label = e.name.clone();
        match self.hit(&label) {
            Some(h) => Some((
                Target { dir: h.dir.clone(), name: h.entry.name.clone(), label, size: h.entry.size },
                h.entry.clone(),
            )),
            None if self.search.is_none() => Some((
                Target { dir: self.path.clone(), name: e.name.clone(), label, size: e.size },
                e.clone(),
            )),
            None => None,
        }
    }

    /// The image files of the listing in its order (for the viewer's
    /// next / previous image).
    pub fn image_targets(&self) -> Vec<(Target, Entry)> {
        self.view
            .iter()
            .filter_map(|r| match r {
                Row::Entry(i) => self.entries.get(*i),
                Row::Up => None,
            })
            .filter(|e| !e.is_dir_like() && thumbs::is_image(&e.name))
            .filter_map(|e| {
                let label = e.name.clone();
                match self.hit(&label) {
                    Some(h) => Some((Target { dir: h.dir.clone(), name: h.entry.name.clone(), label, size: h.entry.size }, h.entry.clone())),
                    None if self.search.is_none() => Some((Target { dir: self.path.clone(), name: e.name.clone(), label, size: e.size }, e.clone())),
                    None => None,
                }
            })
            .collect()
    }

    pub fn hit(&self, name: &str) -> Option<&rolid_search::Hit> {
        self.search.as_ref()?.hits.get(name)
    }

    /// Remove result rows (e.g. after deleting them).
    pub fn remove_results(&mut self, names: &[String]) {
        let Some(sv) = &mut self.search else { return };
        for n in names {
            sv.hits.remove(n);
            self.marked.remove(n);
        }
        self.entries.retain(|e| !names.contains(&e.name));
        self.entries_changed();
        self.listed_print = None;
        self.rebuild_view();
    }

    /// Operation targets (marked entries, or the one under the cursor) with their
    /// real directory, so they work in search results too.
    pub fn target_items(&self) -> Vec<Target> {
        self.targets()
            .into_iter()
            .filter_map(|n| {
                let size = self.entries.iter().find(|e| e.name == n).map(|e| e.size).unwrap_or(0);
                match self.hit(&n) {
                    Some(h) => Some(Target { dir: h.dir.clone(), name: h.entry.name.clone(), label: n, size }),
                    None if self.search.is_none() => {
                        Some(Target { dir: self.path.clone(), name: n.clone(), label: n, size })
                    }
                    None => None,
                }
            })
            .collect()
    }

    /// What a drag of the row `name` takes: all marked items when the row is
    /// marked, otherwise that one item.
    pub fn drag_items(&self, name: &str) -> Vec<Target> {
        if self.marked.contains(name) {
            return self.target_items();
        }
        let size = self.entries.iter().find(|e| e.name == name).map(|e| e.size).unwrap_or(0);
        match self.hit(name) {
            Some(h) => vec![Target { dir: h.dir.clone(), name: h.entry.name.clone(), label: name.to_string(), size }],
            None if self.search.is_none() && self.entries.iter().any(|e| e.name == name) => {
                vec![Target { dir: self.path.clone(), name: name.to_string(), label: name.to_string(), size }]
            }
            None => Vec::new(),
        }
    }

    /// Install a fresh listing. `select` = name to put the cursor on.
    #[cfg(test)]
    pub fn set_listing(&mut self, path: String, entries: Vec<Entry>, select: Option<&str>) {
        self.set_prepared(path, Prepared::new(entries, None), select);
    }

    /// [`Pane::set_listing`] with a listing prepared in the background.
    pub fn set_prepared(&mut self, path: String, listing: Prepared, select: Option<&str>) {
        let Prepared { entries, sorted, print } = listing;
        let was_search = self.search.is_some();
        self.exit_search();
        let same_dir = path == self.path && !was_search;
        let keep = if same_dir { self.current_name() } else { None };
        if !same_dir {
            self.marked.clear();
            self.filter.clear();
            self.scroll_offset = 0.0;
            self.brief_first = 0;
            self.thumb_cancel.store(true, Ordering::Relaxed);
            self.thumb_cancel = Arc::new(AtomicBool::new(false));
            self.clear_dir_sizes();
            if path != self.path {
                self.history.left(&self.path);
            }
        }
        self.path = path;
        drop_later(std::mem::replace(&mut self.entries, entries));
        self.listed_print = Some(print);
        self.loading = false;
        self.error = None;
        self.loaded_at = Some(Instant::now());
        self.forget_missing();
        self.sort_unless(sorted);
        self.rebuild_view();
        let want = select.map(str::to_owned).or(keep);
        self.cursor = 0;
        if let Some(w) = want {
            self.select_name(&w);
        }
        if let Some(any) = self.select_any.take() {
            let shown: Vec<&str> = self
                .view
                .iter()
                .map(|r| match r {
                    Row::Entry(i) => self.entries[*i].name.as_str(),
                    Row::Up => "",
                })
                .collect();
            if let Some(i) = first_in_view(&shown, &any) {
                self.cursor = i;
            }
        }
        self.scroll_to_cursor = true;
    }

    /// A refresh in the background (another program changed the folder, the
    /// interval, coming back to the panel): only what changed changes. The
    /// cursor stays on its item (on the next one when the item is gone), the
    /// marks, filter, scroll position, sort and computed folder sizes stay.
    /// Returns whether anything changed.
    #[cfg(test)]
    pub fn update_listing(&mut self, entries: Vec<Entry>) -> bool {
        self.update_prepared(Prepared::new(entries, None))
    }

    /// [`Pane::update_listing`] with a listing prepared in the background.
    pub fn update_prepared(&mut self, listing: Prepared) -> bool {
        self.loaded_at = Some(Instant::now());
        if self.search.is_some() || self.archive.is_some() {
            return false;
        }
        let Prepared { entries, sorted, print } = listing;
        if self.listed_print == Some(print) && self.entries.len() == entries.len() {
            drop_later(entries);
            return false;
        }
        let keep = self.current_name();
        let old_cursor = self.cursor;
        let scroll = self.scroll_to_cursor;
        drop_later(std::mem::replace(&mut self.entries, entries));
        self.listed_print = Some(print);
        self.error = None;
        self.forget_missing();
        self.sort_unless(sorted);
        self.rebuild_view();
        let at = keep.and_then(|n| self.find_row(&n, old_cursor));
        self.cursor = at.unwrap_or(old_cursor).min(self.view.len().saturating_sub(1));
        // The view does not move to the cursor (it may have been scrolled away on purpose).
        self.scroll_to_cursor = scroll;
        true
    }

    fn entries_changed(&mut self) {
        self.narrow = None;
        self.entries_gen += 1;
    }

    /// The footer: counts and sizes of all and of the marked items. In a big
    /// folder it is made again only when something it counts changed (a
    /// million names every frame made each frame take ~400 ms).
    pub(crate) fn footer_text(&mut self) -> String {
        if self.entries.len() < 20_000 {
            return selection_summary(&self.entries, &self.marked, &self.dir_sizes);
        }
        let key = (self.entries_gen, self.marked.generation(), self.sizes_gen);
        if let Some((k, text)) = &self.footer {
            if *k == key {
                return text.clone();
            }
        }
        // After marking one item or all of them only the change is counted
        // (looking up a million marked names took ~0.8 s for each change).
        let text = match (self.all_sums, self.mark_sums()) {
            (Some((k, all)), Some(m)) if k == (self.entries_gen, self.sizes_gen) => summary_text(&all, &m),
            _ => {
                let (all, m) = sums(&self.entries, &self.marked, &self.dir_sizes);
                self.all_sums = Some(((self.entries_gen, self.sizes_gen), all));
                self.marks_sums = Some(((self.entries_gen, self.marked.generation(), self.sizes_gen), m));
                summary_text(&all, &m)
            }
        };
        self.footer = Some((key, text.clone()));
        text
    }

    /// The sums of the marked items, if they are known for the current marks
    /// and listing.
    fn mark_sums(&self) -> Option<Sums> {
        if self.marked.is_empty() {
            return Some(Sums::default());
        }
        match self.marks_sums {
            Some((k, m)) if k == (self.entries_gen, self.marked.generation(), self.sizes_gen) => Some(m),
            _ => None,
        }
    }

    /// Change the marks with `f`, which tells which entries it marked and
    /// unmarked; the sums of the marked items follow when they were known
    /// (and no folder sizes are computed: those are counted in only by a
    /// full count).
    fn change_marks(&mut self, f: impl FnOnce(&mut Self, &mut dyn FnMut(usize, bool))) {
        let base = self.mark_sums().filter(|_| self.dir_sizes.is_empty());
        let mut changes: Vec<(usize, bool)> = Vec::new();
        f(self, &mut |i, marked| changes.push((i, marked)));
        if let Some(mut m) = base {
            for (i, marked) in changes {
                let e = &self.entries[i];
                if marked {
                    m.add(e);
                } else {
                    m.sub(e);
                }
            }
            self.marks_sums = Some(((self.entries_gen, self.marked.generation(), self.sizes_gen), m));
        }
    }

    /// Every entry is marked (known from the sums, without looking up names).
    fn all_marked_known(&self) -> bool {
        self.mark_sums().is_some_and(|m| m.count() == self.entries.len() && !self.entries.is_empty())
    }

    /// Marks and folder sizes of items that are gone are dropped.
    fn forget_missing(&mut self) {
        if self.marked.is_empty() && self.dir_sizes.is_empty() && self.dir_pending.is_empty() {
            return;
        }
        let names: HashSet<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
        self.marked.retain(|m| names.contains(m.as_str()));
        self.dir_sizes.retain(|m, _| names.contains(m.as_str()));
        self.dir_pending.retain(|m| names.contains(m.as_str()));
        self.sizes_gen += 1;
    }

    /// Sort `entries`, unless they already come sorted the panel's way.
    fn sort_unless(&mut self, sorted: Option<(SortKey, bool)>) {
        let by_size_with_sizes = self.sort == SortKey::Size && !self.dir_sizes.is_empty();
        if sorted == Some((self.sort, self.sort_asc)) && !by_size_with_sizes {
            self.entries_changed();
        } else {
            self.sort_entries();
        }
    }

    /// Forget the computed folder sizes and stop a computation (Ctrl+R, leaving the folder).
    pub fn clear_dir_sizes(&mut self) {
        self.size_cancel.store(true, Ordering::Relaxed);
        self.size_cancel = Arc::new(AtomicBool::new(false));
        self.dir_sizes.clear();
        self.sizes_gen += 1;
        self.dir_pending.clear();
    }

    /// A folder's size arrived: store it, re-sort when sorted by size.
    pub fn set_dir_size(&mut self, name: &str, size: Option<crate::dirsize::DirSize>) {
        self.dir_pending.remove(name);
        let Some(size) = size else { return };
        self.dir_sizes.insert(name.to_string(), size);
        self.sizes_gen += 1;
        if self.sort == SortKey::Size && self.search.is_none() {
            let keep = self.current_name();
            let first = self.cursor;
            self.sort_entries();
            self.rebuild_view();
            match keep {
                Some(k) => self.select_name(&k),
                None => self.cursor = first,
            }
        }
    }

    /// Size used for sorting: a folder's computed size, else the entry's.
    fn sort_size(&self, e: &Entry) -> u64 {
        if e.is_dir_like() {
            self.dir_sizes.get(&e.name).map_or(0, |s| s.bytes)
        } else {
            e.size
        }
    }

    fn sort_entries(&mut self) {
        let sizes: HashMap<String, u64> = if self.sort == SortKey::Size {
            self.entries.iter().filter(|e| e.is_dir_like()).map(|e| (e.name.clone(), self.sort_size(e))).collect()
        } else {
            HashMap::new()
        };
        let (all, marks) = (self.all_sums.filter(|(k, _)| *k == (self.entries_gen, self.sizes_gen)), self.mark_sums());
        self.entries = sorted(std::mem::take(&mut self.entries), self.sort, self.sort_asc, &sizes);
        self.entries_changed();
        // Sorting changes the order, not the sums.
        self.all_sums = all.map(|(_, a)| ((self.entries_gen, self.sizes_gen), a));
        self.marks_sums = marks.map(|m| ((self.entries_gen, self.marked.generation(), self.sizes_gen), m));
    }

    pub fn set_sort(&mut self, key: SortKey) {
        let keep = self.current_name();
        if self.sort == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort = key;
            self.sort_asc = key == SortKey::Name;
        }
        self.sort_entries();
        self.rebuild_view();
        if let Some(k) = keep {
            self.select_name(&k);
        }
    }

    pub fn rebuild_view(&mut self) {
        // Rows move: an edit in the list ends (it never jumps to another item).
        self.rename = None;
        let keep = self.current_name();
        let prev_view = std::mem::take(&mut self.view);
        if self.filter.is_empty() {
            // The old rows' memory is used again (a million rows).
            let mut view = prev_view;
            view.clear();
            view.reserve(self.entries.len() + 1);
            if self.has_parent() {
                view.push(Row::Up);
            }
            view.extend((0..self.entries.len()).map(Row::Entry));
            self.view = view;
        } else {
            // A longer filter matches only rows the shorter one matched.
            let narrowed = self.narrow.as_deref().is_some_and(|prev| self.filter.starts_with(prev));
            let candidates: Vec<usize> = if narrowed {
                prev_view.iter().filter_map(|r| if let Row::Entry(i) = r { Some(*i) } else { None }).collect()
            } else {
                (0..self.entries.len()).collect()
            };
            let (entries, query) = (&self.entries, self.filter.as_str());
            let scored = par_filter_map(&candidates, &|i| filter::score(&entries[i].name, query).map(|s| (s, i)));
            // Keep directory-first sort order among equal scores (indices are unique).
            let scored = par_sort_by(scored, &|a: &(i32, usize), b: &(i32, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.view
                .extend(scored.into_iter().map(|(_, i)| Row::Entry(i)));
        }
        self.narrow = (!self.filter.is_empty()).then(|| self.filter.clone());
        let near = self.cursor;
        self.cursor = 0;
        if !self.filter.is_empty() {
            // While filtering, the best match is selected.
        } else if let Some(k) = keep {
            if let Some(pos) = self.find_row(&k, near) {
                self.cursor = pos;
            }
        }
        self.clamp_cursor();
        self.scroll_to_cursor = true;
    }

    /// The row of entry `name`, looked for from row `near` outwards (after a
    /// refresh or sort the item is mostly close to where it was: a million
    /// names compared from the top took ~100 ms).
    fn find_row(&self, name: &str, near: usize) -> Option<usize> {
        let is = |r: usize| matches!(self.view[r], Row::Entry(i) if self.entries[i].name == name);
        let len = self.view.len();
        let near = near.min(len.checked_sub(1)?);
        for d in 0..len {
            let (lo, hi) = (near.checked_sub(d), near + d);
            if lo.is_none() && hi >= len {
                break;
            }
            if lo.is_some_and(is) {
                return lo;
            }
            if d > 0 && hi < len && is(hi) {
                return Some(hi);
            }
        }
        None
    }

    pub fn select_name(&mut self, name: &str) {
        if let Some(pos) = self.find_row(name, self.cursor) {
            self.cursor = pos;
            self.scroll_to_cursor = true;
        }
    }

    pub fn current_row(&self) -> Option<Row> {
        self.view.get(self.cursor).copied()
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        match self.current_row()? {
            Row::Entry(i) => self.entries.get(i),
            Row::Up => None,
        }
    }

    pub fn current_name(&self) -> Option<String> {
        self.current_entry().map(|e| e.name.clone())
    }

    /// Names to operate on: marked entries, or the entry under the cursor.
    pub fn targets(&self) -> Vec<String> {
        if !self.marked.is_empty() {
            let all = self.all_marked_known();
            let v: Vec<String> = self
                .entries
                .iter()
                .filter(|e| all || self.marked.contains(&e.name))
                .map(|e| e.name.clone())
                .collect();
            par_sort_by(v, &|a: &String, b: &String| a.cmp(b))
        } else {
            self.current_name().into_iter().collect()
        }
    }

    fn clamp_cursor(&mut self) {
        if self.view.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.view.len() {
            self.cursor = self.view.len() - 1;
        }
    }

    /// Cursor step of PgUp / PgDn in the current listing style.
    pub fn page_rows(&self) -> usize {
        match self.style {
            ViewStyle::Full => ((self.viewport_h / row_h()).floor() as usize).saturating_sub(1).max(1),
            ViewStyle::Brief => self.brief_rows.max(1),
            ViewStyle::Thumbs => {
                let tile_h = self.thumb_tile_h.max(1.0);
                ((self.viewport_h / tile_h).floor() as usize).max(1) * self.grid_cols.max(1)
            }
        }
    }

    /// ← / →: between columns in the brief listing, between tiles in the
    /// thumbnail grid. Returns false in the detailed listing (not handled).
    pub fn move_horiz(&mut self, d: isize) -> bool {
        let len = self.view.len();
        let to = match self.style {
            ViewStyle::Full => return false,
            ViewStyle::Brief => layout::brief_move(self.cursor, len, self.brief_rows.max(1), d),
            ViewStyle::Thumbs => layout::grid_move(self.cursor, len, self.grid_cols.max(1), d, 0),
        };
        self.cursor = to;
        self.scroll_to_cursor = true;
        true
    }

    /// ↑ / ↓: one row (in the thumbnail grid one row of tiles).
    pub fn move_vert(&mut self, d: isize) {
        if self.style == ViewStyle::Thumbs {
            self.cursor = layout::grid_move(self.cursor, self.view.len(), self.grid_cols.max(1), 0, d);
            self.scroll_to_cursor = true;
        } else {
            self.move_cursor(d);
        }
    }

    /// Mark (or unmark) every shown entry whose name matches `masks`
    /// (`*.jpg;*.png`). Folders only when `dirs`. Returns how many changed.
    pub fn mark_group(&mut self, masks: &str, mark: bool, dirs: bool) -> usize {
        let mut n = 0;
        self.change_marks(|p, changed| {
            let Pane { view, entries, marked, .. } = p;
            for r in view.iter() {
                let Row::Entry(i) = r else { continue };
                let e = &entries[*i];
                if (e.is_dir_like() && !dirs) || !marks::matches(masks, &e.name) {
                    continue;
                }
                let done = if mark { marked.insert(e.name.clone()) } else { marked.remove(&e.name) };
                if done {
                    changed(*i, mark);
                    n += 1;
                }
            }
        });
        n
    }

    /// Invert the marks of the shown files; folders too with `dirs` (otherwise
    /// left alone, as in TC).
    pub fn invert_selection(&mut self, dirs: bool) {
        self.change_marks(|p, changed| {
            let Pane { view, entries, marked, .. } = p;
            for r in view.iter() {
                let Row::Entry(i) = r else { continue };
                let e = &entries[*i];
                if e.is_dir_like() && !dirs {
                    continue;
                }
                let now = !marked.remove(&e.name);
                if now {
                    marked.insert(e.name.clone());
                }
                changed(*i, now);
            }
        });
    }

    /// Change the listing style; the cursor stays on the same item.
    pub fn set_style(&mut self, style: ViewStyle) {
        self.style = style;
        self.scroll_to_cursor = true;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.view.is_empty() {
            return;
        }
        let max = self.view.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
        self.scroll_to_cursor = true;
    }

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
        self.scroll_to_cursor = true;
    }

    pub fn cursor_end(&mut self) {
        self.cursor = self.view.len().saturating_sub(1);
        self.scroll_to_cursor = true;
    }

    pub fn toggle_mark_current(&mut self) {
        if let Some(Row::Entry(i)) = self.current_row() {
            self.toggle_mark(i);
        }
        self.move_cursor(1);
    }

    /// Mark or unmark entry `i` (Insert, Space, Ctrl+click).
    pub fn toggle_mark(&mut self, i: usize) {
        self.change_marks(|p, changed| {
            let name = &p.entries[i].name;
            let marked = !p.marked.remove(name);
            if marked {
                p.marked.insert(name.clone());
            }
            changed(i, marked);
        });
    }

    pub fn mark_all(&mut self) {
        let all_marked = if self.filter.is_empty() && self.all_marked_known() {
            true
        } else {
            self.view
                .iter()
                .filter_map(|r| match r {
                    Row::Entry(i) => Some(&self.entries[*i].name),
                    Row::Up => None,
                })
                .all(|n| self.marked.contains(n))
        };
        if all_marked {
            self.marked.clear();
            return;
        }
        self.change_marks(|p, changed| {
            p.marked.reserve(p.view.len());
            let Pane { view, entries, marked, .. } = p;
            for r in view.iter() {
                if let Row::Entry(i) = r {
                    if marked.insert(entries[*i].name.clone()) {
                        changed(*i, true);
                    }
                }
            }
        });
    }

    /// "/" (root), ".." (up), "~" (home) and "*" (favorite and recent folders) at the
    /// right of the path row.
    fn path_buttons(&mut self, ui: &mut egui::Ui, r: Rect, bw: f32, fg: Color32, favorites: &[(String, rolid_config::Location)], events: &mut Vec<PaneEvent>) {
        let cell = |i: usize| Rect::from_min_size(r.min + Vec2::new(i as f32 * bw, 0.0), Vec2::new(bw - 2.0, r.height()));
        let txt = |t: &str| egui::RichText::new(t).monospace().strong().size(crate::fonts::fs(13.0)).color(fg);
        let put = |ui: &mut egui::Ui, i: usize, t: &str| -> egui::Response {
            let b = egui::Button::new(txt(t)).frame_when_inactive(false).min_size(Vec2::new(bw - 2.0, r.height()));
            ui.scope(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(1.0, 0.0);
                ui.put(cell(i), b)
            })
            .inner
        };
        let small = |ui: &mut egui::Ui, i: usize, t: &str, tip: &str| -> bool { put(ui, i, t).on_hover_text(tip).clicked() };
        if small(ui, 0, "/", &tr!("Root /")) {
            events.push(PaneEvent::Focus);
            events.push(PaneEvent::GoTo("/".into()));
        }
        if small(ui, 1, "..", &tr!("Up one level (Backspace)")) {
            events.push(PaneEvent::Focus);
            events.push(PaneEvent::Up);
        }
        if small(ui, 2, "~", &tr!("Home folder")) {
            events.push(PaneEvent::Focus);
            events.push(PaneEvent::GoTo("~".into()));
        }
        let resp = put(ui, 3, "*").on_hover_text(tr!("Favorite and recent folders (Ctrl+D adds this folder)"));
        self.favorites_menu(&resp, favorites, events);
    }

    /// Free space (a thin bar and the text), the file system and the disk
    /// health badge in the path row, right-aligned at `right`; the text
    /// shortens so that it does not reach left of `min_left`. Returns the
    /// area (clickable when the disk is known).
    fn paint_space_info(&self, ui: &mut egui::Ui, p: &egui::Painter, hrect: Rect, right: f32, min_left: f32, fg: Color32) -> Option<(Rect, egui::Response)> {
        let space = self.space.filter(|s| s.total > 0);
        if space.is_none() && self.fstype.is_none() && self.health.is_none() {
            return None;
        }
        let font = crate::fonts::prop(12.0);
        let cy = hrect.center().y;
        let mut x = right;
        // The badge: a pill with the heartbeat icon (and the reason when not OK).
        if let Some(b) = &self.health {
            let w = crate::app::disk_badge_width(p, b, &font);
            let badge = Rect::from_min_max(egui::pos2(x - w, hrect.top() + 4.0), egui::pos2(x, hrect.bottom() - 4.0));
            crate::app::disk_paint_badge(p, badge, b, &font);
            x = badge.left() - 8.0;
        }
        let bar_w = if space.is_some() { 40.0 + 6.0 } else { 0.0 };
        let budget = (x - bar_w - min_left).max(0.0);
        let text = match space {
            Some(sp) => {
                let measure = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), fg).size().x;
                navbar::space_label(&human_size(sp.avail), &human_size(sp.total), self.fstype.as_deref(), budget, measure)
            }
            None => self.fstype.clone().unwrap_or_default(),
        };
        if !text.is_empty() {
            let t = p.text(egui::pos2(x, cy), Align2::RIGHT_CENTER, text, font, fg);
            x = t.left() - 6.0;
        }
        if let Some(sp) = space {
            let used = 1.0 - sp.avail as f32 / sp.total as f32;
            let bar = Rect::from_min_size(egui::pos2(x - 40.0, cy - 2.0), Vec2::new(40.0, 4.0));
            p.rect_filled(bar, 2.0, fg.gamma_multiply(0.3));
            let fill = crate::app::disk_space_color(sp.avail, sp.total).unwrap_or(fg);
            p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(40.0 * used.clamp(0.0, 1.0), 4.0)), 2.0, fill);
            x = bar.left();
        }
        let area = Rect::from_min_max(egui::pos2(x - 2.0, hrect.top()), egui::pos2(right, hrect.bottom()));
        let clickable = self.disks.is_some();
        let resp = ui.interact(area, ui.scope_id().with("space-info"), if clickable { Sense::click() } else { Sense::hover() });
        let resp = if clickable {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            match &self.health {
                Some(b) => resp.on_hover_text(format!("{}\n{}", b.tip, tr!("Click for details."))),
                None => resp.on_hover_text(tr!("Disk health (SMART) – click for details")),
            }
        } else {
            resp
        };
        Some((area, resp))
    }

    /// Render the panel. Returns UI events for the app to handle.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        active: bool,
        accent: Color32,
        list_bg: Color32,
        view: &mut ViewCtx,
    ) -> Vec<PaneEvent> {
        let mut events = Vec::new();
        let mut drop_dirs = Vec::new();
        let full = ui.available_rect_before_wrap();
        if ui.rect_contains_pointer(full) && ui.input(|i| i.pointer.any_pressed()) {
            events.push(PaneEvent::Focus);
        }

        // --- The bar above the panel (drop-down tree, navigation buttons) ---
        if view.nav_bar {
            self.nav_bar(ui, full.width(), view.favorites, &mut events);
        }

        // --- Header: backend label + path (click to edit), free space, disk
        // badge and (without the bar) the path buttons ---
        let header_h = 26.0;
        let dark = ui.visuals().dark_mode;
        let (hbg, hfg) = header_colors(accent, active, dark);
        let (hrect, hresp) =
            ui.allocate_exact_size(Vec2::new(full.width(), header_h), Sense::click());
        let painter = ui.painter_at(hrect);
        painter.rect_filled(hrect, 3.0, hbg);
        if self.remote.as_ref().is_some_and(|r| r.production) {
            // Red hazard stripe: production must be unmistakable regardless of tab color.
            let stripe = Rect::from_min_max(
                hrect.left_bottom() - Vec2::new(0.0, 4.0),
                hrect.right_bottom(),
            );
            painter.rect_filled(stripe, 0.0, Color32::from_rgb(210, 30, 30));
        }
        // Path buttons on the right (only without the bar): / .. ~ *
        let bw = 30.0;
        let nbuttons = if view.nav_bar { 0.0 } else { 4.0 };
        let buttons = Rect::from_min_max(
            egui::pos2(hrect.right() - nbuttons * bw - 4.0, hrect.top() + 2.0),
            egui::pos2(hrect.right() - 2.0, hrect.bottom() - 2.0),
        );
        let icon = if self.fs.is_remote() { "🖧" } else { "🖴" };
        let mut label = match &self.archive {
            // The archive glyph is painted in front (Phosphor font).
            Some(_) => format!("     {}", tr!("Archive · read-only")),
            None => format!("{icon} {}", self.display_label()),
        };
        if self.remote.as_ref().is_some_and(|r| r.production) {
            label = format!("⚠ {}", tr!("PRODUCTION · {label}", label = label));
        }
        if self.connecting {
            label += &format!(" · {}", tr!("connecting…"));
        } else if self.offline {
            label += &format!(" · {}", tr!("not connected"));
        }
        let lgal = painter.layout_no_wrap(label, crate::fonts::prop(14.0), hfg);
        let label_w = lgal.size().x;
        // Free space, file system and disk badge, right-aligned before the buttons.
        let info_right = buttons.left() - 4.0;
        let info = if self.path_edit.is_none() {
            self.paint_space_info(ui, &painter, hrect, info_right, hrect.left() + 8.0 + label_w + 10.0 + 120.0, hfg)
        } else {
            None
        };
        let text_right = info.as_ref().map_or(info_right, |(r, _)| r.left() - 8.0);
        // The label and the path give way to the free space and the badge.
        let path_painter = ui.painter_at(Rect::from_min_max(hrect.min, egui::pos2(text_right, hrect.bottom())));
        path_painter.galley(hrect.left_center() + Vec2::new(8.0, -lgal.size().y / 2.0), lgal, hfg);
        if self.archive.is_some() {
            path_painter.text(
                hrect.left_center() + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                crate::icons::ARCHIVE,
                crate::icons::font(16.0),
                hfg,
            );
        }
        let path_x = hrect.left() + 8.0 + label_w + 10.0;
        // Without the bar Alt+↓ opens the tree under the path row.
        if !view.nav_bar && self.tree_open.is_some() {
            self.tree_popup(ui, Rect::from_min_size(hrect.min, Vec2::new(hrect.width().min(320.0), hrect.height())), &mut events);
        }
        if let Some(edit) = &mut self.path_edit {
            let r = Rect::from_min_max(
                egui::pos2(path_x, hrect.top() + 3.0),
                egui::pos2(info_right.max(path_x + 40.0), hrect.bottom() - 3.0),
            );
            let resp = ui.put(
                r,
                egui::TextEdit::singleline(edit).font(FontId::monospace(crate::fonts::fs(13.0))),
            );
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    events.push(PaneEvent::GoTo(edit.clone()));
                }
                self.path_edit = None;
            }
        } else {
            let header_text = match &self.search {
                Some(sv) => format!("🔍 {}", tr!("{query} in {path}", query = sv.describe(), path = &self.path)),
                None => self.path.clone(),
            };
            path_painter.text(
                egui::pos2(path_x, hrect.center().y),
                Align2::LEFT_CENTER,
                header_text,
                FontId::monospace(crate::fonts::fs(13.0)),
                hfg,
            );
            let info_clicked = info.as_ref().is_some_and(|(_, r)| r.clicked());
            if info_clicked {
                events.push(PaneEvent::DiskInfo);
            } else if hresp.clicked() {
                self.path_edit = Some(self.path.clone());
                events.push(PaneEvent::Focus);
            }
            if !view.nav_bar {
                self.path_buttons(ui, buttons, bw, hfg, view.favorites, &mut events);
            }
        }

        if self.style == ViewStyle::Full {
            // --- Column headers ---
            let (crect, _) = ui.allocate_exact_size(Vec2::new(full.width(), row_h()), Sense::hover());
            let cols = Columns::new(crect, view.ext_column, view.seconds, &view.widths);
            let hdr_color = ui.visuals().weak_text_color();
            let titles: HashMap<Col, String> = [
                (Col::Ext, trc!("column", "Ext")),
                (Col::Size, trc!("column", "Size")),
                (Col::Date, trc!("column", "Modified")),
                (Col::Mode, trc!("column", "Permissions")),
            ]
            .into_iter()
            .collect();
            let arrow = |k: SortKey| {
                if self.sort == k {
                    if self.sort_asc {
                        " ▲"
                    } else {
                        " ▼"
                    }
                } else {
                    ""
                }
            };
            for (rect, title, key, align) in [
                (
                    cols.name,
                    format!("{}{}", trc!("column", "Name"), arrow(SortKey::Name)),
                    Some(SortKey::Name),
                    Align2::LEFT_CENTER,
                ),
                (
                    cols.ext,
                    format!("{}{}", trc!("column", "Ext"), arrow(SortKey::Ext)),
                    Some(SortKey::Ext),
                    Align2::LEFT_CENTER,
                ),
                (
                    cols.size,
                    format!("{}{}", trc!("column", "Size"), arrow(SortKey::Size)),
                    Some(SortKey::Size),
                    Align2::RIGHT_CENTER,
                ),
                (
                    cols.date,
                    format!("{}{}", trc!("column", "Modified"), arrow(SortKey::Modified)),
                    Some(SortKey::Modified),
                    Align2::LEFT_CENTER,
                ),
                (cols.mode, trc!("column", "Permissions"), None, Align2::LEFT_CENTER),
            ] {
                if rect.width() < 1.0 {
                    continue;
                }
                let resp = ui.interact(
                    rect,
                    ui.scope_id().with(("col", rect.left() as i32)),
                    Sense::click(),
                );
                let pos = if align == Align2::RIGHT_CENTER {
                    rect.right_center() - Vec2::new(6.0, 0.0)
                } else {
                    rect.left_center() + Vec2::new(6.0, 0.0)
                };
                ui.painter_at(rect)
                    .text(pos, align, title, crate::fonts::prop(12.5), hdr_color);
                if let (Some(k), true) = (key, resp.clicked()) {
                    self.set_sort(k);
                }
            }
            // Separators: drag to resize the column on the right, double click to fit it.
            let line = views::separator_color(ui);
            let k = crate::fonts::scale();
            for (col, x) in cols.separators() {
                columns::separator(ui.painter(), x, crect.top(), crect.bottom(), line);
                let grip = Rect::from_min_max(egui::pos2(x - 4.0, crect.top()), egui::pos2(x + 4.0, crect.bottom()));
                let resp = ui.interact(grip, ui.scope_id().with(("col-sep", col)), Sense::click_and_drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if resp.double_clicked() {
                    let title = titles.get(&col).map(String::as_str).unwrap_or("");
                    let w = self.content_width(ui.painter(), col, title, view.seconds);
                    events.push(PaneEvent::ColumnWidth { col, width: Some(columns::fit(w, k)), done: true });
                } else if resp.dragged() || resp.drag_stopped() {
                    if let Some(p) = ui.ctx().pointer_interact_pos() {
                        let width = Some(cols.dragged(col, p.x, k));
                        events.push(PaneEvent::ColumnWidth { col, width, done: resp.drag_stopped() });
                    }
                }
            }

        }
        // --- Footer (reserve space) ---
        let footer_h = 20.0;
        let list_rect = {
            let r = ui.available_rect_before_wrap();
            Rect::from_min_max(r.min, egui::pos2(r.max.x, r.max.y - footer_h))
        };
        ui.painter().rect_filled(list_rect, 0.0, list_bg);

        // --- List ---
        let colors = ItemColors::new(ui, active, view);
        if self.loading && self.entries.is_empty() {
            ui.painter().text(
                list_rect.left_top() + Vec2::new(8.0, 6.0),
                Align2::LEFT_TOP,
                tr!("Loading…"),
                crate::fonts::prop(13.5),
                colors.text,
            );
        }
        if let Some(err) = &self.error {
            ui.painter().text(
                list_rect.left_top() + Vec2::new(8.0, 24.0),
                Align2::LEFT_TOP,
                format!("⚠ {err}"),
                crate::fonts::prop(13.5),
                Color32::from_rgb(200, 30, 30),
            );
        }
        // The empty space of the list: under the rows, which are added later
        // and so take the clicks on themselves.
        let bg = ui.interact(list_rect, ui.scope_id().with("list-bg"), Sense::click());
        self.folder_menu(&bg, &mut events);
        match self.style {
            ViewStyle::Full => self.show_full(ui, list_rect, &colors, &mut events, &mut drop_dirs),
            ViewStyle::Brief => self.show_brief(ui, list_rect, &colors, &mut events, &mut drop_dirs),
            ViewStyle::Thumbs => self.show_thumbs(ui, list_rect, &colors, view, &mut events, &mut drop_dirs),
        }
        self.end_unseen_rename();


        // --- Footer ---
        let frect = Rect::from_min_max(
            egui::pos2(full.left(), list_rect.bottom()),
            egui::pos2(full.right(), list_rect.bottom() + footer_h),
        );
        ui.allocate_rect(frect, Sense::hover());
        let mut footer = self.footer_text();
        if !self.dir_pending.is_empty() {
            footer += &format!("  |  {}", trn!("computing the size of {n} folder…", "computing the size of {n} folders…", self.dir_pending.len()));
        }
        if let Some(sv) = &self.search {
            footer += &match (&sv.summary, sv.running) {
                (_, true) => format!("  |  ⟳ {}", tr!("searching… ({count} found, Esc stops)", count = sv.hits.len())),
                (Some(s), false) => format!("  |  {s}"),
                (None, false) => String::new(),
            };
        }
        if !self.filter.is_empty() {
            footer += &format!(
                "  |  🔍 {}",
                trn!("filter: \"{filter}\" ({n} match, Esc clears)", "filter: \"{filter}\" ({n} matches, Esc clears)", self.view.len(), filter = &self.filter)
            );
        }
        if self.loading {
            footer += &format!("  |  ⟳ {}", tr!("loading…"));
        }
        if let Some(s) = &self.status {
            footer += &format!("  |  {s}");
        }
        let fp = ui.painter_at(frect);
        if !self.filter.is_empty() {
            fp.rect_filled(frect, 0.0, accent.gamma_multiply(0.6));
        }
        fp.text(
            frect.left_center() + Vec2::new(6.0, 0.0),
            Align2::LEFT_CENTER,
            footer,
            crate::fonts::prop(12.5),
            colors.text,
        );
        self.drop_dirs = drop_dirs;
        events
    }
}

/// The same entry as listed before (nothing to repaint).
/// Extension of a file name without the dot ("" for none or ".bashrc").
pub fn ext_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    }
}

/// Background and text of the path header: the active panel gets a strong,
/// saturated version of the tab color with white text, the inactive one a
/// muted one with dark text.
pub fn header_colors(accent: Color32, active: bool, dark: bool) -> (Color32, Color32) {
    use egui::ecolor::HsvaGamma;
    let t = HsvaGamma::from(accent);
    let hsv = |s: f32, v: f32| -> Color32 { HsvaGamma { h: t.h, s, v, a: 1.0 }.into() };
    if active {
        let s = if t.s < 0.05 { 0.0 } else { (t.s * 3.0).clamp(0.6, 0.9) };
        (hsv(s, 0.42), Color32::WHITE)
    } else if dark {
        (hsv((t.s * 0.6).min(0.35), 0.25), Color32::from_gray(205))
    } else {
        (hsv((t.s * 0.5).min(0.18), 0.92), Color32::from_gray(60))
    }
}

/// The label of a file system for the user: the local one says "Computer"
/// (marked `n!` in rolid-local), translated here.
pub fn fs_label(fs: &dyn rolid_core::vfs::Vfs) -> String {
    let l = fs.label();
    if l == "Computer" {
        tr!("Computer")
    } else {
        l
    }
}


/// Status line of a panel: "Selected: 3 of 95 files, 12.4 MB of 850 MB"
/// (folders are counted separately when some are selected); without a
/// selection the total count and size.
pub fn selection_summary(entries: &[Entry], marked: &HashSet<String>, dir_sizes: &HashMap<String, crate::dirsize::DirSize>) -> String {
    let (all, m) = sums(entries, marked, dir_sizes);
    summary_text(&all, &m)
}

/// Counts and sizes of items for the footer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sums {
    files: usize,
    dirs: usize,
    bytes: u64,
    /// Computed folder sizes are counted in; `ge`: one of them is incomplete.
    sized: bool,
    ge: bool,
}

impl Sums {
    /// One more item (without computed folder sizes).
    fn add(&mut self, e: &Entry) {
        if e.is_dir_like() {
            self.dirs += 1;
        } else {
            self.files += 1;
            self.bytes += e.size;
        }
    }

    /// One item less (without computed folder sizes).
    fn sub(&mut self, e: &Entry) {
        if e.is_dir_like() {
            self.dirs -= 1;
        } else {
            self.files -= 1;
            self.bytes -= e.size;
        }
    }

    fn count(&self) -> usize {
        self.files + self.dirs
    }
}

/// [`Sums`] of all items and of the marked ones.
fn sums(entries: &[Entry], marked: &HashSet<String>, dir_sizes: &HashMap<String, crate::dirsize::DirSize>) -> (Sums, Sums) {
    let (mut all, mut m) = (Sums::default(), Sums::default());
    let any_marked = !marked.is_empty();
    for e in entries {
        let marked = any_marked && marked.contains(&e.name);
        all.add(e);
        if marked {
            m.add(e);
        }
        if e.is_dir_like() {
            if let Some(d) = dir_sizes.get(&e.name) {
                for s in [Some(&mut all), marked.then_some(&mut m)].into_iter().flatten() {
                    s.bytes += d.bytes;
                    s.sized = true;
                    s.ge |= d.unreadable > 0;
                }
            }
        }
    }
    (all, m)
}

/// The footer text from [`sums`].
fn summary_text(all: &Sums, m: &Sums) -> String {
    let Sums { files, dirs, bytes: size, sized, ge } = *all;
    let Sums { files: mf, dirs: md, bytes: msize, sized: msized, ge: mge } = *m;
    let size_of = |n: u64, ge: bool| if ge { format!("≥ {}", human_size(n)) } else { human_size(n) };
    // "1 of 3 files", "1 of 1 folder" (the noun goes with the total).
    let of_files = || txtn!("{sel} of {n} file", "{sel} of {n} files", files, sel = mf);
    let of_dirs = || txtn!("{sel} of {n} folder", "{sel} of {n} folders", dirs, sel = md);
    if mf == 0 && md > 0 {
        if msized {
            tr!("Selected: {folders}, {size}", folders = of_dirs(), size = size_of(msize, mge))
        } else {
            tr!("Selected: {folders}", folders = of_dirs())
        }
    } else if mf + md > 0 {
        let (sel, total) = (size_of(msize, mge), size_of(size, ge));
        if md > 0 {
            tr!("Selected: {files}, {size} of {total} and {folders}", files = of_files(), size = sel, total = total, folders = of_dirs())
        } else {
            tr!("Selected: {files}, {size} of {total}", files = of_files(), size = sel, total = total)
        }
    } else {
        let f = txtn!("{n} file", "{n} files", files);
        let size = size_of(size, ge);
        if dirs == 0 {
            tr!("{files}, {size}", files = f, size = size)
        } else {
            let d = txtn!("{n} folder", "{n} folders", dirs);
            if sized {
                tr!("{files}, {size}, {folders} (with folder sizes)", files = f, size = size, folders = d)
            } else {
                tr!("{files}, {size}, {folders}", files = f, size = size, folders = d)
            }
        }
    }
}

pub fn text_on(bg: Color32) -> Color32 {
    let c = rolid_core::color::Rgb(bg.r(), bg.g(), bg.b()).text_color();
    Color32::from_rgb(c.0, c.1, c.2)
}

/// The marked names of a panel. Every change through `&mut` counts a
/// generation up, so that what is computed from the marks can be kept.
#[derive(Default, Debug)]
pub struct Marks {
    set: HashSet<String>,
    generation: u64,
}

impl Marks {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::ops::Deref for Marks {
    type Target = HashSet<String>;
    fn deref(&self) -> &HashSet<String> {
        &self.set
    }
}

impl std::ops::DerefMut for Marks {
    fn deref_mut(&mut self) -> &mut HashSet<String> {
        self.generation += 1;
        &mut self.set
    }
}

/// A listing made ready for a panel off the UI thread (sorting a million
/// names takes seconds): the entries, how they are sorted, their fingerprint.
pub struct Prepared {
    pub entries: Vec<Entry>,
    /// Sorted by this key and direction (`None` = as listed).
    pub sorted: Option<(SortKey, bool)>,
    pub print: u64,
}

impl Prepared {
    /// Fingerprint `entries` and sort them by `sort` (folders by size count 0:
    /// a panel with computed folder sizes sorts again).
    pub fn new(entries: Vec<Entry>, sort: Option<(SortKey, bool)>) -> Self {
        let print = fingerprint(&entries);
        let entries = match sort {
            Some((key, asc)) => sorted(entries, key, asc, &HashMap::new()),
            None => entries,
        };
        Prepared { entries, sorted: sort, print }
    }
}

/// The same for the same entries in any order; differs when a name, kind,
/// size, time or permissions differ.
pub fn fingerprint(entries: &[Entry]) -> u64 {
    use std::hash::{Hash, Hasher};
    entries.iter().fold(entries.len() as u64, |acc, e| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&e.name, e.kind, e.link_to_dir, e.size, e.modified, e.mode).hash(&mut h);
        acc.wrapping_add(h.finish())
    })
}

/// Free a big listing on another thread (a million names take tens of ms).
fn drop_later(entries: Vec<Entry>) {
    if entries.len() > 50_000 {
        std::thread::spawn(move || drop(entries));
    }
}

/// `entries` sorted the panel's way: folders first, then by `key`; `sizes` =
/// computed folder sizes (by name) for sorting by size.
fn sorted(entries: Vec<Entry>, key: SortKey, asc: bool, sizes: &HashMap<String, u64>) -> Vec<Entry> {
    let size_of = |e: &Entry| if e.is_dir_like() { sizes.get(&e.name).copied().unwrap_or(0) } else { e.size };
    // Text keys computed once (not in every comparison: 3 000 names would
    // build ~70 000 strings and make a refresh frame take ~20 ms).
    // Runs of digits are padded to the longest run of this listing (not to
    // 20 digits: shorter keys sort a million names faster, in the same order).
    let width = if key == SortKey::Name { entries.iter().map(|e| longest_number(&e.name)).max().unwrap_or(0).max(1) } else { 0 };
    let text_key = |e: &Entry| match key {
        SortKey::Name => natural_key(&e.name, width),
        SortKey::Ext => ext_of(&e.name).to_lowercase(),
        SortKey::Size | SortKey::Modified => String::new(),
    };
    par_sort_by_key(entries, &text_key, &|(ka, a): &(String, Entry), (kb, b): &(String, Entry)| {
        // Directories always first, like in Total Commander.
        let ord = b.is_dir_like().cmp(&a.is_dir_like());
        if ord.is_ne() {
            return ord;
        }
        let o = match key {
            SortKey::Name | SortKey::Ext => ka.cmp(kb),
            SortKey::Size => size_of(a).cmp(&size_of(b)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        }
        .then_with(|| a.name.cmp(&b.name));
        if asc {
            o
        } else {
            o.reverse()
        }
    })
}

/// Threads for sorting and filtering a big listing (a million names take
/// seconds on one).
fn threads_for(len: usize) -> usize {
    if len < 50_000 {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(8)
}

/// `v` sorted by `cmp` (stable), on several threads when it is big.
fn par_sort_by<T: Send>(v: Vec<T>, cmp: &(impl Fn(&T, &T) -> std::cmp::Ordering + Sync)) -> Vec<T> {
    let parts = split(v, threads_for);
    let parts: Vec<Vec<T>> = std::thread::scope(|s| {
        let hs: Vec<_> = parts
            .into_iter()
            .map(|mut p| {
                s.spawn(move || {
                    p.sort_by(cmp);
                    p
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("sort thread")).collect()
    });
    merge_all(parts, cmp)
}

/// `v` sorted by `cmp` on (key, item), the keys made once for each item (on
/// several threads when it is big).
fn par_sort_by_key<T: Send, K: Send>(
    v: Vec<T>,
    key: &(impl Fn(&T) -> K + Sync),
    cmp: &(impl Fn(&(K, T), &(K, T)) -> std::cmp::Ordering + Sync),
) -> Vec<T> {
    let parts = split(v, threads_for);
    let parts: Vec<Vec<(K, T)>> = std::thread::scope(|s| {
        let hs: Vec<_> = parts
            .into_iter()
            .map(|p| {
                s.spawn(move || {
                    let mut p: Vec<(K, T)> = p.into_iter().map(|e| (key(&e), e)).collect();
                    p.sort_by(cmp);
                    p
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("sort thread")).collect()
    });
    merge_all(parts, cmp).into_iter().map(|(_, e)| e).collect()
}

/// `v` cut into as many parts as `threads` says, in order.
fn split<T>(mut v: Vec<T>, threads: impl Fn(usize) -> usize) -> Vec<Vec<T>> {
    let n = threads(v.len());
    let size = v.len().div_ceil(n.max(1)).max(1);
    let mut parts = Vec::with_capacity(n);
    while v.len() > size {
        parts.push(v.split_off(v.len() - size));
    }
    parts.push(v);
    parts.reverse();
    parts
}

/// Sorted parts merged into one, pairs at the same time (stable: of equal
/// items the one from the earlier part comes first).
fn merge_all<T: Send>(mut parts: Vec<Vec<T>>, cmp: &(impl Fn(&T, &T) -> std::cmp::Ordering + Sync)) -> Vec<T> {
    while parts.len() > 1 {
        parts = std::thread::scope(|s| {
            let mut it = parts.into_iter();
            let mut hs = Vec::new();
            while let Some(a) = it.next() {
                match it.next() {
                    Some(b) => hs.push(s.spawn(move || merge(a, b, cmp))),
                    None => hs.push(s.spawn(move || a)),
                }
            }
            hs.into_iter().map(|h| h.join().expect("merge thread")).collect()
        });
    }
    parts.pop().unwrap_or_default()
}

fn merge<T>(a: Vec<T>, b: Vec<T>, cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) -> Vec<T> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter().peekable(), b.into_iter().peekable());
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        let next = if cmp(y, x) == std::cmp::Ordering::Less { b.next() } else { a.next() };
        out.extend(next);
    }
    out.extend(a);
    out.extend(b);
    out
}

/// `f` of each item of `items` that gives something, in order (on several
/// threads when there are many).
fn par_filter_map<R: Send>(items: &[usize], f: &(impl Fn(usize) -> Option<R> + Sync)) -> Vec<R> {
    let n = threads_for(items.len());
    if n == 1 {
        return items.iter().filter_map(|&i| f(i)).collect();
    }
    std::thread::scope(|s| {
        let hs: Vec<_> = items.chunks(items.len().div_ceil(n)).map(|c| s.spawn(move || c.iter().filter_map(|&i| f(i)).collect::<Vec<R>>())).collect();
        hs.into_iter().flat_map(|h| h.join().expect("filter thread")).collect()
    })
}

/// Lowercased name with digit runs zero-padded, for natural sort (file2 < file10).
#[cfg(test)]
fn natural_lower(s: &str) -> String {
    natural_key(s, 20)
}

/// Digits of the longest run of digits in `s`, without leading zeros.
fn longest_number(s: &str) -> usize {
    s.as_bytes()
        .split(|b| !b.is_ascii_digit())
        .map(|run| run.iter().skip_while(|&&b| b == b'0').count())
        .max()
        .unwrap_or(0)
}

/// [`natural_lower`] with digit runs padded to `width` digits (at least the
/// longest run of every name compared: keys of the same width sort the same
/// way as with 20).
fn natural_key(s: &str, width: usize) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        if !digits.is_empty() {
            let trimmed = digits.trim_start_matches('0');
            for _ in trimmed.len()..width {
                out.push('0');
            }
            out.push_str(trimmed);
            digits.clear();
        }
    };
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut digits, &mut out);
            if c.is_ascii() {
                out.push(c.to_ascii_lowercase());
            } else {
                out.extend(c.to_lowercase());
            }
        }
    }
    flush(&mut digits, &mut out);
    out
}

pub fn human_size(n: u64) -> String {
    rolid_i18n::size(n)
}

fn fmt_time(t: Option<SystemTime>, seconds: bool) -> String {
    t.map(|t| rolid_i18n::datetime(t, seconds)).unwrap_or_default()
}

fn fmt_mode(e: &Entry) -> String {
    let Some(m) = e.mode else {
        return String::new();
    };
    let t = match e.kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink => 'l',
        EntryKind::File => '-',
        EntryKind::Other => '?',
    };
    let mut s = String::with_capacity(10);
    s.push(t);
    for shift in [6, 3, 0] {
        let b = (m >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// The "Open with" submenu: the file type's applications (default first),
/// then the system chooser (Windows) and "Other application…".
pub fn open_with_items(name: &str) -> Vec<(PaneCmd, String)> {
    let db = rolid_edit::apps::AppDb::system();
    let mime = db.mime_for(name);
    let default = db.default_for(&mime).map(|a| a.id.clone());
    let mut v: Vec<(PaneCmd, String)> = db
        .apps_for(&mime)
        .into_iter()
        .map(|a| {
            let t = if Some(&a.id) == default.as_ref() { tr!("{app} (default)", app = &a.name) } else { a.name.clone() };
            (PaneCmd::OpenWith(Opener::Desktop(a.id.clone())), t)
        })
        .collect();
    v.push((PaneCmd::OpenWith(Opener::Editor), tr!("Text editor (F4)")));
    if cfg!(windows) {
        v.push((PaneCmd::OpenWith(Opener::WindowsChooser), tr!("Choose an application in Windows…")));
    }
    v.push((PaneCmd::OpenWithDialog, tr!("Other application…")));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort_key() {
        assert!(natural_lower("file2") < natural_lower("file10"));
        assert!(natural_lower("A") == natural_lower("a"));
    }

    fn ent(name: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            kind: if dir { EntryKind::Dir } else { EntryKind::File },
            link_to_dir: false,
            size: 10,
            modified: None,
            mode: None,
        }
    }

    fn pane_with(names: &[(&str, bool)]) -> Pane {
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        p.set_listing("/a".into(), names.iter().map(|(n, d)| ent(n, *d)).collect(), None);
        p
    }

    #[test]
    fn a_drag_takes_the_marked_items_or_the_one_row() {
        let mut p = pane_with(&[("d", true), ("a.txt", false), ("b.txt", false), ("c.txt", false)]);
        p.marked.insert("a.txt".into());
        p.marked.insert("c.txt".into());
        let names = |v: Vec<Target>| v.into_iter().map(|t| (t.dir, t.name)).collect::<Vec<_>>();
        let mut marked = names(p.drag_items("c.txt"));
        marked.sort();
        assert_eq!(marked, [("/a".to_string(), "a.txt".to_string()), ("/a".to_string(), "c.txt".to_string())]);
        assert_eq!(names(p.drag_items("b.txt")), [("/a".to_string(), "b.txt".to_string())], "an unmarked row goes alone");
        assert_eq!(names(p.drag_items("d")), [("/a".to_string(), "d".to_string())]);
        assert!(p.drag_items("nothing").is_empty());
    }

    #[test]
    fn sorting_by_each_column_in_both_directions() {
        let mut p = pane_with(&[("b10.txt", false), ("B2.md", false), ("a.TXT", false), ("zdir", true), ("adir", true), ("c", false)]);
        let names = |p: &Pane| p.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(" ");
        assert_eq!(names(&p), "adir zdir a.TXT B2.md b10.txt c", "natural, case-insensitive, folders first");
        p.set_sort(SortKey::Name);
        assert_eq!(names(&p), "zdir adir c b10.txt B2.md a.TXT", "descending: folders still first");
        // Another column starts descending; a second click turns it.
        p.set_sort(SortKey::Ext);
        assert_eq!(names(&p), "zdir adir b10.txt a.TXT B2.md c", "by extension, then by name");
        p.set_sort(SortKey::Ext);
        assert_eq!(names(&p), "adir zdir c B2.md a.TXT b10.txt");
        let t = |s: u64| Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(s));
        for (i, e) in p.entries.iter_mut().enumerate() {
            e.modified = t(100 - i as u64);
        }
        p.set_sort(SortKey::Modified);
        assert_eq!(names(&p), "adir zdir c B2.md a.TXT b10.txt", "newest first");
        p.set_sort(SortKey::Modified);
        assert_eq!(names(&p), "zdir adir b10.txt a.TXT B2.md c", "oldest first");
    }

    #[test]
    fn a_refresh_keeps_the_cursor_marks_and_sizes() {
        let mut p = pane_with(&[("folder", true), ("b.txt", false), ("c.txt", false), ("d.txt", false)]);
        p.select_name("c.txt");
        p.marked.insert("b.txt".into());
        p.marked.insert("d.txt".into());
        p.dir_sizes.insert("folder".into(), crate::dirsize::DirSize::default());
        p.scroll_offset = 40.0;
        p.scroll_to_cursor = false;
        let list = |names: &[(&str, bool)]| names.iter().map(|(n, d)| ent(n, *d)).collect::<Vec<_>>();

        // Nothing changed: nothing is touched.
        let before = (p.cursor, p.view.clone());
        assert!(!p.update_listing(list(&[("d.txt", false), ("c.txt", false), ("b.txt", false), ("folder", true)])));
        assert_eq!((p.cursor, p.view.clone()), before);

        // Another program added an item above the cursor and removed a marked one.
        assert!(p.update_listing(list(&[("folder", true), ("a.txt", false), ("c.txt", false), ("d.txt", false)])));
        assert_eq!(p.current_name().as_deref(), Some("c.txt"), "the cursor stays on its item");
        assert_eq!(p.marked.iter().map(String::as_str).collect::<Vec<_>>(), ["d.txt"], "marks of the remaining items stay");
        assert!(p.dir_sizes.contains_key("folder"), "computed sizes stay");
        assert_eq!(p.scroll_offset, 40.0);
        assert!(!p.scroll_to_cursor, "the view does not jump");
        assert!(p.loaded_at.is_some());

        // A changed size is a change too.
        let mut l = list(&[("folder", true), ("a.txt", false), ("c.txt", false), ("d.txt", false)]);
        l[2].size = 99;
        assert!(p.update_listing(l));
        assert_eq!(p.current_name().as_deref(), Some("c.txt"));

        // The cursor's item was removed: the cursor stays on its row (the next item).
        assert!(p.update_listing(list(&[("folder", true), ("a.txt", false), ("d.txt", false)])));
        assert_eq!(p.current_name().as_deref(), Some("d.txt"));
        // Removed at the end: the last row.
        assert!(p.update_listing(list(&[("folder", true), ("a.txt", false)])));
        assert_eq!(p.current_name().as_deref(), Some("a.txt"));

        // The filter and the sort stay.
        p.sort = SortKey::Size;
        p.sort_asc = false;
        let mut l = list(&[("folder", true), ("a.txt", false), ("z.txt", false)]);
        l[2].size = 1000;
        assert!(p.update_listing(l));
        assert_eq!(p.sort, SortKey::Size);
        let order: Vec<_> = p.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, ["folder", "z.txt", "a.txt"]);
    }

    #[test]
    fn cursor_after_a_drop() {
        assert_eq!(first_in_view(&["", "a", "b"], &["b".into(), "a".into()]), Some(1), "display order wins");
        assert_eq!(first_in_view(&["", "a"], &["x".into()]), None);
        // Dropped into the shown folder: after the refresh the first new item (in the
        // panel's order) gets the cursor, not the first dropped one.
        let mut p = pane_with(&[("folder", true), ("b.txt", false)]);
        p.select_any = Some(vec!["z.jpg".into(), "c.png".into()]);
        p.set_listing(
            "/a".into(),
            [("folder", true), ("b.txt", false), ("c.png", false), ("z.jpg", false)].iter().map(|(n, d)| ent(n, *d)).collect(),
            None,
        );
        assert_eq!(p.current_name().as_deref(), Some("c.png"));
        assert!(p.select_any.is_none(), "used once");
        // Onto a folder row: the cursor is put on the folder.
        p.select_name("folder");
        assert_eq!(p.current_name().as_deref(), Some("folder"));
    }

    #[test]
    fn rename_in_the_list_belongs_to_the_item_not_the_row() {
        let names = [("folder", true), ("a.txt", false), ("b.txt", false), ("c.txt", false)];
        let started = |p: &mut Pane, name: &str| {
            p.select_name(name);
            p.start_inline_rename().unwrap();
            assert_eq!(p.rename.as_ref().map(|r| r.name.as_str()), Some(name));
        };
        let row_of = |p: &Pane, name: &str| p.entries.iter().position(|e| e.name == name).unwrap();

        // Sorting moves the rows: the edit ends (WinSCP renamed the item now on that row).
        let mut p = pane_with(&names);
        started(&mut p, "b.txt");
        p.set_sort(SortKey::Size);
        assert!(p.rename.is_none());
        assert_eq!(p.current_name().as_deref(), Some("b.txt"));

        // A refresh of the same folder (Ctrl+R, a finished transfer) ends it as well.
        started(&mut p, "b.txt");
        p.set_listing("/a".into(), names.iter().map(|(n, d)| ent(n, *d)).collect(), None);
        assert!(p.rename.is_none());

        // The quick filter and leaving the folder too.
        started(&mut p, "c.txt");
        p.filter = "a".into();
        p.rebuild_view();
        assert!(p.rename.is_none());
        p.filter.clear();
        p.rebuild_view();
        started(&mut p, "c.txt");
        p.set_listing("/elsewhere".into(), vec![ent("c.txt", false)], None);
        assert!(p.rename.is_none());

        // The field is drawn only on that item's row, wherever it is.
        let mut p = pane_with(&names);
        started(&mut p, "a.txt");
        let a = row_of(&p, "a.txt");
        assert!(p.renaming_entry(a));
        assert!((0..p.entries.len()).filter(|i| *i != a).all(|i| !p.renaming_entry(i)));
        // An item of the same name in another folder is not it.
        p.path = "/elsewhere".into();
        assert!(!p.renaming_entry(a));

        // Not on "..", not in an archive or in search results.
        let mut p = pane_with(&names);
        p.cursor_home();
        assert!(p.start_inline_rename().is_err());
        p.offline = true;
        p.select_name("a.txt");
        assert!(p.start_inline_rename().unwrap_err().contains("not connected"));

        // "Cancel" in a question: back in the field with the typed name.
        let mut p = pane_with(&names);
        p.resume_inline_rename("/a".into(), "a.txt".into(), "b.txt".into());
        let r = p.rename.as_ref().unwrap();
        assert_eq!((r.name.as_str(), r.buf.as_str()), ("a.txt", "b.txt"));
        assert_eq!(p.current_name().as_deref(), Some("a.txt"));
    }

    /// A listing sorted in the background is taken as it is (sorting a million
    /// names on the UI thread froze the window for seconds); it is the same
    /// order the panel makes itself, for every column and direction.
    #[test]
    fn a_listing_sorted_in_the_background_is_used_as_it_is() {
        let mut list: Vec<Entry> = ["b10.txt", "B2.txt", "a.zip", "dir", "z", "c.TXT"]
            .iter()
            .enumerate()
            .map(|(i, n)| Entry { size: (i as u64 * 7) % 5, modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(i as u64 * 3 % 4)), ..ent(n, *n == "dir") })
            .collect();
        list.reverse();
        for key in [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Ext] {
            for asc in [true, false] {
                let mut own = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
                own.sort = key;
                own.sort_asc = asc;
                let mut bg = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
                bg.sort = key;
                bg.sort_asc = asc;
                own.set_listing("/a".into(), list.clone(), None);
                bg.set_prepared("/a".into(), Prepared::new(list.clone(), Some((key, asc))), None);
                let names = |p: &Pane| p.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
                assert_eq!(names(&own), names(&bg), "{key:?} {asc}");
            }
        }
        // Claimed sorted: not sorted again on the UI thread.
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        let claimed = Prepared { entries: vec![ent("z", false), ent("a", false)], sorted: Some((SortKey::Name, true)), print: 1 };
        p.set_prepared("/a".into(), claimed, None);
        assert_eq!(p.entries[0].name, "z");
        // The same fingerprint again: nothing changes; another one: the new listing.
        assert!(!p.update_prepared(Prepared { entries: vec![ent("z", false), ent("a", false)], sorted: Some((SortKey::Name, true)), print: 1 }));
        assert!(p.update_prepared(Prepared::new(vec![ent("q", false)], Some((SortKey::Name, true)))));
        assert_eq!(p.entries[0].name, "q");
    }

    /// Names in a mixed order (a simple deterministic generator).
    fn shuffled(n: usize) -> Vec<Entry> {
        let mut x: u64 = 12345;
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let mut e = ent(&format!("F{}x{}.{}", x >> 44, i, ["txt", "JPG", "rs"][(x >> 20) as usize % 3]), x.is_multiple_of(17));
                e.size = (x >> 30) % 1000;
                e.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs((x >> 33) % 5000));
                e
            })
            .collect()
    }

    /// A big listing is sorted and filtered on several threads; the result is
    /// the same as on one (roadmap 0.8.15: a million names took 4 s to sort).
    #[test]
    fn sorting_and_filtering_on_threads_give_the_same_order() {
        let mut list = shuffled(120_000);
        for n in ["a-1", "a01", "a1b", "a 1", "a10", "a9", "a0", "a", "a00012x3", "a12x03", "b2.10", "b2.9", "Ä1", "ä01"] {
            list.push(ent(n, false));
        }
        for key in [SortKey::Name, SortKey::Ext, SortKey::Size, SortKey::Modified] {
            for asc in [true, false] {
                let got = sorted(list.clone(), key, asc, &HashMap::new());
                // The same comparison on one thread.
                let mut want = list.clone();
                let text = |e: &Entry| match key {
                    SortKey::Name => natural_lower(&e.name),
                    SortKey::Ext => ext_of(&e.name).to_lowercase(),
                    _ => String::new(),
                };
                want.sort_by(|a, b| {
                    let ord = b.is_dir_like().cmp(&a.is_dir_like());
                    if ord.is_ne() {
                        return ord;
                    }
                    let o = match key {
                        SortKey::Name | SortKey::Ext => text(a).cmp(&text(b)),
                        // Folders without a computed size count 0.
                        SortKey::Size => (!a.is_dir_like() as u64 * a.size).cmp(&(!b.is_dir_like() as u64 * b.size)),
                        SortKey::Modified => a.modified.cmp(&b.modified),
                    }
                    .then_with(|| a.name.cmp(&b.name));
                    if asc { o } else { o.reverse() }
                });
                let names = |v: &[Entry]| v.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
                assert_eq!(names(&got), names(&want), "{key:?} {asc}");
            }
        }
        // Stable: equal items keep their order.
        let pairs: Vec<(u32, usize)> = (0..200_000).map(|i| ((i * 7919 % 1000) as u32, i)).collect();
        let got = par_sort_by(pairs.clone(), &|a: &(u32, usize), b: &(u32, usize)| a.0.cmp(&b.0));
        let mut want = pairs;
        want.sort_by_key(|p| p.0);
        assert_eq!(got, want);
        assert!(par_sort_by(Vec::<u8>::new(), &|a: &u8, b: &u8| a.cmp(b)).is_empty());
        // Filtering keeps the order of the candidates.
        let idx: Vec<usize> = (0..150_000).collect();
        let f = |i: usize| i.is_multiple_of(3).then_some(i * 2);
        assert_eq!(par_filter_map(&idx, &f), idx.iter().filter_map(|&i| f(i)).collect::<Vec<_>>());
    }

    /// The item under the cursor is found again after a refresh, wherever it
    /// moved, looking from where it was.
    #[test]
    fn the_cursor_item_is_found_near_where_it_was() {
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        let list: Vec<Entry> = (0..1000).map(|i| ent(&format!("f{i:04}"), false)).collect();
        p.set_listing("/a".into(), list, None);
        for (name, near) in [("f0000", 999), ("f0999", 0), ("f0500", 500), ("f0501", 499), ("f0499", 501)] {
            let row = p.find_row(name, near).unwrap();
            assert!(matches!(p.view[row], Row::Entry(i) if p.entries[i].name == name), "{name}");
        }
        assert_eq!(p.find_row("missing", 3), None);
        assert_eq!(p.find_row("f0001", 5000).map(|r| p.view[r]), p.view.iter().position(|r| matches!(r, Row::Entry(i) if p.entries[*i].name == "f0001")).map(|r| p.view[r]));
        p.cursor = 10;
        let mut fewer = p.entries.clone();
        fewer.remove(0);
        let name = p.current_name().unwrap();
        p.update_listing(fewer);
        assert_eq!(p.current_name().unwrap(), name);
    }

    /// Marking all items of a big folder, one more, inverting, by a mask:
    /// the footer counts only the change (looking up a million marked names
    /// took 0.8 s for every change) and always says the same as a full count.
    #[test]
    fn the_footer_counts_only_what_marking_changed() {
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        p.set_listing("/a".into(), shuffled(30_000), None);
        let fresh = |p: &Pane| selection_summary(&p.entries, &p.marked, &p.dir_sizes);
        assert_eq!(p.footer_text(), fresh(&p));
        type Step = Box<dyn Fn(&mut Pane)>;
        let steps: Vec<(&str, Step)> = vec![
            ("all", Box::new(|p: &mut Pane| p.mark_all())),
            ("one less", Box::new(|p: &mut Pane| p.toggle_mark_current())),
            ("one more", Box::new(|p: &mut Pane| p.toggle_mark_current())),
            ("invert", Box::new(|p: &mut Pane| p.invert_selection(false))),
            ("invert with folders", Box::new(|p: &mut Pane| p.invert_selection(true))),
            ("mask", Box::new(|p: &mut Pane| {
                p.mark_group("*.rs", true, false);
            })),
            ("unmask", Box::new(|p: &mut Pane| {
                p.mark_group("F1*", false, true);
            })),
            ("sort", Box::new(|p: &mut Pane| p.set_sort(SortKey::Size))),
            ("click", Box::new(|p: &mut Pane| p.toggle_mark(7))),
        ];
        for (what, step) in steps {
            step(&mut p);
            assert!(p.mark_sums().is_some(), "{what}: counted again from all names");
            assert_eq!(p.footer_text(), fresh(&p), "{what}");
        }
        // Everything marked: known without looking up a name.
        p.marked.clear();
        p.mark_all();
        assert!(p.all_marked_known());
        let mut names: Vec<String> = p.entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(p.targets(), names);
        p.mark_all();
        assert!(p.marked.is_empty(), "the second Ctrl+A unmarks");
        // A change of the marks from elsewhere is counted in full.
        p.marked.insert(p.entries[3].name.clone());
        assert!(p.mark_sums().is_none());
        assert_eq!(p.footer_text(), fresh(&p));
    }

    /// The footer of a big folder is kept between frames, but follows every
    /// change of the marks, the listing and the folder sizes.
    #[test]
    fn the_footer_of_a_big_folder_follows_every_change() {
        let mut list: Vec<Entry> = (0..25_000).map(|i| ent(&format!("f{i:05}"), false)).collect();
        list[0].size = 1_000;
        list.push(ent("dir", true));
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        p.set_listing("/a".into(), list, None);
        let fresh = |p: &Pane| selection_summary(&p.entries, &p.marked, &p.dir_sizes);
        let first = p.footer_text();
        assert_eq!(first, fresh(&p));
        p.marked.insert("f00001".into());
        let one = p.footer_text();
        assert_ne!(one, first);
        assert_eq!(one, fresh(&p));
        // The same number of marks, another (bigger) file.
        p.marked.remove("f00001");
        p.marked.insert("f00000".into());
        assert_eq!(p.footer_text(), fresh(&p));
        assert_ne!(p.footer_text(), one);
        p.set_dir_size("dir", Some(crate::dirsize::DirSize { bytes: 5_000_000, unreadable: 0 }));
        assert_eq!(p.footer_text(), fresh(&p));
        let mut more = p.entries.clone();
        more.push(ent("new", false));
        p.update_listing(more);
        assert_eq!(p.footer_text(), fresh(&p));
    }

    #[test]
    fn the_fingerprint_sees_every_change_but_not_the_order() {
        let a = vec![ent("x", false), ent("y", true)];
        let b = vec![ent("y", true), ent("x", false)];
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = a.clone();
        c[0].size += 1;
        assert_ne!(fingerprint(&a), fingerprint(&c));
        let mut d = a.clone();
        d[1].mode = Some(0o700);
        assert_ne!(fingerprint(&a), fingerprint(&d));
        assert_ne!(fingerprint(&a), fingerprint(&a[..1]));
    }

    /// Typing more letters narrows the rows already found; the result is the
    /// same as filtering from scratch, also after the folder changed.
    #[test]
    fn a_longer_filter_narrows_the_same_way() {
        let names: Vec<String> = (0..500).map(|i| format!("f{i:04}")).chain(["F12.txt".into(), "x1y2".into()]).collect();
        let mut p = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
        p.set_listing("/a".into(), names.iter().map(|n| ent(n, false)).collect(), None);
        let shown = |p: &Pane| -> Vec<String> {
            p.view.iter().filter_map(|r| if let Row::Entry(i) = r { Some(p.entries[*i].name.clone()) } else { None }).collect()
        };
        let fresh = |p: &Pane, q: &str| {
            let mut f = Pane::new(Arc::new(rolid_local::LocalFs), "/a".into());
            f.set_listing("/a".into(), p.entries.clone(), None);
            f.filter = q.into();
            f.rebuild_view();
            shown(&f)
        };
        for q in ["f", "f1", "f12", "f12.", "f1", "f"] {
            p.filter = q.into();
            p.rebuild_view();
            assert_eq!(shown(&p), fresh(&p, q), "{q}");
        }
        p.filter = "f12".into();
        p.rebuild_view();
        // A new matching file arrives (background refresh): it must show.
        let mut more = p.entries.clone();
        more.push(ent("f1200-new", false));
        p.update_listing(more);
        p.filter = "f120".into();
        p.rebuild_view();
        assert!(shown(&p).contains(&"f1200-new".to_string()));
        assert_eq!(shown(&p), fresh(&p, "f120"));
    }

    #[test]
    fn group_marks_and_invert() {
        let mut p = pane_with(&[("photos", true), ("A.JPG", false), ("b.png", false), ("c.txt", false)]);
        assert_eq!(p.mark_group("*.jpg;*.png", true, false), 2);
        assert!(p.marked.contains("A.JPG") && p.marked.contains("b.png"));
        assert_eq!(p.mark_group("*", true, true), 2, "folder and c.txt added");
        assert_eq!(p.mark_group("*.png", false, false), 1);
        p.marked.clear();
        p.marked.insert("c.txt".into());
        p.invert_selection(false);
        let mut m: Vec<_> = p.marked.iter().cloned().collect();
        m.sort();
        assert_eq!(m, ["A.JPG", "b.png"], "folders are not touched");
        // With folders (the setting): the folder flips too.
        p.invert_selection(true);
        let mut m: Vec<_> = p.marked.iter().cloned().collect();
        m.sort();
        assert_eq!(m, ["c.txt", "photos"]);
        // Mask without / with folders: a folder matching the mask.
        let mut p = pane_with(&[("photo.jpg", true), ("x.jpg", false)]);
        assert_eq!(p.mark_group("*.jpg", true, false), 1);
        assert!(!p.marked.contains("photo.jpg"));
        assert_eq!(p.mark_group("*.jpg", true, true), 1);
        assert!(p.marked.contains("photo.jpg"));
    }

    /// Odd names are painted on one readable line (roadmap 0.8.15), and the
    /// pane keeps the real name for everything else.
    #[test]
    fn odd_names_are_painted_readably() {
        use super::views::shown_name;
        let odd = rolid_core::names::from_bytes(b"bad\xff\nname.txt").into_owned();
        let f = ent(&odd, false);
        assert_eq!(shown_name(&f, false, false), "bad\\xFF\\nname.txt");
        assert_eq!(shown_name(&f, true, false), "bad\\xFF\\nname");
        assert_eq!(shown_name(&ent("tab\there", true), false, true), "[tab\\there]");
        let p = pane_with(&[(odd.as_str(), false)]);
        assert_eq!(p.entries[0].name, odd);
    }

    #[test]
    fn brackets_only_when_painting() {
        use super::views::shown_name;
        let d = ent("Photos", true);
        let f = ent("a.txt", false);
        assert_eq!(shown_name(&d, false, true), "[Photos]");
        assert_eq!(shown_name(&d, true, true), "[Photos]");
        assert_eq!(shown_name(&d, false, false), "Photos");
        assert_eq!(shown_name(&f, false, true), "a.txt");
        assert_eq!(shown_name(&f, true, true), "a");
        // The pane itself keeps the real names: sort, filter, current name.
        let mut p = pane_with(&[("b", false), ("Photos", true), ("[zz]", true), ("a", true)]);
        let names = |p: &Pane| p.view.iter().filter_map(|r| match r { Row::Entry(i) => Some(p.entries[*i].name.clone()), _ => None }).collect::<Vec<_>>();
        // By the real names: "[zz]" (a real bracket) first, "Photos" among the
        // letters, not next to it as the shown "[Photos]" would be.
        assert_eq!(names(&p), ["[zz]", "a", "Photos", "b"]);
        p.filter = "pho".into();
        p.rebuild_view();
        assert_eq!(names(&p), ["Photos"]);
        assert_eq!(p.current_name().as_deref(), Some("Photos"));
        p.filter = "[".into();
        p.rebuild_view();
        assert_eq!(names(&p), ["[zz]"], "a bracket in the filter matches only real brackets");
    }

    #[test]
    fn history_follows_folder_changes() {
        let mut p = pane_with(&[]);
        p.set_listing("/a/b".into(), vec![], None);
        p.set_listing("/a/b".into(), vec![], None); // refresh: not a visit
        p.set_listing("/c".into(), vec![], None);
        assert_eq!(p.history.go_back("/c").as_deref(), Some("/a/b"));
        p.set_listing("/a/b".into(), vec![], None);
        assert!(p.history.can_forward());
        assert_eq!(p.history.go_back("/a/b").as_deref(), Some("/a"));
    }

    #[test]
    fn leaving_a_folder_cancels_its_thumbnails() {
        let mut p = pane_with(&[]);
        let old = p.thumb_cancel.clone();
        p.set_listing("/a".into(), vec![], None);
        assert!(!old.load(Ordering::Relaxed), "refresh keeps them");
        p.set_listing("/b".into(), vec![], None);
        assert!(old.load(Ordering::Relaxed));
        assert!(!p.thumb_cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn arrows_per_style() {
        let names: Vec<(String, bool)> = (0..10).map(|i| (format!("f{i}"), false)).collect();
        let refs: Vec<(&str, bool)> = names.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let mut p = pane_with(&refs);
        // view = ".." + 10 files
        assert!(!p.move_horiz(1), "detailed listing ignores ← →");
        p.set_style(ViewStyle::Brief);
        p.brief_rows = 4;
        p.cursor = 1;
        assert!(p.move_horiz(1));
        assert_eq!(p.cursor, 5);
        p.move_horiz(1);
        p.move_horiz(1);
        assert_eq!(p.cursor, 10, "stops on the last item");
        p.set_style(ViewStyle::Thumbs);
        p.grid_cols = 3;
        p.cursor = 4;
        p.move_vert(1);
        assert_eq!(p.cursor, 7);
        p.move_horiz(-1);
        assert_eq!(p.cursor, 6);
    }

    #[test]
    fn selection_summary_texts() {
        let mut big = ent("big.iso", false);
        big.size = 850 * 1024 * 1024;
        let mut small = ent("a.jpg", false);
        small.size = 12 * 1024 * 1024 + 400 * 1024;
        let entries = vec![ent("photos", true), big, small, ent("b.txt", false)];
        let mut marked = HashSet::new();
        assert_eq!(selection_summary(&entries, &marked, &HashMap::new()), "3 files, 862.4 MB, 1 folder");
        marked.insert("a.jpg".to_string());
        assert_eq!(selection_summary(&entries, &marked, &HashMap::new()), "Selected: 1 of 3 files, 12.4 MB of 862.4 MB");
        marked.insert("photos".to_string());
        assert_eq!(
            selection_summary(&entries, &marked, &HashMap::new()),
            "Selected: 1 of 3 files, 12.4 MB of 862.4 MB and 1 of 1 folder"
        );
        let one = vec![ent("x", false)];
        assert_eq!(selection_summary(&one, &HashSet::new(), &HashMap::new()), "1 file, 10 B");
        let many: Vec<Entry> = (0..95).map(|i| ent(&format!("f{i}"), false)).collect();
        assert!(selection_summary(&many, &HashSet::new(), &HashMap::new()).starts_with("95 files"));
        marked.clear();
        marked.insert("photos".to_string());
        assert_eq!(selection_summary(&entries, &marked, &HashMap::new()), "Selected: 1 of 1 folder");
        // Computed folder sizes count in (with ≥ when incomplete).
        let mut sizes = HashMap::new();
        sizes.insert("photos".to_string(), crate::dirsize::DirSize { bytes: 1024 * 1024, unreadable: 0 });
        assert_eq!(selection_summary(&entries, &marked, &sizes), "Selected: 1 of 1 folder, 1.0 MB");
        marked.insert("b.txt".to_string());
        assert_eq!(selection_summary(&entries, &marked, &sizes), "Selected: 1 of 3 files, 1.0 MB of 863.4 MB and 1 of 1 folder");
        sizes.insert("photos".to_string(), crate::dirsize::DirSize { bytes: 1024 * 1024, unreadable: 2 });
        assert_eq!(selection_summary(&entries, &HashSet::new(), &sizes), "3 files, ≥ 863.4 MB, 1 folder (with folder sizes)");
        assert_eq!(ext_of("archive.tar.gz"), "gz");
        assert_eq!(ext_of(".bashrc"), "");
    }

    #[test]
    fn active_header_is_strong_and_readable() {
        fn lum(c: Color32) -> f32 {
            let ch = |v: u8| {
                let s = v as f32 / 255.0;
                if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        let contrast = |a: Color32, b: Color32| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for (_, c) in rolid_core::color::PASTEL_PRESETS {
            let accent = Color32::from_rgb(c.0, c.1, c.2);
            for dark in [false, true] {
                let (bg, fg) = header_colors(accent, true, dark);
                assert!(contrast(bg, fg) >= 4.5, "active {accent:?}: {}", contrast(bg, fg));
                let (ibg, ifg) = header_colors(accent, false, dark);
                assert!(contrast(ibg, ifg) >= 4.5, "inactive {accent:?}");
                // Much more saturated than the pastel tab color.
                let s_tab = egui::ecolor::HsvaGamma::from(accent).s;
                let s_hdr = egui::ecolor::HsvaGamma::from(bg).s;
                assert!(s_tab < 0.05 || (s_hdr >= 0.55 && s_hdr > s_tab + 0.2), "{accent:?}: {s_tab} -> {s_hdr}");
                assert!(lum(bg) < lum(ibg) || dark, "active is darker than inactive");
            }
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 kB");
    }
}

