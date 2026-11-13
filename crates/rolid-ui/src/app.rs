//! Application state: tabs, keyboard handling, dialogs and background tasks.

mod archdir_ui;
mod checksum_ui;
mod disk_ui;
mod terminal_ui;
pub use disk_ui::{badge_width as disk_badge_width, paint_badge as disk_paint_badge, space_color as disk_space_color, Badge as DiskBadge};
pub(crate) mod archive_ui;
mod ctx_ui;
mod drag_out;
mod drag_out_ui;
mod drag_ui;
mod folder_pick;
mod viewer;
mod drop_ui;
mod edit_ui;
mod fbar;
mod link_ui;
mod menu;
mod newfile_ui;
mod places;
mod refresh;
mod rename_ui;
mod search;
mod session;
mod size_ui;
mod unpack_ui;
mod vault_ui;
mod view_ui;
mod x11_drag;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, Stroke, StrokeKind, Vec2};
use rolid_config::Profiles;
use rolid_core::color::{Rgb, PASTEL_PRESETS};
use rolid_core::copy::{Conflict, CopyJob, CopyState, Failure, Progress, Totals, MAX_AUTO_RETRIES};
use rolid_core::rate::RateLimit;
use rolid_core::vfs::validate_name;
use rolid_core::{Entry, Vfs, VfsResult};
use rolid_local::LocalFs;

use crate::dialog;
use crate::look::Role;
use crate::pane::{human_size, text_on, Pane, PaneEvent, Row, Target};
use session::{ConnectForm, ConnectMeta, ConnectResult};
use view_ui::columns_key;

pub type TabId = u64;
type Action = Box<dyn FnOnce(&mut App)>;

#[derive(Clone)]
enum SwitchTo {
    Tab(usize, usize),
    Profile(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaneRef {
    pub tab: TabId,
    pub side: usize,
}

/// One tab of a panel: a location (local or remote) with its own color and name.
pub struct Tab {
    pub id: TabId,
    pub name: Option<String>,
    pub color: Rgb,
    pub pane: Pane,
}

impl Tab {
    pub fn title(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        let p = &self.pane;
        let dir = p
            .path
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or("/");
        if p.remote.is_some() {
            format!("{}:{}", p.display_label(), dir)
        } else {
            dir.to_string()
        }
    }
}

/// The tabs of the left or the right panel.
#[derive(Default)]
pub struct Side {
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Most recently used tabs first (for Ctrl+Tab).
    pub mru: Vec<TabId>,
}

impl Side {
    fn index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn current(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }
}

/// Pane lookup on the sides only (usable while `self.dialog` is borrowed).
fn find_pane(sides: &[Side; 2], at: PaneRef) -> Option<&Pane> {
    sides
        .get(at.side)?
        .tabs
        .iter()
        .find(|t| t.id == at.tab)
        .map(|t| &t.pane)
}

enum Msg {
    /// A watched local folder changed.
    DirChanged(String),
    /// A listing made in the background (see `refresh`).
    Refreshed {
        key: refresh::LocKey,
        result: VfsResult<Vec<Entry>>,
    },
    /// Subfolders for "Browse…".
    PickListed {
        req: u64,
        path: String,
        result: Result<Vec<String>, String>,
    },
    /// How many of the items being copied already exist in `dest`.
    CopyOverwrite {
        dest: String,
        n: usize,
    },
    Listed {
        at: PaneRef,
        req: u64,
        path: String,
        select: Option<String>,
        result: VfsResult<Vec<Entry>>,
    },
    Resolved {
        at: PaneRef,
        path: String,
        select: Option<String>,
    },
    Connected {
        at: PaneRef,
        meta: ConnectMeta,
        result: Box<ConnectResult>,
    },
    /// Background note (e.g. keyring result); `Some` is shown as an error.
    Note(Option<String>),
    /// A server asks a keyboard-interactive question during login.
    AuthPrompt(Box<session::AuthPromptForm>),
    /// Vault (master password) work finished in the background.
    Vault(Box<vault_ui::VaultMsg>),
    /// F4 editing: saves detected, downloads / uploads finished.
    Edit(Box<edit_ui::EditMsg>),
    /// Files dragged in from the system are counted and checked.
    DropReady(Box<drop_ui::DropReady>),
    /// Free space of a panel's disk (`None` = unknown / not offered by the server).
    Space {
        at: PaneRef,
        path: String,
        space: Option<rolid_core::vfs::Space>,
        /// File system type (`ext4`, `nfs4`…), when it could be found out.
        fstype: Option<String>,
        /// The physical disks behind it, or why their health is not read.
        disks: Result<Vec<String>, rolid_disk::physical::NoSmart>,
    },
    /// The health of a disk was read.
    DiskHealth(Box<rolid_disk::health::DiskHealth>),
    /// Temperatures read again (disk name, °C), at a time (seconds since 1970).
    DiskTemps(Vec<(String, Option<f64>)>, u64),
    /// The permissions change was planned (folders' contents walked).
    PermsPlanned(Box<ctx_ui::PermsPlanned>),
    JobDone {
        id: u64,
        refresh: Vec<PaneRef>,
        result: VfsResult<()>,
        /// Files skipped because they could not be copied (keep-going jobs).
        failures: Vec<Failure>,
        /// For a failed copy: everything needed to try again by hand (continuing
        /// where it stopped).
        retry: Option<Box<CopySpec>>,
        /// Next step after a successful copy.
        then: Option<Then>,
    },
    /// Background work finished: remove its job row and run the continuation.
    Then { job: Option<u64>, f: Then },
    OpDone {
        refresh: Vec<PaneRef>,
        select: Option<(PaneRef, String)>,
        result: VfsResult<()>,
    },
    /// Delete finished in a pane that shows search results: drop these rows.
    Removed {
        at: PaneRef,
        labels: Vec<String>,
        refresh: Vec<PaneRef>,
        result: VfsResult<()>,
    },
    SearchDone {
        at: PaneRef,
        id: u64,
        result: Result<rolid_search::Outcome, String>,
    },
}

struct Job {
    id: u64,
    title: String,
    progress: Arc<Mutex<Progress>>,
    total_bytes: u64,
    cancel: Arc<AtomicBool>,
    /// Shown instead of the progress (e.g. waiting for the connection).
    note: Arc<Mutex<Option<String>>>,
    /// Item-counted job (permissions): progress = `files_done` of this.
    count: Option<u64>,
    /// Files and bytes in all (copy jobs count them in the background).
    totals: Arc<Mutex<Totals>>,
}

/// A copy job: what, from where, to where (kept for a manual retry).
pub struct CopySpec {
    sfs: Arc<dyn Vfs>,
    dfs: Arc<dyn Vfs>,
    dpath: String,
    items: Vec<Target>,
    conflict: Conflict,
    title: String,
    refresh: Vec<PaneRef>,
    /// Connections of both sides (to wait for after a loss).
    links: Vec<Arc<rolid_ssh::Link>>,
    /// Continue automatically after a lost connection (not when it could
    /// overwrite on a production server – that needs a new confirmation).
    auto_retry: bool,
    state: Option<CopyState>,
    /// Do not stop at unreadable files: list them in a summary at the end.
    keep_going: bool,
    /// Items refused before the start (shown in the same summary).
    pre_failed: Vec<Failure>,
    /// Run after the copy finished successfully (next step of a longer task).
    then: Option<Then>,
    /// Temporary folder the copy reads from or writes to; deleted when the
    /// last task using it is gone (also after a failed job is dismissed).
    /// Only held for its `Drop`.
    #[allow(dead_code)]
    temp: Option<Arc<archive_ui::TempDir>>,
    /// A move (F6 between disks): the sources are deleted after the copy.
    moving: bool,
    /// The single item's name in the target, when it changes.
    target_name: Option<String>,
}

/// Continuation run on the UI thread when background work is done.
pub(crate) type Then = Box<dyn FnOnce(&mut App) + Send>;

/// What a job does, where its failure is reported differently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum JobKind {
    Move,
    Perms,
}

/// A transfer that failed; can be tried again.
struct FailedJob {
    id: u64,
    title: String,
    error: String,
    spec: Box<CopySpec>,
}

enum Dialog {
    None,
    Checksum(Box<checksum_ui::SumDialog>),
    /// Disk health (SMART) detail.
    Disk(Box<disk_ui::DiskDialog>),
    NewFile(Box<newfile_ui::NewFile>),
    /// Dragging out: a large download into a temporary folder first?
    DragOutLarge(Box<drag_out_ui::LargeAsk>),
    /// Ctrl+P on a production server: open the terminal?
    TermConfirm(Box<terminal_ui::TermConfirm>),
    Connect(Box<ConnectForm>),
    Search(Box<search::SearchForm>),
    Copy {
        from: PaneRef,
        to: PaneRef,
        /// The target folder (in the `to` panel's file system).
        dest: String,
        items: Vec<Target>,
        overwrite: usize,
        /// `None` = nothing preselected yet (Enter does nothing).
        conflict: Option<Conflict>,
    },
    Mkdir {
        at: PaneRef,
        name: String,
        focus: bool,
    },
    Delete {
        at: PaneRef,
        items: Vec<Target>,
        production: bool,
    },
    Switcher {
        query: String,
        cursor: usize,
        focus: bool,
    },
    /// "+" / Ctrl+T: what the new tab of panel `side` shows.
    NewTab {
        side: usize,
        query: String,
        cursor: usize,
        focus: bool,
        /// Renaming or removing a favorite in the list.
        fav: places::FavEdit,
    },
    /// The favorite folders (rename, order, remove).
    Favorites(places::FavoritesForm),
    /// F1: the guide (`false`) or the keyboard shortcuts (`true`).
    Help(bool),
    About,
    /// Confirm files dragged in from the system (production / large / existing).
    Drop(Box<drop_ui::DropAsk>),
    /// Files were dropped but where is unknown: let the user pick the panel.
    DropWhere {
        paths: Vec<std::path::PathBuf>,
    },
    /// Permissions of the selected items.
    Perms(Box<ctx_ui::PermsForm>),
    /// "Change permissions of N files and M folders?"
    PermsConfirm(Box<ctx_ui::PermsConfirm>),
    /// "Open with → Other application…"
    OpenWith(Box<ctx_ui::OpenWithForm>),
    /// "Select / unselect group" by mask.
    MarkGroup(Box<view_ui::MarkForm>),
    /// Settings (view, thumbnails).
    Settings(Box<view_ui::SettingsForm>),
    /// Pack / extract / test archives.
    Archive(Box<archive_ui::ArchiveDialog>),
    /// A typed rename needs an answer (production, existing target).
    Rename(Box<rename_ui::RenameAsk>),
    /// F6.
    Move(Box<rename_ui::MoveForm>),
}

pub struct App {
    rt: tokio::runtime::Handle,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    /// Left and right panel, each with its own tabs.
    sides: [Side; 2],
    /// 0 = left, 1 = right.
    active_side: usize,
    next_id: u64,
    /// Ctrl+Tab cycling in the active panel: (MRU snapshot, index).
    switching: Option<(Vec<TabId>, usize)>,
    renaming: Option<(TabId, String, bool)>,
    dialog: Dialog,
    /// Disk health (SMART) of the disks shown in the panels.
    disks: disk_ui::Disks,
    jobs: Vec<Job>,
    failed_jobs: Vec<FailedJob>,
    /// Jobs whose failure is reported differently (a move, permissions).
    job_kinds: HashMap<u64, JobKind>,
    /// Shared connections per server (with automatic reconnection).
    links: link_ui::LinksState,
    last_error: Option<String>,
    /// A short confirmation in the status line (not an error); hides itself.
    notice: Option<(String, Instant)>,
    last_title: String,
    profiles: Profiles,
    /// Limits concurrent transfers; further jobs wait in the queue.
    transfer_slots: Arc<tokio::sync::Semaphore>,
    saved_workspace: String,
    /// Favorite folders and recent places (saved with the workspace).
    places: rolid_config::Places,
    last_save_check: Instant,
    search_history: Vec<rolid_search::Query>,
    settings: rolid_config::Settings,
    /// Donation address shown in the About dialog.
    donate_btc: &'static str,
    /// Bandwidth limit shared by all transfers (from `settings.speed_limit_kib`).
    rate_limit: Arc<RateLimit>,
    /// Pending keyboard-interactive questions (oldest first).
    auth_prompts: Vec<session::AuthPromptForm>,
    /// Saved server passwords protected by the master password.
    vault: vault_ui::VaultState,
    /// Files open in an external editor (F4).
    edits: edit_ui::EditsState,
    /// Listings that keep up by themselves.
    auto: refresh::AutoRefresh,
    /// Drag and drop from the system file manager.
    drop: drop_ui::DropState,
    /// Image previews of the thumbnail listing (shared by all panels).
    thumbs: crate::pane::thumbs::Thumbs,
    /// Files dragged with the mouse inside the app (not from the system).
    inner_drag: Option<drag_ui::InnerDrag>,
    /// X11: the dragged rows went out of the window to another application.
    drag_out: Option<drag_out_ui::DragOut>,
    /// Temporary folders of finished drags out, deleted when unused.
    drag_temps: Vec<drag_out_ui::DragTemp>,
    /// X11 id of our window (for dragging out).
    x11_win: Option<u32>,
    /// "Browse…" of the copy and move dialogs.
    folder_pick: Option<folder_pick::FolderPick>,
    /// Open F3 viewers.
    viewers: viewer::Viewers,
    /// X11: asks the X server where the cursor is while files are dragged in.
    x11: Option<x11_drag::Tracker>,
    x11_tried: bool,
    /// A drag from the system is in progress: successful pointer queries when it began
    /// (`None` = no drag). Logged once when it ends.
    drag_log: Option<u64>,
    /// Remaining-time windows of the running jobs (by job id).
    eta: HashMap<u64, rolid_core::eta::Window>,
    /// Why no pointer query could be made (not X11, window id unknown…).
    x11_why: Option<&'static str>,
    /// ROLIDSCP_FRAME_STATS=1: frame count, total and longest frame since `since`.
    frame_stats: Option<FrameStats>,
    /// Text font in use (for Settings and About).
    pub(crate) font_desc: String,
    /// The display scale was written to the log (first frame).
    scale_logged: bool,
}

/// Frame time diagnostics (ROLIDSCP_FRAME_STATS=1), logged every 2 s.
struct FrameStats {
    since: Instant,
    frames: u32,
    total: Duration,
    max: Duration,
}

impl FrameStats {
    fn add(&mut self, took: Duration) {
        self.frames += 1;
        self.total += took;
        self.max = self.max.max(took);
        if self.since.elapsed() >= Duration::from_secs(2) {
            let avg = self.total.as_secs_f64() * 1000.0 / self.frames.max(1) as f64;
            tracing::info!(
                "frames: {} in 2 s, average {:.2} ms, longest {:.2} ms",
                self.frames,
                avg,
                self.max.as_secs_f64() * 1000.0
            );
            *self = FrameStats { since: Instant::now(), frames: 0, total: Duration::ZERO, max: Duration::ZERO };
        }
    }
}

const SETTINGS_FILE: &str = "settings.toml";

/// A frame taking longer than this is logged (the UI must not wait for the network).
const SLOW_FRAME: Duration = Duration::from_millis(150);

/// Maximum number of transfers running at the same time.
const MAX_PARALLEL_TRANSFERS: usize = 3;

fn rgb32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// How much of the tab color tints the file list (the rest is the text background).
pub(crate) const LIST_TINT: f32 = 0.22;

pub(crate) fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Light);
        crate::look::apply(&cc.egui_ctx);
        // The grammars of the viewer take a moment to load: not on the UI thread.
        std::thread::spawn(rolid_view::syntax::preload);
        // Applications for "Open with" are read in the background.
        rolid_edit::apps::AppDb::preload();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = Self {
            rt: rt.clone(),
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            sides: [Side::default(), Side::default()],
            active_side: 0,
            next_id: 1,
            switching: None,
            renaming: None,
            dialog: Dialog::None,
            disks: Default::default(),
            jobs: Vec::new(),
            failed_jobs: Vec::new(),
            job_kinds: HashMap::new(),
            links: link_ui::LinksState::default(),
            last_error: None,
            notice: None,
            last_title: String::new(),
            profiles: Profiles::default(),
            transfer_slots: Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL_TRANSFERS)),
            saved_workspace: String::new(),
            places: rolid_config::Places::default(),
            last_save_check: Instant::now(),
            search_history: Vec::new(),
            settings: rolid_config::Settings::default(),
            donate_btc: menu::DONATE_BTC[0],
            rate_limit: Arc::new(RateLimit::default()),
            auth_prompts: Vec::new(),
            vault: vault_ui::VaultState::new(),
            edits: edit_ui::EditsState::default(),
            auto: refresh::AutoRefresh::default(),
            drop: drop_ui::DropState::default(),
            thumbs: crate::pane::thumbs::Thumbs::new(rt.clone(), cc.egui_ctx.clone()),
            inner_drag: None,
            drag_out: None,
            drag_temps: Vec::new(),
            x11_win: None,
            folder_pick: None,
            viewers: viewer::Viewers::default(),
            x11: None,
            x11_tried: false,
            drag_log: None,
            eta: HashMap::new(),
            x11_why: None,
            frame_stats: std::env::var_os("ROLIDSCP_FRAME_STATS").filter(|v| v == "1").map(|_| FrameStats {
                since: Instant::now(),
                frames: 0,
                total: Duration::ZERO,
                max: Duration::ZERO,
            }),
            font_desc: String::new(),
            scale_logged: false,
        };
        app.load_settings();
        app.load_search_history();
        app.edits_startup();
        std::thread::spawn(archive_ui::clean_old_temps);
        match rolid_config::load_profiles() {
            Ok(p) => app.profiles = p,
            Err(e) => app.last_error = Some(tr!("Cannot load the profiles: {error}", error = e)),
        }
        match rolid_config::load_workspace() {
            Ok(ws) => {
                app.saved_workspace = rolid_config::workspace_fingerprint(&ws);
                app.restore_workspace(ws);
            }
            Err(e) => app.last_error = Some(tr!("Cannot load the workspace: {error}", error = e)),
        }
        for side in 0..2 {
            if app.sides[side].tabs.is_empty() {
                let home = Self::home_dir();
                app.push_tab(side, Pane::new(Arc::new(LocalFs), home), None, None);
            }
        }
        // Both panels are visible: show (and connect) the other one, then the active one.
        let s = app.active_side;
        app.activate_tab(1 - s, app.sides[1 - s].active);
        app.activate_tab(s, app.sides[s].active);
        app
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn home_dir() -> String {
        std::env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into())
    }

    /// Folder with the user's translations (`<lang>.po`, over the built-in ones).
    pub(crate) fn lang_dir() -> std::path::PathBuf {
        rolid_config::config_dir().join("lang")
    }

    /// Switch to the language and date format of the settings (live: every
    /// text is translated when it is drawn).
    pub(crate) fn apply_language(&mut self) {
        let a = rolid_i18n::set_language(self.settings.language.as_deref(), self.settings.date_iso, Some(&Self::lang_dir()));
        for w in &a.warnings {
            tracing::warn!("translation: {w}");
        }
        tracing::info!(lang = %a.lang, date_iso = self.settings.date_iso, "language");
    }

    fn load_settings(&mut self) {
        match rolid_config::load_file::<rolid_config::Settings>(SETTINGS_FILE) {
            Ok(s) => self.settings = s,
            Err(e) => self.last_error = Some(tr!("Settings: {error}", error = e)),
        }
        self.rate_limit
            .set_rate(self.settings.speed_limit_kib.unwrap_or(0).saturating_mul(1024));
        self.apply_language();
        self.apply_font();
        let (addr, changed) = menu::pick_donate_address(self.settings.donate_address.as_deref());
        self.donate_btc = addr;
        if changed {
            self.settings.donate_address = Some(addr.to_string());
            self.save_settings();
        }
    }

    /// Apply a new transfer speed limit (KiB/s, `None` = unlimited) and remember it.
    fn set_speed_limit(&mut self, kib: Option<u64>) {
        self.settings.speed_limit_kib = kib;
        self.rate_limit.set_rate(kib.unwrap_or(0).saturating_mul(1024));
        self.save_settings();
    }

    /// "Default choice on conflict" for drag and drop (`drop`) or F5.
    fn set_conflict_preset(&mut self, drop: bool, p: rolid_config::ConflictPreset) {
        if drop {
            self.settings.drop_conflict = Some(p);
        } else {
            self.settings.copy_conflict = Some(p);
        }
        self.save_settings();
    }

    /// Install the text font and size from the settings.
    pub(super) fn apply_font(&mut self) {
        let a = crate::fonts::apply(&self.ctx, &crate::fonts::Spec::from_settings(&self.settings));
        tracing::info!("font: {}", a.desc);
        if let Some(e) = a.error {
            tracing::warn!("system font: {e}, using the built-in one");
            self.last_error = Some(tr!("System font: {error}. The built-in {font} is used.", error = e.as_str(), font = crate::fonts::NOTO_NAME));
        }
        self.font_desc = a.desc;
    }

    fn save_settings(&mut self) {
        if let Err(e) = rolid_config::save_file(SETTINGS_FILE, &self.settings) {
            self.last_error = Some(tr!("Settings: {error}", error = e));
        }
    }

    /// Color for a new tab: the first preset not yet used in that panel.
    fn free_color(&self, side: usize) -> Rgb {
        let used: Vec<Rgb> = self.sides[side].tabs.iter().map(|t| t.color).collect();
        PASTEL_PRESETS
            .iter()
            .map(|(_, c)| *c)
            .find(|c| !used.contains(c))
            .unwrap_or(PASTEL_PRESETS[self.sides[side].tabs.len() % PASTEL_PRESETS.len()].1)
    }

    /// Add a tab to a panel (after the current one), make it active and load it.
    fn push_tab(&mut self, side: usize, pane: Pane, color: Option<Rgb>, name: Option<String>) -> PaneRef {
        let id = self.id();
        let color = color.unwrap_or_else(|| self.free_color(side));
        let s = &self.sides[side];
        let pos = if s.tabs.is_empty() { 0 } else { s.active + 1 };
        self.insert_tab(side, pos, Tab { id, name, color, pane })
    }

    /// Insert a tab at `pos` of a panel, make it current and load it.
    fn insert_tab(&mut self, side: usize, pos: usize, tab: Tab) -> PaneRef {
        let id = tab.id;
        let s = &mut self.sides[side];
        let pos = pos.min(s.tabs.len());
        s.tabs.insert(pos, tab);
        s.active = pos;
        let at = PaneRef { tab: id, side };
        if let Some(p) = self.pane(at) {
            if !p.offline {
                let path = p.path.clone();
                self.load(at, path, None);
            }
        }
        at
    }

    fn close_tab(&mut self, side: usize, idx: usize) {
        let s = &mut self.sides[side];
        if s.tabs.len() <= 1 || idx >= s.tabs.len() {
            return;
        }
        // Edited files of this tab with changes not on the server yet: ask first.
        let tab_id = s.tabs[idx].id;
        if self.edits_block_tab_close(tab_id) {
            return;
        }
        let s = &mut self.sides[side];
        let t = s.tabs.remove(idx);
        if let Some(sv) = &t.pane.search {
            sv.cancel.store(true, Ordering::Relaxed);
        }
        s.mru.retain(|x| *x != t.id);
        let next = s.mru.first().and_then(|id| s.index(*id)).unwrap_or(0);
        // A pending login question of this tab is canceled (dropping it answers "no").
        self.auth_prompts.retain(|f| f.at.tab != t.id);
        self.activate_tab(side, next);
    }

    fn close_active_tab(&mut self) {
        let side = self.active_side;
        self.close_tab(side, self.sides[side].active);
    }

    /// Show tab `idx` in panel `side` and make that panel active.
    fn activate_tab(&mut self, side: usize, idx: usize) {
        if idx >= self.sides[side].tabs.len() {
            return;
        }
        // Another tab: a rename typed in the list ends.
        for t in self.sides.iter_mut().flat_map(|s| s.tabs.iter_mut()) {
            t.pane.rename = None;
        }
        self.active_side = side;
        let s = &mut self.sides[side];
        s.active = idx;
        let id = s.tabs[idx].id;
        if self.switching.is_none() {
            s.mru.retain(|t| *t != id);
            s.mru.insert(0, id);
        }
        // Refresh the listing in the background if it is older than a few seconds;
        // the cached listing is shown immediately, so switching is instant.
        let at = PaneRef { tab: id, side };
        let p = &self.sides[side].tabs[idx].pane;
        if p.offline {
            if !p.connecting && self.switching.is_none() {
                self.connect_pane(at);
            }
            return;
        }
        self.refresh_if_stale(at);
    }

    /// Next / previous tab in the active panel.
    fn step_tab(&mut self, dir: isize) {
        let side = self.active_side;
        let n = self.sides[side].tabs.len() as isize;
        let i = (self.sides[side].active as isize + dir).rem_euclid(n) as usize;
        self.activate_tab(side, i);
    }

    fn tab_mut(&mut self, at: PaneRef) -> Option<&mut Tab> {
        self.sides.get_mut(at.side)?.tabs.iter_mut().find(|t| t.id == at.tab)
    }

    fn pane(&self, at: PaneRef) -> Option<&Pane> {
        find_pane(&self.sides, at)
    }

    fn pane_mut(&mut self, at: PaneRef) -> Option<&mut Pane> {
        self.tab_mut(at).map(|t| &mut t.pane)
    }

    /// All panels in all tabs.
    fn all_panes(&self) -> impl Iterator<Item = (PaneRef, &Pane)> {
        self.sides.iter().enumerate().flat_map(|(side, s)| {
            s.tabs.iter().map(move |t| (PaneRef { tab: t.id, side }, &t.pane))
        })
    }

    fn active_ref(&self) -> PaneRef {
        let side = self.active_side;
        PaneRef {
            tab: self.sides[side].current().id,
            side,
        }
    }

    /// The panel shown on the other side (its active tab).
    fn other(&self, at: PaneRef) -> PaneRef {
        let side = 1 - at.side;
        PaneRef {
            tab: self.sides[side].current().id,
            side,
        }
    }

    // ---------------------------------------------------------------- tasks

    fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = Msg> + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let msg = fut.await;
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn load(&mut self, at: PaneRef, path: String, select: Option<String>) {
        let Some(p) = self.pane_mut(at) else { return };
        if p.offline {
            self.connect_pane(at);
            return;
        }
        p.req_id += 1;
        p.loading = true;
        let req = p.req_id;
        let fs = p.fs.clone();
        self.spawn(async move {
            let result = fs.list(&path).await;
            Msg::Listed {
                at,
                req,
                path,
                select,
                result,
            }
        });
    }

    /// Free space and file system type of the disk shown in `at` (in the
    /// background, after each listing). Inside an archive: the disk the archive is on.
    fn request_space(&self, at: PaneRef, path: String) {
        let Some(p) = self.pane(at) else { return };
        let mut outer = p.archive.as_deref();
        while let Some(b) = outer.and_then(|a| a.back_archive.as_deref()) {
            outer = Some(b);
        }
        let (fs, disk_path, remote) = match outer {
            Some(a) => (a.back_fs.clone(), a.back_path.clone(), a.back_remote.clone()),
            None => (p.fs.clone(), path.clone(), p.remote.clone()),
        };
        let link = remote.as_ref().and_then(|r| self.link(&r.target.key()));
        let local = remote.is_none() && !fs.is_remote();
        self.spawn(async move {
            let space = match tokio::time::timeout(Duration::from_secs(10), fs.space(&disk_path)).await {
                Ok(Ok(s)) => s,
                _ => None,
            };
            use rolid_disk::physical::NoSmart;
            let (fstype, disks) = if local {
                let dp = disk_path.clone();
                tokio::task::spawn_blocking(move || {
                    let fstype = rolid_disk::mounts::mount_of(&dp).map(|m| m.fstype);
                    (fstype, rolid_disk::health::disks_of_path(&dp))
                })
                .await
                .unwrap_or((None, Err(NoSmart::NoDisk)))
            } else if let Some(l) = link {
                (remote_fs_type(&l, &disk_path).await, Err(NoSmart::Server))
            } else {
                (None, Err(NoSmart::Server))
            };
            Msg::Space { at, path, space, fstype, disks }
        });
    }

    /// Panes (in all tabs) that show the same location as `at`.
    fn same_location(&self, at: PaneRef) -> Vec<PaneRef> {
        let Some(p) = self.pane(at) else {
            return vec![];
        };
        let key = (p.fs.is_remote(), p.fs.label(), p.path.clone());
        self.all_panes()
            .filter(|(_, q)| {
                q.search.is_none() && (q.fs.is_remote(), q.fs.label(), q.path.clone()) == key
            })
            .map(|(r, _)| r)
            .collect()
    }

    fn refresh(&mut self, at: PaneRef) {
        if let Some(p) = self.pane(at) {
            let path = p.path.clone();
            self.load(at, path, None);
        }
    }

    /// Navigate to a user-typed path: resolve it first (handles `~`, relative paths).
    fn goto(&mut self, at: PaneRef, input: String) {
        let Some(p) = self.pane(at) else { return };
        // Inside an archive a path outside it (absolute, "~") leaves the archive first.
        if let Some(st) = &p.archive {
            let raw = input.trim();
            let inside = !(raw.is_empty() || raw.starts_with('/') || raw.starts_with('~') || (cfg!(windows) && raw.contains(':')))
                || st.fs.inner(raw).is_some_and(|_| raw.starts_with(st.fs.name.as_str()));
            if !inside {
                while self.pane(at).is_some_and(|p| p.archive.is_some()) {
                    self.leave_archive(at);
                }
                return self.goto(at, input);
            }
        }
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let base = p.path.clone();
        self.spawn(async move {
            let raw = input.trim().to_string();
            let target = if raw == "~" || raw.is_empty() {
                fs.home().await.unwrap_or(base)
            } else if let Some(rest) = raw.strip_prefix("~/") {
                let h = fs.home().await.unwrap_or_default();
                fs.join(&h, rest)
            } else if raw.starts_with('/') || (cfg!(windows) && raw.contains(':')) {
                raw
            } else {
                fs.join(&base, &raw)
            };
            let path = fs.canonicalize(&target).await.unwrap_or(target);
            Msg::Resolved {
                at,
                path,
                select: None,
            }
        });
    }

    fn handle_msgs(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Listed {
                    at,
                    req,
                    path,
                    select,
                    result,
                } => {
                    let Some(p) = self.pane_mut(at) else { continue };
                    if p.req_id != req {
                        continue; // stale
                    }
                    match result {
                        Ok(entries) => {
                            p.set_listing(path.clone(), entries, select.as_deref());
                            self.request_space(at, path);
                            self.remember_place(at);
                        }
                        Err(e) => {
                            p.loading = false;
                            p.history.cancel_nav();
                            let msg = format!("{path}: {}", rolid_i18n::t(&e.text()));
                            if p.entries.is_empty() {
                                p.error = Some(msg.clone());
                            }
                            // A lost connection is shown in the panel's strip.
                            if !e.is_connection_lost() {
                                self.last_error = Some(msg);
                            }
                        }
                    }
                }
                Msg::Resolved { at, path, select } => self.load(at, path, select),
                Msg::DirChanged(path) => self.on_dir_changed(path),
                Msg::Refreshed { key, result } => self.on_refreshed(key, result),
                Msg::PickListed { req, path, result } => self.on_pick_listed(req, path, result),
                Msg::CopyOverwrite { dest, n } => {
                    if let Dialog::Copy { dest: d, overwrite, .. } = &mut self.dialog {
                        if *d == dest {
                            *overwrite = n;
                        }
                    }
                }
                Msg::Connected { at, meta, result } => self.on_connected(at, meta, *result),
                Msg::AuthPrompt(f) => self.auth_prompts.push(*f),
                Msg::Vault(m) => self.on_vault_msg(*m),
                Msg::Edit(m) => self.on_edit_msg(*m),
                Msg::Note(n) => {
                    if n.is_some() {
                        self.last_error = n;
                    }
                }
                Msg::DropReady(r) => self.on_drop_ready(*r),
                Msg::DiskHealth(h) => self.on_disk_health(*h),
                Msg::DiskTemps(t, at) => self.on_disk_temps(t, at),
                Msg::Space { at, path, space, fstype, disks } => {
                    let mut read = None;
                    if let Some(p) = self.pane_mut(at) {
                        if p.path == path {
                            p.space = space;
                            p.fstype = fstype;
                            read = disks.as_ref().ok().cloned();
                            p.disks = Some(disks);
                        }
                    }
                    match read {
                        Some(names) => {
                            self.request_health(&names, false);
                            self.update_pane_health();
                        }
                        None => self.update_pane_health(),
                    }
                }
                Msg::PermsPlanned(m) => self.on_perms_planned(*m),
                Msg::JobDone {
                    id,
                    refresh,
                    result,
                    failures,
                    retry,
                    then,
                } => {
                    let title = self.jobs.iter().find(|j| j.id == id).map(|j| j.title.clone()).unwrap_or_default();
                    self.jobs.retain(|j| j.id != id);
                    let kind = self.job_kinds.remove(&id);
                    let canceled = result.as_ref().is_err_and(|e| e.is_canceled());
                    if !failures.is_empty() && (retry.is_none() || canceled) {
                        self.drop.reports.push(drop_ui::Report { id, title: title.clone(), failures });
                    }
                    if let Err(e) = result {
                        match retry {
                            Some(spec) if !e.is_canceled() => {
                                let error = if e.is_connection_lost() && !spec.auto_retry {
                                    tr!(
                                        "{error}. On a production server overwriting is not repeated by itself after an outage: \
                                         check the state and press “Try again”.",
                                        error = e.text()
                                    )
                                } else {
                                    rolid_i18n::t(&e.text())
                                };
                                self.failed_jobs.push(FailedJob { id, title: spec.title.clone(), error, spec });
                            }
                            _ if kind == Some(JobKind::Perms) => {
                                if !e.is_canceled() {
                                    self.last_error = Some(format!("{title}: {}", rolid_i18n::t(&e.text())));
                                }
                            }
                            _ if kind == Some(JobKind::Move) => {
                                self.last_error = Some(tr!("{title}: {error}. The source stayed where it was.", title = title.as_str(), error = e.text()));
                            }
                            _ => self.last_error = Some(tr!("Copying: {error}", error = e.text())),
                        }
                    }
                    let all: Vec<PaneRef> = refresh
                        .iter()
                        .flat_map(|r| self.same_location(*r))
                        .collect();
                    for r in all {
                        self.refresh(r);
                    }
                    if let Some(f) = then {
                        f(self);
                    }
                }
                Msg::Then { job, f } => {
                    if let Some(id) = job {
                        self.jobs.retain(|j| j.id != id);
                    }
                    f(self);
                }
                Msg::SearchDone { at, id, result } => self.on_search_done(at, id, result),
                Msg::Removed {
                    at,
                    labels,
                    refresh,
                    result,
                } => {
                    if let Err(e) = result {
                        self.last_error = Some(rolid_i18n::t(&e.text()));
                    }
                    if let Some(p) = self.pane_mut(at) {
                        p.remove_results(&labels);
                    }
                    let all: Vec<PaneRef> = refresh
                        .iter()
                        .flat_map(|r| self.same_location(*r))
                        .collect();
                    for r in all {
                        self.refresh(r);
                    }
                }
                Msg::OpDone {
                    refresh,
                    select,
                    result,
                } => {
                    if let Err(e) = result {
                        self.last_error = Some(rolid_i18n::t(&e.text()));
                    }
                    let mut all: Vec<PaneRef> = refresh
                        .iter()
                        .flat_map(|r| self.same_location(*r))
                        .collect();
                    all.dedup();
                    for r in all {
                        let sel = select
                            .as_ref()
                            .filter(|(s, _)| *s == r)
                            .map(|(_, n)| n.clone());
                        if let Some(p) = self.pane(r) {
                            let path = p.path.clone();
                            self.load(r, path, sel);
                        }
                    }
                }
            }
        }
    }

    fn set_local(&mut self, at: PaneRef) {
        if let Some(p) = self.pane_mut(at) {
            let home = Self::home_dir();
            *p = Pane::new(Arc::new(LocalFs), home.clone());
            self.load(at, home, None);
        }
    }

    // ----------------------------------------------------------- operations

    /// A command from a row's context menu.
    fn pane_command(&mut self, at: PaneRef, cmd: crate::pane::PaneCmd) {
        use crate::pane::PaneCmd;
        match cmd {
            PaneCmd::Open => self.activate_entry(at),
            PaneCmd::OpenWith(o) => self.open_with(at, o),
            PaneCmd::OpenWithDialog => self.ask_open_with(at),
            PaneCmd::Perms => self.ask_perms(at),
            PaneCmd::CopyNames => self.copy_to_clipboard(at, ctx_ui::ClipKind::Names),
            PaneCmd::CopyPaths => self.copy_to_clipboard(at, ctx_ui::ClipKind::Paths),
            PaneCmd::CopyUrls => self.copy_to_clipboard(at, ctx_ui::ClipKind::Urls),
            PaneCmd::View => self.open_viewer(at),
            PaneCmd::Edit => self.start_edit(at),
            PaneCmd::NewFile => self.ask_new_file(),
            PaneCmd::Copy => self.ask_copy(),
            PaneCmd::Move => self.ask_move(),
            PaneCmd::Rename => self.start_rename_active(),
            PaneCmd::Delete => self.ask_delete(),
            PaneCmd::Pack => self.ask_pack(),
            PaneCmd::Unpack => self.ask_archive(unpack_ui::Purpose::Extract),
            PaneCmd::TestArchive => self.ask_archive(unpack_ui::Purpose::Test),
            PaneCmd::SizeSelected => self.size_selected_dirs(),
            PaneCmd::SizeAll => self.size_all_dirs(),
            PaneCmd::Checksums => self.ask_checksums(),
            PaneCmd::NewFolder => self.ask_mkdir(),
            PaneCmd::Refresh => self.refresh_or_rerun(),
            PaneCmd::AddFavorite => self.add_current_favorite(),
            PaneCmd::Terminal => self.open_terminal(),
            PaneCmd::DiskProps => self.open_disk_props(at),
            PaneCmd::ToggleMark => {
                if let Some(p) = self.pane_mut(at) {
                    p.toggle_mark_current();
                }
            }
        }
    }

    fn activate_entry(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        if p.offline {
            self.connect_pane(at);
            return;
        }
        if p.search.is_some() {
            match p.current_row() {
                Some(Row::Up) => self.leave_search(at),
                Some(Row::Entry(i)) => {
                    let label = p.entries[i].name.clone();
                    let Some(h) = p.hit(&label) else { return };
                    let (path, select) = if h.entry.is_dir_like() {
                        (p.fs.join(&h.dir, &h.entry.name), None)
                    } else {
                        (h.dir.clone(), Some(h.entry.name.clone()))
                    };
                    if let Some(p) = self.pane_mut(at) {
                        p.exit_search();
                    }
                    self.load(at, path, select);
                }
                None => {}
            }
            return;
        }
        match p.current_row() {
            Some(Row::Up) => self.go_up(at),
            Some(Row::Entry(i)) => {
                let e = &p.entries[i];
                use crate::open_rule::{decide, OpenAction, Where};
                let place = if p.fs.read_only() {
                    Where::Archive
                } else if p.fs.is_local_disk() {
                    Where::Local
                } else {
                    Where::Remote
                };
                let path = p.fs.join(&p.path, &e.name);
                let mut e = e.clone();
                let mut head = Vec::new();
                if place == Where::Local && !e.is_dir_like() {
                    // A link: the target's permissions count; scripts start with "#!".
                    if let Ok(m) = std::fs::metadata(&path) {
                        use std::os::unix::fs::PermissionsExt;
                        e.mode = Some(m.permissions().mode());
                    }
                    head = edit_ui::read_head(std::path::Path::new(&path));
                }
                let target = Target { dir: p.path.clone(), label: e.name.clone(), name: e.name.clone(), size: e.size };
                match decide(&e, place, &head) {
                    OpenAction::EnterDir => self.load(at, path, None),
                    OpenAction::OpenArchive => self.enter_archive(at, target),
                    OpenAction::OpenDefault => self.open_file(at),
                    OpenAction::AskRun { reason, executable } => {
                        self.edits.prompts.push(edit_ui::EditPrompt::LocalExec { path, name: e.name.clone(), reason, executable })
                    }
                    OpenAction::EditorOnly => self.open_from_archive(at, target, crate::pane::Opener::Editor),
                    OpenAction::Nothing => {}
                }
            }
            None => {}
        }
    }

    /// Leave search results and show the searched directory again.
    fn leave_search(&mut self, at: PaneRef) {
        let Some(p) = self.pane_mut(at) else { return };
        p.exit_search();
        let path = p.path.clone();
        self.load(at, path, None);
    }

    fn go_up(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        if p.search.is_some() {
            self.leave_search(at);
            return;
        }
        if p.archive.is_some() && p.fs.parent(&p.path).is_none() {
            self.leave_archive(at);
            return;
        }
        if let Some(parent) = p.fs.parent(&p.path) {
            let cur = p
                .path
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .map(str::to_owned);
            self.load(at, parent, cur);
        }
    }

    fn ask_copy(&mut self) {
        let from = self.active_ref();
        let to = self.other(from);
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else {
            return;
        };
        let items = src.target_items();
        let dest = dst.path.clone();
        self.open_copy(from, to, dest, items);
    }

    /// The Copy dialog for `items` of panel `from` into folder `dest` of panel `to`.
    pub(super) fn open_copy(&mut self, from: PaneRef, to: PaneRef, dest: String, items: Vec<Target>) {
        let Some(dst) = self.pane(to) else { return };
        if dst.search.is_some() {
            self.last_error =
                Some(tr!("The target panel shows search results: open a folder in it first."));
            return;
        }
        if self.refuse_read_only(to, &tr!("Cannot copy into an archive")) {
            return;
        }
        if items.is_empty() {
            return;
        }
        self.dialog = Dialog::Copy {
            from,
            to,
            dest,
            items,
            overwrite: 0,
            conflict: preset_conflict(self.settings.copy_preset()),
        };
        self.recount_copy();
    }

    /// The folder `path` on the disk or server of the panel `at`.
    pub(super) fn location_at(&self, at: PaneRef, path: &str) -> Option<rolid_config::Location> {
        let mut l = self.pane(at)?.location();
        match &mut l {
            rolid_config::Location::Local { path: p } | rolid_config::Location::Remote { path: p, .. } => *p = path.to_string(),
        }
        Some(l)
    }

    /// Recent targets on the disk or server of the panel `at`.
    pub(super) fn targets_on(&self, at: PaneRef) -> Vec<String> {
        self.pane(at).map(|p| self.places.targets_on(&p.location())).unwrap_or_default()
    }

    /// How many of the items of the copy dialog exist in its target already
    /// (the panel's own folder at once, another folder in the background).
    pub(super) fn recount_copy(&mut self) {
        let Dialog::Copy { to, dest, items, .. } = &self.dialog else { return };
        let Some(dst) = self.pane(*to) else { return };
        let n = if *dest == dst.path {
            items.iter().filter(|t| dst.entries.iter().any(|e| e.name == t.name)).count()
        } else {
            let (dfs, d, names) = (dst.fs.clone(), dest.clone(), items.iter().map(|t| t.name.clone()).collect::<Vec<_>>());
            self.spawn(async move {
                let mut n = 0;
                for name in names {
                    if dfs.stat(&dfs.join(&d, &name)).await.is_ok() {
                        n += 1;
                    }
                }
                Msg::CopyOverwrite { dest: d, n }
            });
            0
        };
        if let Dialog::Copy { overwrite, .. } = &mut self.dialog {
            *overwrite = n;
        }
    }

    fn start_copy(&mut self, from: PaneRef, to: PaneRef, dest: String, items: Vec<Target>, conflict: Conflict) {
        if self.pane(from).is_some_and(|p| p.archive.is_some()) {
            return self.copy_out_of_archive(from, to, dest, items, conflict);
        }
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else {
            return;
        };
        let (sfs, dfs) = (src.fs.clone(), dst.fs.clone());
        let dpath = dest;
        if let Some(l) = self.location_at(to, &dpath) {
            self.places.remember_target(l);
        }
        let Some(dst) = self.pane(to) else { return };
        let dst_label = dst.display_label();
        let production = dst.remote.as_ref().is_some_and(|r| r.production);
        let title = if items.len() == 1 {
            format!("{} → {dst_label}", items[0].name)
        } else {
            trn!("{n} item → {target}", "{n} items → {target}", items.len(), target = dst_label.as_str())
        };
        let links = [self.pane_link(from), self.pane_link(to)].into_iter().flatten().collect();
        if let Some(p) = self.pane_mut(from) {
            p.marked.clear();
        }
        self.spawn_copy(CopySpec {
            sfs,
            dfs,
            dpath,
            items,
            conflict,
            title,
            refresh: vec![to],
            links,
            // Skip never overwrites; other policies on production need a new "yes".
            auto_retry: !production || conflict == Conflict::Skip,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: None,
            temp: None,
            moving: false,
            target_name: None,
        });
    }

    /// Run a copy job through the queue. After a lost connection it waits for the
    /// reconnection (freeing its queue slot meanwhile) and continues from the
    /// `.filepart` – at most [`MAX_AUTO_RETRIES`] times.
    fn spawn_copy(&mut self, spec: CopySpec) {
        let total_bytes = spec.items.iter().map(|t| t.size).sum();
        let totals = Arc::new(Mutex::new(Totals::default()));
        let id = self.id();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let note = Arc::new(Mutex::new(None));
        self.jobs.push(Job {
            id,
            title: spec.title.clone(),
            progress: progress.clone(),
            total_bytes,
            cancel: cancel.clone(),
            note: note.clone(),
            count: None,
            totals: totals.clone(),
        });
        if spec.moving {
            self.job_kinds.insert(id, JobKind::Move);
        }
        // Count files and bytes meanwhile ("file 3 of 120", progress of folders).
        let measured = Arc::new(AtomicBool::new(false));
        {
            let (fs, items, stop, totals, ctx) = (
                spec.sfs.clone(),
                spec.items.iter().map(|t| (t.dir.clone(), t.name.clone())).collect::<Vec<_>>(),
                measured.clone(),
                totals.clone(),
                self.ctx.clone(),
            );
            let cancel = cancel.clone();
            self.rt.spawn(async move {
                let upd = |t: Totals| {
                    *totals.lock().unwrap() = t;
                    ctx.request_repaint_after(Duration::from_millis(200));
                };
                // Stop when the job is canceled or finished.
                let stop2 = stop.clone();
                let watch = async {
                    while !cancel.load(Ordering::Relaxed) && !stop2.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    stop2.store(true, Ordering::Relaxed);
                };
                tokio::select! {
                    _ = rolid_core::copy::measure(fs.as_ref(), &items, &stop, &upd) => {}
                    _ = watch => {}
                }
            });
        }
        let ctx = self.ctx.clone();
        let slots = self.transfer_slots.clone();
        let limit = self.rate_limit.clone();
        self.spawn(async move {
            // Queue: wait for a free transfer slot (cancel works while waiting too).
            let acquire = |slots: Arc<tokio::sync::Semaphore>, cancel: Arc<AtomicBool>| async move {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    if let Ok(p) = slots.clone().try_acquire_owned() {
                        return Some(p);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            };
            let permit = Arc::new(Mutex::new(acquire(slots.clone(), cancel.clone()).await));
            if permit.lock().unwrap().is_none() {
                return Msg::JobDone {
                    id,
                    refresh: vec![],
                    result: Err(rolid_core::VfsError::Backend(rolid_i18n::rolid_text::txt!("canceled in the queue"))),
                    failures: vec![],
                    retry: None,
                    then: None,
                };
            }
            progress.lock().unwrap().started = Some(Instant::now());
            let ctx2 = ctx.clone();
            let cb = move |p: &Progress| {
                *progress.lock().unwrap() = p.clone();
                ctx2.request_repaint_after(Duration::from_millis(100));
            };
            let mut job = CopyJob::new(spec.sfs.as_ref(), spec.dfs.as_ref(), cancel.clone(), &cb)
                .with_conflict(spec.conflict)
                .with_limit(limit);
            if let Some(st) = spec.state.clone() {
                job = job.with_state(st);
            }
            if spec.keep_going {
                job = job.keep_going();
            }
            if spec.moving {
                job = job.moving().with_target_name(spec.target_name.clone());
            }
            let items: Vec<(String, String)> = spec.items.iter().map(|t| (t.dir.clone(), t.name.clone())).collect();
            let max = if spec.auto_retry { MAX_AUTO_RETRIES } else { 0 };
            let links = spec.links.clone();
            let result = job
                .copy_items_retrying(&items, &spec.dpath, max, |n| {
                    let (links, cancel, permit, slots, note, ctx) =
                        (links.clone(), cancel.clone(), permit.clone(), slots.clone(), note.clone(), ctx.clone());
                    async move {
                        // Free the slot for other transfers while waiting.
                        permit.lock().unwrap().take();
                        *note.lock().unwrap() =
                            Some(tr!("⚡ waiting for the connection to come back (attempt {n}/{max})", n = n, max = MAX_AUTO_RETRIES));
                        ctx.request_repaint();
                        let mut ok = true;
                        for l in &links {
                            ok = ok && l.wait_up(&cancel).await;
                        }
                        let p = if ok { acquire(slots, cancel).await } else { None };
                        ok = p.is_some();
                        *permit.lock().unwrap() = p;
                        *note.lock().unwrap() = None;
                        ctx.request_repaint();
                        ok
                    }
                })
                .await;
            measured.store(true, Ordering::Relaxed);
            // A move: only now, after the whole copy succeeded, the sources go.
            let cleanup = if spec.moving && result.is_ok() { Some(job.delete_moved_sources().await) } else { None };
            let state = job.state().clone();
            let failures = drop_ui::merge_failures(spec.pre_failed.clone(), job.failures());
            drop(job);
            let refresh = spec.refresh.clone();
            let mut spec = spec;
            let mut then = if result.is_ok() { spec.then.take() } else { None };
            if let Some(c) = cleanup.filter(|c| !c.kept.is_empty() || c.kept_dirs > 0) {
                let report: Then = Box::new(move |a: &mut App| {
                    let mut msg = if c.kept_dirs > 0 {
                        tr!(
                            "The move finished, but {files} and {dirs} stayed in the source (skipped by the choice for existing files, links, or files changed during the move).",
                            files = rolid_i18n::rolid_text::txtn!("{n} file", "{n} files", c.kept.len()),
                            dirs = rolid_i18n::rolid_text::txtn!("{n} folder", "{n} folders", c.kept_dirs)
                        )
                    } else {
                        trn!(
                            "The move finished, but {n} file stayed in the source (skipped by the choice for existing files, links, or files changed during the move).",
                            "The move finished, but {n} files stayed in the source (skipped by the choice for existing files, links, or files changed during the move).",
                            c.kept.len()
                        )
                    };
                    if let Some(first) = c.kept.first() {
                        msg += " ";
                        msg += &tr!("E.g. {path}", path = first.as_str());
                    }
                    a.last_error = Some(msg);
                });
                then = Some(match then {
                    Some(f) => Box::new(move |a: &mut App| {
                        f(a);
                        report(a);
                    }),
                    None => report,
                });
            }
            // A retry keeps what failed so far, for the final summary.
            let retry = result
                .is_err()
                .then(|| Box::new(CopySpec { state: Some(state), pre_failed: failures.clone(), ..spec }));
            Msg::JobDone { id, refresh, result, failures, retry, then }
        });
    }

    /// "Try again" for a failed transfer: continue where it stopped.
    fn retry_failed_job(&mut self, id: u64) {
        if let Some(i) = self.failed_jobs.iter().position(|f| f.id == id) {
            let f = self.failed_jobs.remove(i);
            self.spawn_copy(*f.spec);
        }
    }

    fn start_mkdir(&mut self, at: PaneRef, name: String) {
        let name = name.trim().to_string();
        if let Err(e) = validate_name(&name) {
            self.last_error = Some(rolid_i18n::t(&e.text()));
            return;
        }
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let path = fs.join(&p.path, &name);
        self.spawn(async move {
            let result = fs.mkdir(&path).await;
            Msg::OpDone {
                refresh: vec![at],
                select: Some((at, name)),
                result,
            }
        });
    }

    fn start_delete(&mut self, at: PaneRef, items: Vec<Target>) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let in_search = p.search.is_some();
        // Panels that show the directories the deleted items were in.
        let dirs: Vec<PaneRef> = if in_search {
            self.all_panes()
                .filter(|(_, q)| {
                    q.search.is_none()
                        && Arc::ptr_eq(&q.fs, &fs)
                        && items.iter().any(|i| i.dir == q.path)
                })
                .map(|(r, _)| r)
                .collect()
        } else {
            Vec::new()
        };
        if let Some(p) = self.pane_mut(at) {
            p.marked.clear();
        }
        self.spawn(async move {
            let mut result = Ok(());
            let mut done = Vec::new();
            for t in &items {
                if let Err(e) = remove_recursive(fs.as_ref(), &fs.join(&t.dir, &t.name)).await {
                    result = Err(e);
                    break;
                }
                done.push(t.label.clone());
            }
            if in_search {
                return Msg::Removed {
                    at,
                    labels: done,
                    refresh: dirs,
                    result,
                };
            }
            Msg::OpDone {
                refresh: vec![at],
                select: None,
                result,
            }
        });
    }

    // ------------------------------------------------------------- keyboard

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Commit Ctrl+Tab cycling once Ctrl is released.
        if self.switching.is_some() && !ctx.input(|i| i.modifiers.ctrl) {
            self.switching = None;
            let s = self.active_side;
            self.activate_tab(s, self.sides[s].active);
        }
        let path_editing = self.sides.iter().any(|s| s.current().pane.path_edit.is_some());
        // Also in the frames before the name field has the focus (after F2, after
        // "Cancel" in a question): keys typed then belong to the field, not the panel.
        let inline_renaming = self.sides.iter().any(|s| s.current().pane.rename.is_some() || s.current().pane.tree_open.is_some());
        if !matches!(self.dialog, Dialog::None)
            || !self.auth_prompts.is_empty()
            || self.vault.blocks_input()
            || self.edits_blocks_input()
            || self.renaming.is_some()
            || path_editing
            || inline_renaming
            || ctx.text_edit_focused()
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        // No text field is active: the keyboard belongs to the panels. Drop any focus
        // egui gave to buttons (e.g. via its own Tab navigation) so it can't steal keys.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });

        let mut actions: Vec<Action> = Vec::new();
        ctx.input_mut(|i| {
            let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
            macro_rules! on {
                ($m:expr, $k:expr, $f:expr) => {
                    if i.consume_key($m, $k) {
                        actions.push(Box::new($f));
                    }
                };
            }
            on!(ctrl_shift, Key::Tab, |a: &mut Self| a.cycle_tabs(-1));
            // Ctrl+Shift+… before Ctrl+… (egui ignores an extra Shift when matching).
            on!(ctrl_shift, Key::N, |a: &mut Self| a.copy_to_clipboard(a.active_ref(), ctx_ui::ClipKind::Names));
            on!(ctrl_shift, Key::P, |a: &mut Self| a.copy_to_clipboard(a.active_ref(), ctx_ui::ClipKind::Paths));
            on!(ctrl_shift, Key::S, |a: &mut Self| a.copy_to_clipboard(a.active_ref(), ctx_ui::ClipKind::Urls));
            on!(ctrl_shift, Key::A, |a: &mut Self| a.ask_perms(a.active_ref()));
            // Listing styles (Ctrl+Shift+F1 before Ctrl+F1).
            on!(ctrl_shift, Key::F1, |a: &mut Self| a.set_view_style(rolid_config::ViewStyle::Thumbs));
            on!(Modifiers::CTRL, Key::F1, |a: &mut Self| a.set_view_style(rolid_config::ViewStyle::Brief));
            on!(Modifiers::CTRL, Key::F2, |a: &mut Self| a.set_view_style(rolid_config::ViewStyle::Full));
            // Alt+… before the plain keys (egui ignores an extra Alt when matching).
            on!(Modifiers::ALT, Key::ArrowLeft, |a: &mut Self| a.history_go(false));
            on!(Modifiers::ALT, Key::ArrowRight, |a: &mut Self| a.history_go(true));
            on!(Modifiers::ALT, Key::Equals, |a: &mut Self| a.ask_mark_group(true));
            on!(Modifiers::ALT, Key::Minus, |a: &mut Self| a.ask_mark_group(false));
            on!(Modifiers::ALT, Key::I, |a: &mut Self| a.invert_selection());
            on!(Modifiers::ALT, Key::F5, |a: &mut Self| a.ask_pack());
            on!(Modifiers::ALT | Modifiers::SHIFT, Key::F9, |a: &mut Self| a.ask_archive(unpack_ui::Purpose::Test));
            on!(Modifiers::ALT, Key::F9, |a: &mut Self| a.ask_archive(unpack_ui::Purpose::Extract));
            // Num+ (only the numeric keypad: on the main keyboard "+" is typed into
            // the quick filter, e.g. on the Czech layout it is the "1" key).
            let mut num_plus = false;
            i.events.retain(|e| match e {
                egui::Event::Key { physical_key: Some(Key::Plus), pressed, modifiers, .. }
                    if !modifiers.ctrl && !modifiers.alt && !modifiers.command =>
                {
                    num_plus |= *pressed;
                    false
                }
                _ => true,
            });
            if num_plus {
                i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t == "+"));
                actions.push(Box::new(|a: &mut Self| a.ask_mark_group(true)));
            }
            // Before Shift+Enter: egui's matching ignores an extra Alt.
            on!(Modifiers::ALT | Modifiers::SHIFT, Key::Enter, |a: &mut Self| a.size_all_dirs());
            on!(Modifiers::SHIFT, Key::Enter, |a: &mut Self| a.ask_open_with(a.active_ref()));
            on!(Modifiers::CTRL, Key::Tab, |a: &mut Self| a.cycle_tabs(1));
            on!(Modifiers::CTRL, Key::PageDown, |a: &mut Self| a.step_tab(1));
            on!(Modifiers::CTRL, Key::PageUp, |a: &mut Self| a.step_tab(-1));
            for (n, k) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                on!(Modifiers::CTRL, k, move |a: &mut Self| {
                    // Ctrl+9 = last tab, as in browsers.
                    let side = a.active_side;
                    let idx = if n == 8 { a.sides[side].tabs.len() - 1 } else { n };
                    a.activate_tab(side, idx)
                });
            }
            on!(Modifiers::CTRL, Key::T, |a: &mut Self| a.open_new_tab_menu(a.active_side));
            on!(Modifiers::CTRL, Key::D, |a: &mut Self| a.toggle_favorite(a.active_ref()));
            on!(Modifiers::CTRL, Key::W, |a: &mut Self| a.close_active_tab());
            on!(Modifiers::CTRL, Key::P, |a: &mut Self| a.open_terminal());
            on!(Modifiers::CTRL, Key::K, |a: &mut Self| a.open_switcher());
            on!(Modifiers::CTRL, Key::Q, |a: &mut Self| a
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::Close));
            on!(Modifiers::NONE, Key::F1, |a: &mut Self| a.dialog = Dialog::Help(false));
            on!(Modifiers::CTRL, Key::O, |a: &mut Self| a.open_connect());
            on!(Modifiers::CTRL, Key::L, |a: &mut Self| a
                .set_local(a.active_ref()));
            on!(Modifiers::CTRL, Key::R, |a: &mut Self| a.refresh_or_rerun());
            on!(Modifiers::ALT, Key::ArrowDown, |a: &mut Self| a.open_path_tree());
            on!(Modifiers::CTRL, Key::Backslash, |a: &mut Self| {
                let at = a.active_ref();
                let root = a.pane(at).map(|p| p.root_path()).unwrap_or_else(|| "/".into());
                a.goto(at, root);
            });
            on!(Modifiers::CTRL, Key::H, |a: &mut Self| a.goto(a.active_ref(), "~".into()));
            on!(Modifiers::CTRL, Key::F7, |a: &mut Self| a.open_search_dialog());
            on!(Modifiers::ALT, Key::F7, |a: &mut Self| a.open_search_dialog());
            on!(Modifiers::CTRL, Key::F, |a: &mut Self| a.open_search_dialog());
            on!(Modifiers::CTRL, Key::A, |a: &mut Self| with_pane(a, |p| p
                .mark_all()));
            on!(Modifiers::CTRL, Key::G, |a: &mut Self| with_pane(a, |p| {
                p.path_edit = Some(p.path.clone())
            }));
            on!(Modifiers::CTRL, Key::ArrowLeft, |a: &mut Self| a
                .open_in_other(0));
            on!(Modifiers::CTRL, Key::ArrowRight, |a: &mut Self| a
                .open_in_other(1));

            on!(Modifiers::NONE, Key::F2, |a: &mut Self| a.start_rename_active());
            // Shift+F6 before F6 (egui ignores an extra Shift when matching).
            on!(Modifiers::SHIFT, Key::F6, |a: &mut Self| a.start_rename_active());
            on!(Modifiers::NONE, Key::F6, |a: &mut Self| a.ask_move());
            on!(Modifiers::NONE, Key::F3, |a: &mut Self| a.view_active());
            // Shift+F4 before F4 (egui ignores an extra Shift when matching).
            on!(Modifiers::SHIFT, Key::F4, |a: &mut Self| a.ask_new_file());
            on!(Modifiers::NONE, Key::F4, |a: &mut Self| a.start_edit_active());
            on!(Modifiers::NONE, Key::F5, |a: &mut Self| a.ask_copy());
            on!(Modifiers::NONE, Key::F7, |a: &mut Self| a.ask_mkdir());
            for k in [Key::F8, Key::Delete] {
                on!(Modifiers::NONE, k, |a: &mut Self| a.ask_delete());
            }
            on!(Modifiers::NONE, Key::Tab, |a: &mut Self| a.switch_pane());
            on!(Modifiers::NONE, Key::ArrowUp, |a: &mut Self| with_pane(a, |p| p.move_vert(-1)));
            on!(Modifiers::NONE, Key::ArrowDown, |a: &mut Self| with_pane(a, |p| p.move_vert(1)));
            // ← / → move between columns (brief) or tiles (thumbnails).
            on!(Modifiers::NONE, Key::ArrowLeft, |a: &mut Self| with_pane(a, |p| {
                p.move_horiz(-1);
            }));
            on!(Modifiers::NONE, Key::ArrowRight, |a: &mut Self| with_pane(a, |p| {
                p.move_horiz(1);
            }));
            on!(Modifiers::NONE, Key::PageUp, |a: &mut Self| with_pane(
                a,
                |p| p.move_cursor(-(p.page_rows() as isize))
            ));
            on!(Modifiers::NONE, Key::PageDown, |a: &mut Self| with_pane(
                a,
                |p| p.move_cursor(p.page_rows() as isize)
            ));
            on!(Modifiers::NONE, Key::Home, |a: &mut Self| with_pane(
                a,
                |p| p.cursor_home()
            ));
            on!(Modifiers::NONE, Key::End, |a: &mut Self| with_pane(
                a,
                |p| p.cursor_end()
            ));
            on!(Modifiers::NONE, Key::Insert, |a: &mut Self| with_pane(
                a,
                |p| p.toggle_mark_current()
            ));
            // Before Space (egui ignores an extra Alt / Shift): size of the folder under
            // the cursor or of the marked ones. Shift+Space too: Cinnamon takes Alt+Space.
            on!(Modifiers::ALT, Key::Space, |a: &mut Self| a.size_selected_dirs());
            on!(Modifiers::SHIFT, Key::Space, |a: &mut Self| a.size_selected_dirs());
            on!(Modifiers::NONE, Key::Space, |a: &mut Self| with_pane(
                a,
                |p| p.toggle_mark_current()
            ));
            on!(Modifiers::NONE, Key::Enter, |a: &mut Self| {
                let at = a.active_ref();
                a.activate_entry(at);
            });
            on!(Modifiers::NONE, Key::Backspace, |a: &mut Self| {
                let at = a.active_ref();
                let filtering = a.pane(at).map(|p| !p.filter.is_empty()).unwrap_or(false);
                if filtering {
                    with_pane(a, |p| {
                        p.filter.pop();
                        p.rebuild_view();
                    });
                } else {
                    a.go_up(at);
                }
            });
            on!(Modifiers::NONE, Key::Escape, |a: &mut Self| {
                let at = a.active_ref();
                let Some(p) = a.pane_mut(at) else { return };
                if !p.filter.is_empty() {
                    p.filter.clear();
                    p.rebuild_view();
                } else if !p.dir_pending.is_empty() {
                    // Stop the folder size computation (a server `du` is killed).
                    p.size_cancel.store(true, Ordering::Relaxed);
                } else if !p.marked.is_empty() {
                    p.marked.clear();
                } else if let Some(sv) = &p.search {
                    if sv.running {
                        sv.cancel.store(true, Ordering::Relaxed);
                    } else {
                        a.leave_search(at);
                    }
                }
            });

            // Typing filters the panel (quick filter).
            let mut typed = String::new();
            i.events.retain(|e| match e {
                egui::Event::Text(t) => {
                    typed.extend(t.chars().filter(|c| !c.is_control() && *c != ' '));
                    false
                }
                _ => true,
            });
            if !typed.is_empty() && !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt {
                actions.push(Box::new(move |a: &mut Self| {
                    with_pane(a, |p| {
                        p.filter.push_str(&typed);
                        p.rebuild_view();
                    })
                }));
            }
        });
        for a in actions {
            a(self);
        }
    }

    fn open_switcher(&mut self) {
        self.dialog = Dialog::Switcher {
            query: String::new(),
            cursor: 0,
            focus: true,
        };
    }

    fn refresh_or_rerun(&mut self) {
        let at = self.active_ref();
        if let Some(p) = self.pane_mut(at) {
            p.clear_dir_sizes();
        }
        if self.pane(at).is_some_and(|p| p.search.is_some()) {
            self.rerun_search(at);
        } else {
            self.refresh(at);
        }
    }

    fn start_rename_tab(&mut self) {
        let t = self.sides[self.active_side].current();
        self.renaming = Some((t.id, t.title(), true));
    }

    fn switch_pane(&mut self) {
        let side = 1 - self.active_side;
        self.activate_tab(side, self.sides[side].active);
    }

    fn ask_mkdir(&mut self) {
        if self.pane(self.active_ref()).is_some_and(|p| p.search.is_some()) {
            self.last_error = Some(tr!("Cannot create a folder in search results."));
            return;
        }
        if self.refuse_read_only(self.active_ref(), &tr!("Cannot create a folder in an archive")) {
            return;
        }
        self.dialog = Dialog::Mkdir {
            at: self.active_ref(),
            name: String::new(),
            focus: true,
        }
    }

    fn ask_delete(&mut self) {
        let at = self.active_ref();
        if self.refuse_read_only(at, &tr!("Cannot delete from an archive")) {
            return;
        }
        let items = self.pane(at).map(|p| p.target_items()).unwrap_or_default();
        let production = self
            .pane(at)
            .and_then(|p| p.remote.as_ref())
            .is_some_and(|r| r.production);
        if !items.is_empty() {
            self.dialog = Dialog::Delete {
                at,
                items,
                production,
            };
        }
    }

    fn cycle_tabs(&mut self, dir: isize) {
        let side = self.active_side;
        if self.sides[side].tabs.len() < 2 {
            return;
        }
        let (snap, idx) = self
            .switching
            .take()
            .unwrap_or_else(|| (self.sides[side].mru.clone(), 0));
        let len = snap.len() as isize;
        let idx = ((idx as isize + dir).rem_euclid(len)) as usize;
        if let Some(t) = self.sides[side].index(snap[idx]) {
            self.switching = Some((snap, idx));
            self.activate_tab(side, t);
        }
    }

    /// Ctrl+←/→: open the directory under the cursor in the other panel.
    fn open_in_other(&mut self, target_side: usize) {
        let at = self.active_ref();
        if target_side == at.side {
            return;
        }
        let target = self.other(at);
        let Some(p) = self.pane(at) else { return };
        if p.archive.is_some() {
            // Inside an archive: the other panel opens the folder containing it.
            let (fs, path, remote, offline) = p.real_location();
            if offline {
                return;
            }
            if let Some(t) = self.pane_mut(target) {
                *t = Pane::new(fs, path.clone());
                t.remote = remote;
            }
            self.load(target, path, None);
            return;
        }
        let path = match (p.current_entry(), p.current_name().and_then(|n| p.hit(&n))) {
            (_, Some(h)) if h.entry.is_dir_like() => p.fs.join(&h.dir, &h.entry.name),
            (_, Some(h)) => h.dir.clone(),
            (Some(e), None) if e.is_dir_like() => p.fs.join(&p.path, &e.name),
            _ => p.path.clone(),
        };
        let same_fs = self
            .pane(target)
            .map(|t| Arc::ptr_eq(&t.fs, &p.fs))
            .unwrap_or(false);
        let fs = p.fs.clone();
        let (remote, offline) = (p.remote.clone(), p.offline);
        if target == at || offline {
            return;
        }
        if let Some(t) = self.pane_mut(target) {
            t.exit_search();
        }
        if !same_fs {
            if let Some(t) = self.pane_mut(target) {
                *t = Pane::new(fs, path.clone());
                t.remote = remote;
            }
        }
        self.load(target, path, None);
    }

    // ------------------------------------------------------------------- UI

    /// Tab row of one panel.
    fn tab_bar(&mut self, ui: &mut egui::Ui, side: usize) {
        let mut activate = None;
        let mut close = None;
        let mut new_tab = false;
        let mut menu_act: Option<Action> = None;
        let side_active = self.active_side == side;
        let tips: Vec<String> = self.sides[side].tabs.iter().map(|t| self.tab_tooltip(side, t)).collect();
        let lost: Vec<TabId> = self.sides[side]
            .tabs
            .iter()
            .filter(|t| self.pane_disconnected(PaneRef { tab: t.id, side }))
            .map(|t| t.id)
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
            let active_idx = self.sides[side].active;
            for (idx, tab) in self.sides[side].tabs.iter_mut().enumerate() {
                let is_active = idx == active_idx;
                let bg = rgb32(tab.color);
                let fg = text_on(bg);

                if let Some((id, buf, focus)) = &mut self.renaming {
                    if *id == tab.id {
                        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(140.0));
                        if *focus {
                            r.request_focus();
                            *focus = false;
                        }
                        if r.lost_focus() {
                            if ui.input(|i| i.key_pressed(Key::Enter)) {
                                let n = buf.trim().to_string();
                                tab.name = (!n.is_empty()).then_some(n);
                            }
                            self.renaming = None;
                        }
                        continue;
                    }
                }

                let num = if idx < 9 && side_active { format!("{} ", idx + 1) } else { String::new() };
                let remote = tab.pane.remote.is_some();
                let production = tab.pane.remote.as_ref().is_some_and(|r| r.production);
                let disconnected = lost.contains(&tab.id);
                let text = egui::RichText::new(format!(
                    "{num}{}{}{}",
                    if production { "⚠ " } else { "" },
                    if disconnected { "⚡ " } else if remote { "🖧 " } else { "" },
                    tab.title()
                ))
                    .color(if disconnected { fg.gamma_multiply(0.5) } else { fg })
                    .size(crate::fonts::fs(13.5));
                let text = if disconnected { text.italics() } else { text };
                let text = if is_active { text.strong() } else { text };
                let btn = egui::Button::new(text)
                    .fill(bg)
                    .stroke(if is_active {
                        Stroke::new(2.0, fg)
                    } else {
                        Stroke::new(1.0, fg.gamma_multiply(0.35))
                    })
                    .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                    .min_size(Vec2::new(60.0, 26.0));
                let resp = ui.add(btn);
                self.drop.tabs.push((PaneRef { tab: tab.id, side }, resp.rect));
                if resp.clicked() {
                    activate = Some(idx);
                }
                if resp.double_clicked() {
                    self.renaming = Some((tab.id, tab.title(), true));
                }
                if resp.middle_clicked() {
                    close = Some(idx);
                }
                egui::Popup::context_menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        if let Some(a) = App::tab_menu(ui, side, idx, tab, &mut self.renaming) {
                            menu_act = Some(a);
                        }
                    });
                resp.on_hover_text(&tips[idx]);
            }
            if ui.add(egui::Button::new(egui::RichText::new("＋").size(crate::fonts::fs(15.0))).min_size(Vec2::new(26.0, 26.0)))
                .on_hover_text(tr!("New tab: home folder, favorites, servers, recent places (Ctrl+T)"))
                .clicked()
            {
                new_tab = true;
            }
        });
        if let Some(i) = activate {
            self.activate_tab(side, i);
        }
        if let Some(i) = close {
            self.close_tab(side, i);
        }
        if new_tab {
            self.open_new_tab_menu(side);
        }
        if let Some(a) = menu_act {
            a(self);
        }
    }

    /// Both panels, each with its own tab row and framed in its active tab's color.
    fn panels(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let gap = 6.0;
        let w = (full.width() - gap) / 2.0;
        let base = ui.visuals().extreme_bg_color;
        let favorites = self.favorite_labels();
        let mut events = Vec::new();
        let mut link_actions: Vec<Action> = Vec::new();
        self.drop.zones.clear();
        self.drop.tabs.clear();
        for side in 0..2 {
            let x = full.left() + side as f32 * (w + gap);
            let rect = egui::Rect::from_min_size(egui::pos2(x, full.top()), Vec2::new(w, full.height()));
            let tab = self.sides[side].current();
            let tab_id = tab.id;
            let color = rgb32(tab.color);
            let list_bg = blend(base, color, LIST_TINT);
            let active = self.active_side == side;
            ui.painter().rect_filled(rect, 4.0, color);
            if active {
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    Stroke::new(2.0, text_on(color).gamma_multiply(0.7)),
                    StrokeKind::Inside,
                );
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink(4.0))
                    .id_salt(("side", side)),
            );
            self.tab_bar(&mut child, side);
            child.add_space(2.0);
            let pane_rect = child.available_rect_before_wrap();
            let mut pane_ui = child.new_child(
                egui::UiBuilder::new()
                    .max_rect(pane_rect)
                    .id_salt(("pane", tab_id)),
            );
            pane_ui
                .painter()
                .rect_filled(pane_rect, 3.0, ui.visuals().panel_fill);
            let at = PaneRef { tab: tab_id, side };
            if let Some(a) = self.link_bar(&mut pane_ui, at) {
                link_actions.push(a);
            }
            if self.pane_disconnected(at) {
                // Lost connection: the listing stays visible, grayed out.
                pane_ui.multiply_opacity(0.45);
            }
            let idx = self.sides[side].active;
            let style = self.sides[side].tabs[idx].pane.style;
            let widths = self.settings.columns.get(&columns_key(side, style)).copied().unwrap_or_default();
            let mut view = crate::pane::ViewCtx {
                widths,
                thumbs: &mut self.thumbs,
                thumb_px: self.settings.thumb_px(),
                remote_thumbs: self.settings.thumbs_on_servers(),
                remote_max: self.settings.thumbs_remote_max(),
                production_thumbs: self.settings.thumbs_production,
                kinds: crate::pane::kinds::KindColors {
                    on: !self.settings.type_colors_off,
                    custom: self.settings.type_colors.clone(),
                },
                ext_column: self.settings.ext_column,
                seconds: self.settings.seconds,
                brackets: !self.settings.dir_brackets_off,
                favorites: &favorites,
                nav_bar: !self.settings.hide_nav_bar,
            };
            let ev = self.sides[side].tabs[idx]
                .pane
                .show(&mut pane_ui, active, color, list_bg, &mut view);
            events.extend(ev.into_iter().map(|e| (PaneRef { tab: tab_id, side }, e)));
            self.drop_zone(at, rect);
        }
        ui.allocate_rect(full, egui::Sense::hover());
        for a in link_actions {
            a(self);
        }
        for (at, ev) in events {
            match ev {
                PaneEvent::Focus => {
                    if self.active_side != at.side {
                        self.active_side = at.side;
                        self.refresh_if_stale(at);
                    }
                }
                PaneEvent::Activate => {
                    self.active_side = at.side;
                    self.activate_entry(at);
                }
                PaneEvent::GoTo(p) => self.goto(at, p),
                PaneEvent::Up => self.go_up(at),
                PaneEvent::Command(cmd) => {
                    self.active_side = at.side;
                    self.pane_command(at, cmd);
                }
                PaneEvent::Rename { dir, from, to } => self.on_rename_typed(at, dir, from, to),
                PaneEvent::Favorite(n) => {
                    self.active_side = at.side;
                    self.go_favorite(at, n);
                }
                PaneEvent::ToggleFavorite => self.toggle_favorite(at),
                PaneEvent::ColumnWidth { col, width, done } => self.set_column_width(at, col, width, done),
                PaneEvent::OrganizeFavorites => self.open_favorites(),
                PaneEvent::DragStart(name) => self.start_inner_drag(at, &name),
                PaneEvent::DiskInfo => {
                    self.active_side = at.side;
                    self.open_disk_detail(at);
                }
                PaneEvent::History(forward) => {
                    self.active_side = at.side;
                    self.history_go(forward);
                }
                PaneEvent::Refresh => {
                    self.active_side = at.side;
                    self.refresh_or_rerun();
                }
                PaneEvent::LeaveArchive => {
                    while self.pane(at).is_some_and(|p| p.archive.is_some()) {
                        self.leave_archive(at);
                    }
                }
                PaneEvent::WantMounts => self.read_mounts(at),
                PaneEvent::Jump { path, select } => {
                    self.active_side = at.side;
                    self.load(at, path, select);
                }
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        if !self.edits.sessions.is_empty() {
            let stroke = Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color.gamma_multiply(0.5));
            let mut open = false;
            ui.horizontal(|ui| {
                open = self.edit_indicator(ui, stroke);
                ui.weak(tr!("F4 = edit another file · click = list of edits"));
            });
            if open {
                self.edits.show_list = true;
            }
        }
        if !self.jobs.is_empty() {
            let mut speed = self.settings.speed_limit_kib;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tr!("Transfers: {count}", count = self.jobs.len()));
                ui.separator();
                ui.label(tr!("Max. speed:"));
                changed = speed_limit_combo(ui, "status-speed", &mut speed);
            });
            if changed {
                self.set_speed_limit(speed);
            }
        }
        let ids: HashSet<u64> = self.jobs.iter().map(|j| j.id).collect();
        self.eta.retain(|id, _| ids.contains(id));
        if !self.jobs.is_empty() {
            // One sample a second even when nothing else repaints.
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        for job in &self.jobs {
            let p = job.progress.lock().unwrap().clone();
            let tot = *job.totals.lock().unwrap();
            // Known total: counted totals, else the size given to the job.
            let total = if tot.done && tot.bytes > 0 {
                Some(tot.bytes)
            } else {
                (job.total_bytes > 0).then_some(job.total_bytes)
            };
            let win = self.eta.entry(job.id).or_default();
            // Done / total for the speed and remaining time: the archive read
            // (extracting, testing), else the bytes processed.
            let (eta_done, eta_total) = match p.read {
                Some((d, t)) if t > 0 => (d, Some(t)),
                _ => (p.bytes_done, total.filter(|_| job.count.is_none()).map(|t| t.saturating_sub(p.bytes_resumed))),
            };
            if let Some(start) = p.started {
                win.sample(start.elapsed(), eta_done);
            }
            let speed = win.speed();
            let eta_text = eta_total.map(|t| rolid_core::eta::format_eta(win.eta(t.saturating_sub(eta_done))));
            ui.horizontal(|ui| {
                ui.spinner();
                let frac = if job.total_bytes > 0 {
                    (p.bytes_done as f32 / job.total_bytes as f32).min(1.0)
                } else {
                    0.0
                };
                let note = job.note.lock().unwrap().clone();
                let frac = match job.count {
                    Some(n) if n > 0 => (p.files_done as f32 / n as f32).min(1.0),
                    _ => frac,
                };
                let text = if let Some(n) = note {
                    n
                } else if let Some(n) = job.count {
                    trn!("{done} of {n} item", "{done} of {n} items", n, done = p.files_done)
                } else if p.started.is_none() {
                    tr!("⏳ waiting in the queue")
                } else {
                    let done = match p.read {
                        Some((d, t)) if t > 0 => tr!("archive read {done} of {total}", done = human_size(d), total = human_size(t)),
                        _ if tot.done && tot.bytes > 0 => {
                            tr!("{done} of {total}", done = human_size(p.bytes_done + p.bytes_resumed), total = human_size(tot.bytes))
                        }
                        _ => human_size(p.bytes_done + p.bytes_resumed),
                    };
                    let rate = match speed {
                        Some(s) => tr!("{size}/s", size = human_size(s as u64)),
                        None => tr!("speed: measuring"),
                    };
                    let mut t = format!("{done} · {rate} · {}", rolid_i18n::t(&tot.file_text(&p)));
                    if let Some(e) = &eta_text {
                        t += &format!(" · {}", rolid_i18n::t(e));
                    }
                    if p.files_skipped > 0 {
                        t += &format!(" · {}", trn!("{n} skipped", "{n} skipped", p.files_skipped));
                    }
                    t
                };
                let frac = if let Some((d, t)) = p.read.filter(|r| r.1 > 0) {
                    (d as f32 / t as f32).min(1.0)
                } else if tot.done && tot.bytes > 0 && job.count.is_none() {
                    ((p.bytes_done + p.bytes_resumed) as f32 / tot.bytes as f32).min(1.0)
                } else if job.total_bytes > 0 && job.count.is_none() {
                    ((p.bytes_done + p.bytes_resumed) as f32 / job.total_bytes as f32).min(1.0)
                } else {
                    frac
                };
                ui.add(egui::ProgressBar::new(frac).desired_width(480.0).text(text));
                // The button first: a long title or path must not push it (and
                // the version button below) out of the window.
                if ui.small_button(tr!("Cancel")).clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                ui.add(egui::Label::new(&job.title).truncate());
                ui.add(egui::Label::new(egui::RichText::new(&p.current).weak()).truncate());
            });
        }
        self.reports_ui(ui);
        let mut failed_act: Option<(u64, bool)> = None; // (id, retry)
        for f in &self.failed_jobs {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(190, 20, 20), tr!("⚠ Transfer {name} failed:", name = f.title.as_str()));
                ui.add(egui::Label::new(egui::RichText::new(&f.error).color(Color32::from_rgb(190, 20, 20))).truncate());
                if ui.small_button(tr!("Try again")).on_hover_text(tr!("Continues where the transfer stopped")).clicked() {
                    failed_act = Some((f.id, true));
                }
                if ui.small_button("✖").clicked() {
                    failed_act = Some((f.id, false));
                }
            });
        }
        match failed_act {
            Some((id, true)) => self.retry_failed_job(id),
            Some((id, false)) => self.failed_jobs.retain(|f| f.id != id),
            None => {}
        }
        if let Some((n, since)) = &self.notice {
            let left = Duration::from_secs(6).saturating_sub(since.elapsed());
            if left.is_zero() {
                self.notice = None;
            } else {
                ui.label(egui::RichText::new(format!("✔ {n}")).color(ui.visuals().strong_text_color()));
                ui.ctx().request_repaint_after(left);
            }
        }
        if let Some(e) = &self.last_error {
            let mut clear = false;
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(190, 20, 20), format!("⚠ {e}"));
                clear = ui.small_button("✖").clicked();
            });
            if clear {
                self.last_error = None;
            }
        }
        // Function-key bar like in Total Commander: every item is a real button.
        let mut act: Option<Action> = None;
        ui.add_space(2.0);
        type Btn = (&'static str, String, fn(&mut App));
        let buttons: [Btn; 13] = [
            ("F1", trc!("key bar", "Help"), |a| a.dialog = Dialog::Help(false)),
            ("F2", trc!("key bar", "Rename"), |a| a.start_rename_active()),
            ("F3", trc!("key bar", "View"), |a| a.view_active()),
            ("F4", trc!("key bar", "Edit"), |a| a.start_edit_active()),
            ("F5", trc!("key bar", "Copy"), |a| a.ask_copy()),
            ("F6", trc!("key bar", "Move"), |a| a.ask_move()),
            ("F7", trc!("key bar", "New folder"), |a| a.ask_mkdir()),
            ("F8", trc!("key bar", "Delete"), |a| a.ask_delete()),
            ("Ctrl+F7", trc!("key bar", "Search"), |a| a.open_search_dialog()),
            ("Ctrl+O", trc!("key bar", "Servers"), |a| a.open_connect()),
            ("Ctrl+L", trc!("key bar", "Local"), |a| a.set_local(a.active_ref())),
            ("Ctrl+T", trc!("key bar", "Tab"), |a| a.open_new_tab_menu(a.active_side)),
            ("Ctrl+R", trc!("key bar", "Refresh"), |a| a.refresh_or_rerun()),
        ];
        let stroke = Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color.gamma_multiply(0.5));
        let font = crate::fonts::prop(12.5);
        let height = crate::fonts::fs(24.0).max(24.0);
        let gap = 4.0;
        let pad = ui.spacing().button_padding.x * 2.0;
        let text_w = |t: &str| ui.painter().layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x;
        // Right side: version and saved passwords – always visible, measured first.
        let version = format!("ⓘ RolidSCP {}", menu::VERSION); // i18n-ok: info symbol and product name
        let lock_left = self.vault_lock_left();
        let vault = self.vault.exists().then(|| {
            if self.vault.open.is_some() {
                match lock_left {
                    Some(left) => {
                        let time = vault_ui::countdown(left);
                        (
                            true,
                            tr!("🔓 Passwords unlocked · {time}", time = time.as_str()),
                            tr!("Saved passwords are unlocked; they lock in {time} unless used again. Click to lock them now.", time = time.as_str()),
                        )
                    }
                    None => (true, tr!("🔓 Passwords unlocked"), tr!("Saved passwords are unlocked. Click to lock them.")),
                }
            } else {
                (false, tr!("🔒 Passwords locked"), tr!("Saved passwords are locked. Click to unlock them."))
            }
        });
        let right = text_w(&version) + pad + vault.as_ref().map(|(_, t, _)| text_w(t) + pad + gap).unwrap_or(0.0) + 2.0;
        let full: Vec<f32> = buttons.iter().map(|(k, t, _)| text_w(k) + text_w(&format!(" {t}")) + pad).collect();
        let keys: Vec<f32> = buttons.iter().map(|(k, _, _)| text_w(k) + pad).collect();
        let avail = ui.available_width();
        let mode = fbar::bar_mode(avail, right, &full, &keys, gap);
        let widest = keys.iter().cloned().fold(0.0, f32::max);
        let room = fbar::keys_room(avail, right, gap, widest);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(mode == fbar::BarMode::Wrap);
            ui.allocate_ui_with_layout(Vec2::new(room, height), layout, |ui| {
                ui.set_max_width(room);
                for (k, t, f) in buttons {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(k, 0.0, egui::TextFormat {
                        font_id: font.clone(),
                        color: ui.visuals().strong_text_color(),
                        ..Default::default()
                    });
                    if mode == fbar::BarMode::Full {
                        job.append(&format!(" {t}"), 0.0, egui::TextFormat {
                            font_id: font.clone(),
                            color: ui.visuals().text_color(),
                            ..Default::default()
                        });
                    }
                    let b = egui::Button::new(job).stroke(stroke).min_size(Vec2::new(0.0, height));
                    let mut r = ui.add(b);
                    if mode != fbar::BarMode::Full {
                        r = r.on_hover_text(format!("{k} {t}"));
                    }
                    if r.clicked() {
                        act = Some(Box::new(f));
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let v = egui::Button::new(egui::RichText::new(&version).font(font.clone()))
                    .stroke(stroke)
                    .min_size(Vec2::new(0.0, height));
                if ui.add(v).on_hover_text(tr!("About and supporting the project")).clicked() {
                    act = Some(Box::new(|a: &mut App| a.dialog = Dialog::About));
                }
                // Saved passwords: locked / unlocked (only once a master password exists).
                if let Some((open, text, tip)) = vault {
                    let b = egui::Button::new(egui::RichText::new(text).font(font.clone()))
                        .stroke(stroke)
                        .min_size(Vec2::new(0.0, height));
                    if ui.add(b).on_hover_text(tip).clicked() {
                        act = Some(if open {
                            Box::new(|a: &mut App| a.lock_vault("manually"))
                        } else {
                            Box::new(|a: &mut App| a.ask_unlock())
                        });
                    }
                }
            });
        });
        ui.add_space(2.0);
        if let Some(a) = act {
            a(self);
        }
    }

    fn switch_to(&mut self, to: SwitchTo) {
        match to {
            SwitchTo::Tab(side, i) => self.activate_tab(side, i),
            SwitchTo::Profile(id) => self.open_profile(&id),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // A login question from the server is on top of everything else.
        if !self.auth_prompts.is_empty() {
            self.auth_prompt_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Connect(_)) {
            self.site_manager_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Switcher { .. }) {
            self.switcher_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::NewTab { .. }) {
            self.new_tab_menu_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Favorites(_)) {
            self.favorites_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Search(_)) {
            self.search_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Drop(_) | Dialog::DropWhere { .. }) {
            self.drop_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Perms(_) | Dialog::PermsConfirm(_) | Dialog::OpenWith(_)) {
            self.ctx_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::MarkGroup(_) | Dialog::Settings(_)) {
            self.view_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Archive(_)) {
            self.archive_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Checksum(_)) {
            self.checksum_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Disk(_)) {
            self.disk_dialog_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::TermConfirm(_)) {
            self.term_confirm_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::NewFile(_)) {
            self.new_file_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::DragOutLarge(_)) {
            self.drag_out_large_ui(ctx);
            return;
        }
        if matches!(self.dialog, Dialog::Rename(_) | Dialog::Move(_)) {
            self.rename_dialog_ui(ctx);
            return;
        }
        let mut next: Option<Dialog> = None;
        let mut act: Option<Action> = None;
        let mut speed = self.settings.speed_limit_kib;
        let mut speed_changed = false;
        let mut dest_changed = false;

        match &mut self.dialog {
            Dialog::None
            | Dialog::Drop(_)
            | Dialog::DropWhere { .. }
            | Dialog::Perms(_)
            | Dialog::PermsConfirm(_)
            | Dialog::OpenWith(_)
            | Dialog::MarkGroup(_)
            | Dialog::Settings(_)
            | Dialog::Archive(_)
            | Dialog::Rename(_)
            | Dialog::Move(_)
            | Dialog::Checksum(_)
            | Dialog::Disk(_)
            | Dialog::NewFile(_)
            | Dialog::DragOutLarge(_)
            | Dialog::TermConfirm(_) => {}
            Dialog::Copy {
                from,
                to,
                dest,
                items,
                overwrite,
                conflict,
            } => {
                let dst = find_pane(&self.sides, *to);
                let disk = dst.map(|d| d.display_label()).unwrap_or_default();
                let to_production = dst
                    .and_then(|d| d.remote.as_ref())
                    .is_some_and(|r| r.production);
                let remote_involved = dst.is_some_and(|d| d.fs.is_remote())
                    || find_pane(&self.sides, *from).is_some_and(|p| p.fs.is_remote());
                let history = self.places.targets_on(&dst.map(|d| d.location()).unwrap_or(rolid_config::Location::Local { path: String::new() }));
                dialog::window("copy", trc!("verb", "Copy")).icon(Role::Copy).min_width(560.0).danger(to_production).show(ctx, |ui| {
                    let lead = if items.len() == 1 {
                        tr!("Copy “{name}” to {disk}:", name = items[0].label.as_str(), disk = disk.as_str())
                    } else {
                        trn!("Copy {n} item to {disk}:", "Copy {n} items to {disk}:", items.len(), disk = disk.as_str())
                    };
                    dialog::lead(ui, lead);
                    ui.horizontal(|ui| {
                        let r = dialog::history_field(ui, "copy-dest", dest, &history, dialog::width() - 140.0);
                        if r.changed() {
                            dest_changed = true;
                        }
                        if crate::look::btn(Role::Folder, tr!("Browse…")).ui(ui).clicked() {
                            let (t, d) = (*to, dest.clone());
                            act = Some(Box::new(move |a: &mut Self| a.open_folder_pick(t, d)));
                        }
                    });
                    if to_production {
                        dialog::warn(ui, tr!("⚠ The target is a PRODUCTION server."), true);
                    }
                    let heading = tr!("Existing files");
                    dialog::group(ui, &heading, |ui| {
                        if *overwrite > 0 {
                            dialog::warn(
                                ui,
                                trn!(
                                    "⚠ {n} item already exists in the target. What to do with existing files?",
                                    "⚠ {n} items already exist in the target. What to do with existing files?",
                                    *overwrite
                                ),
                                false,
                            );
                        }
                        conflict_choice(ui, conflict);
                    });
                    let heading = tr!("Transfer");
                    dialog::group(ui, &heading, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr!("Max. speed (all transfers together):"));
                            if speed_limit_combo(ui, "copy-speed", &mut speed) {
                                speed_changed = true;
                            }
                            let mut h = tr!("Files are written through a temporary .filepart that is renamed when finished.");
                            if remote_involved {
                                h = format!("{h}\n\n{}", tr!("Tip: a speed limit keeps a slow line from being flooded (e.g. the server's outgoing line)."));
                            }
                            dialog::help(ui, h);
                        });
                    });
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Copy, trc!("verb", "Copy")).default(true).enabled(conflict.is_some() && !dest.trim().is_empty()),
                        dialog::cancel(),
                    ]);
                    if let (true, Some(c)) = (b[0], *conflict) {
                        let (f, t, d, n) = (*from, *to, dest.trim().to_string(), std::mem::take(items));
                        act = Some(Box::new(move |a: &mut Self| a.start_copy(f, t, d, n, c)));
                        next = Some(Dialog::None);
                    }
                    if b[1] {
                        next = Some(Dialog::None);
                    }
                });
            }
            Dialog::Mkdir { at, name, focus } => {
                dialog::window("mkdir", tr!("New folder")).icon(Role::NewFolder).show(ctx, |ui| {
                    dialog::form(ui, "mkdir-form", |ui| {
                        ui.label(tr!("Name:"));
                        let r = ui.add(egui::TextEdit::singleline(name).desired_width(dialog::width() - 80.0));
                        if *focus {
                            r.request_focus();
                            *focus = false;
                        }
                        ui.end_row();
                    });
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::NewFolder, tr!("Create")).default(true).enabled(!name.trim().is_empty()),
                        dialog::cancel(),
                    ]);
                    if b[0] {
                        let (at, n) = (*at, std::mem::take(name));
                        act = Some(Box::new(move |a: &mut Self| a.start_mkdir(at, n)));
                        next = Some(Dialog::None);
                    }
                    if b[1] {
                        next = Some(Dialog::None);
                    }
                });
            }
            Dialog::Delete {
                at,
                items,
                production,
            } => {
                let remote = find_pane(&self.sides, *at).is_some_and(|p| p.fs.is_remote());
                dialog::window("delete", trc!("verb", "Delete")).icon(Role::Delete).danger(remote || *production).show(ctx, |ui| {
                    let question = match (items.len(), remote) {
                        (1, false) => tr!("Permanently delete “{name}”? Folders are deleted with their contents.", name = items[0].label.as_str()),
                        (1, true) => tr!("Permanently delete “{name}” ON THE SERVER? Folders are deleted with their contents.", name = items[0].label.as_str()),
                        (n, false) => trn!(
                            "Permanently delete {n} item? Folders are deleted with their contents.",
                            "Permanently delete {n} items? Folders are deleted with their contents.",
                            n
                        ),
                        (n, true) => trn!(
                            "Permanently delete {n} item ON THE SERVER? Folders are deleted with their contents.",
                            "Permanently delete {n} items ON THE SERVER? Folders are deleted with their contents.",
                            n
                        ),
                    };
                    dialog::lead(ui, question);
                    if *production {
                        dialog::warn(ui, tr!("⚠ PRODUCTION SERVER: Enter is disabled, confirm with the button."), true);
                    }
                    let label = if *production { tr!("Delete on production") } else { trc!("verb", "Delete") };
                    let b = dialog::buttons(ui, [dialog::btn(Role::Delete, label).default(!*production), dialog::cancel()]);
                    if b[0] {
                        let (at, n) = (*at, std::mem::take(items));
                        act = Some(Box::new(move |a: &mut Self| a.start_delete(at, n)));
                        next = Some(Dialog::None);
                    }
                    if b[1] {
                        next = Some(Dialog::None);
                    }
                });
            }
            Dialog::Help(keys) => {
                if menu::help_ui(ctx, keys) {
                    next = Some(Dialog::None);
                }
            }
            Dialog::About => {
                if menu::about_ui(ctx, self.donate_btc, &self.font_desc) {
                    next = Some(Dialog::None);
                }
            }
            Dialog::Connect(_) | Dialog::Search(_) | Dialog::Switcher { .. } | Dialog::NewTab { .. } | Dialog::Favorites(_) => {}
        }
        if let Some(d) = next {
            self.dialog = d;
        }
        if speed_changed {
            self.set_speed_limit(speed);
        }
        if dest_changed {
            self.recount_copy();
        }
        if let Some(a) = act {
            a(self);
        }
    }
}

fn with_pane(a: &mut App, f: impl FnOnce(&mut Pane)) {
    let at = a.active_ref();
    if let Some(p) = a.pane_mut(at) {
        f(p);
    }
}


/// The answer a conflict preset preselects (`None` = the user has to choose).
fn preset_conflict(p: rolid_config::ConflictPreset) -> Option<Conflict> {
    use rolid_config::ConflictPreset as P;
    match p {
        P::Skip => Some(Conflict::Skip),
        P::Newer => Some(Conflict::Newer),
        P::Overwrite => Some(Conflict::Overwrite),
        P::Ask => None,
    }
}

/// The choice of what to do with existing files (F5 and drag and drop).
/// "Overwrite" is shown in a warning color while it is selected.
fn conflict_choice(ui: &mut egui::Ui, conflict: &mut Option<Conflict>) {
    for (c, label) in [
        (Conflict::Skip, tr!("Skip existing")),
        (Conflict::Newer, tr!("Overwrite only older (by modification date)")),
        (Conflict::Overwrite, tr!("Overwrite")),
        (Conflict::Resume, tr!("Resume an interrupted transfer (.filepart / shorter file)")),
    ] {
        let selected = *conflict == Some(c);
        let text = if selected && c == Conflict::Overwrite {
            egui::RichText::new(tr!("{choice}  ⚠ existing files will be overwritten", choice = label.as_str())).color(WARN_ORANGE).strong()
        } else {
            egui::RichText::new(label)
        };
        if ui.radio(selected, text).clicked() {
            *conflict = Some(c);
        }
    }
    if conflict.is_none() {
        ui.colored_label(WARN_ORANGE, tr!("Choose what to do with existing files: nothing is preselected."));
    }
}

/// Warning color (overwrite) – readable on light and dark backgrounds.
const WARN_ORANGE: Color32 = Color32::from_rgb(200, 90, 0);

fn remove_recursive<'a>(
    fs: &'a dyn Vfs,
    path: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = VfsResult<()>> + Send + 'a>> {
    Box::pin(async move {
        let e = fs.stat(path).await?;
        // Never recurse through a symlink: delete the link itself.
        if e.kind == rolid_core::EntryKind::Dir {
            for child in fs.list(path).await? {
                validate_name(&child.name)?;
                remove_recursive(fs, &fs.join(path, &child.name)).await?;
            }
            fs.remove_dir(path).await
        } else {
            fs.remove_file(path).await
        }
    })
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let frame_start = Instant::now();
        if !self.scale_logged {
            self.scale_logged = true;
            let native = ctx.input(|i| i.viewport().native_pixels_per_point);
            let tess = ctx.tessellation_options(|t| t.round_text_to_pixels);
            tracing::info!(
                "screen scale: pixels_per_point {} (system {}), zoom {}, text rounded to whole pixels: {}",
                ctx.pixels_per_point(),
                native.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into()),
                ctx.zoom_factor(),
                if tess { "yes" } else { "no" },
            );
        }
        crate::fonts::follow_screen_scale(&ctx);
        self.handle_msgs();
        self.links_frame(&ctx);
        self.drain_searches(&ctx);
        self.vault_frame_start(&ctx);
        self.edit_frame(&ctx);
        self.inner_drag_keys(&ctx);
        self.handle_keys(&ctx);
        self.x11_drag_frame(&ctx, frame);
        if self.x11_win.is_none() && x11_drag::is_x11() {
            self.x11_win = x11_drag::window_id(frame);
        }
        self.drag_out_frame(&ctx);
        self.disk_temps_frame(&ctx);
        self.drop_frame(&ctx);
        self.thumbs.frame();

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        if !self.settings.hide_toolbar {
            egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(2.0);
            self.status_bar(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| self.panels(ui));
        self.drop_hover_ui(&ctx);
        self.inner_drag_ui(&ctx);
        crate::dialog::block_keys(self.folder_pick.is_some() || !self.edits.prompts.is_empty());
        self.dialogs(&ctx);
        crate::dialog::block_keys(false);
        self.folder_pick_ui(&ctx);
        self.viewers_ui(&ctx);
        self.report_window(&ctx);
        self.edits_list_ui(&ctx);
        self.edit_prompt_ui(&ctx);
        // Master password dialogs are on top (they may be needed by the site manager).
        self.vault_dialog_ui(&ctx);
        self.auto_refresh_frame(&ctx);

        let title = crate::version::window_title(&self.sides[self.active_side].current().title());
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // The UI thread must never wait for the network: report slow frames.
        let took = frame_start.elapsed();
        if let Some(fs) = &mut self.frame_stats {
            fs.add(took);
        }
        if took > SLOW_FRAME {
            tracing::warn!("slow frame: {} ms", took.as_millis());
        }
        // Debounced autosave: egui only repaints on events, so schedule a wake-up
        // when the workspace changed and save at most once per second.
        if self.workspace_dirty() {
            if self.last_save_check.elapsed() >= Duration::from_secs(1) {
                self.last_save_check = Instant::now();
                self.save_workspace_if_changed();
            } else {
                ctx.request_repaint_after(Duration::from_millis(1100));
            }
        } else {
            self.last_save_check = Instant::now();
        }
    }

    fn on_exit(&mut self) {
        self.lock_vault("application exit");
        self.end_all_edits();
        self.drag_out_exit();
        self.save_workspace_if_changed();
    }
}

/// Human-readable speed limit.
pub(crate) fn fmt_speed(kib: Option<u64>) -> String {
    match kib {
        None => tr!("unlimited"),
        Some(k) if k >= 1024 && k % 1024 == 0 => format!("{} MiB/s", k / 1024),
        Some(k) if k >= 1024 => format!("{} MiB/s", rolid_i18n::decimal(k as f64 / 1024.0, 1)),
        Some(k) => format!("{k} KiB/s"),
    }
}

/// Speed limit presets in KiB/s.
pub(crate) const SPEED_PRESETS: [Option<u64>; 10] = [
    None,
    Some(128),
    Some(256),
    Some(512),
    Some(1024),
    Some(2 * 1024),
    Some(5 * 1024),
    Some(10 * 1024),
    Some(20 * 1024),
    Some(50 * 1024),
];

/// Combo box with speed presets plus an exact value; returns true when changed.
fn speed_limit_combo(ui: &mut egui::Ui, id: &str, kib: &mut Option<u64>) -> bool {
    let before = *kib;
    egui::ComboBox::from_id_salt(id)
        .selected_text(fmt_speed(*kib))
        .width(120.0)
        .show_ui(ui, |ui| {
            for p in SPEED_PRESETS {
                ui.selectable_value(kib, p, fmt_speed(p));
            }
        });
    if let Some(v) = kib {
        ui.add(
            egui::DragValue::new(v)
                .range(16..=10_000_000)
                .speed(16.0)
                .suffix(" KiB/s"),
        )
        .on_hover_text(tr!("Exact value (drag, or double-click and type)"));
    }
    *kib != before
}

/// File system type of `path` on a server (`df -PT` / `stat -f` over SSH);
/// `None` without a shell or when the command does not answer within 5 s.
async fn remote_fs_type(link: &rolid_ssh::Link, path: &str) -> Option<String> {
    let (_, conn) = link.current()?;
    let cmd = rolid_disk::remote::fs_type_command(&rolid_search::remote::sh_quote(path));
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut out = Vec::new();
    let mut take = |b: &[u8]| {
        out.extend_from_slice(b);
        out.len() < 16 * 1024
    };
    let cmd = rolid_search::remote::guarded(&cmd);
    let run = conn.exec_stream(&cmd, &cancel, &mut take);
    match tokio::time::timeout(Duration::from_secs(5), run).await {
        Ok(Ok(_)) => rolid_disk::remote::parse_fs_type(&String::from_utf8_lossy(&out)),
        _ => None,
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    use rolid_config::{ConflictPreset, Settings};

    #[test]
    fn preselected_conflict_follows_the_settings() {
        let s = Settings::default();
        assert_eq!(preset_conflict(s.drop_preset()), Some(Conflict::Skip), "Enter never overwrites");
        assert_eq!(preset_conflict(s.copy_preset()), Some(Conflict::Overwrite));
        assert_eq!(preset_conflict(ConflictPreset::Newer), Some(Conflict::Newer));
        assert_eq!(preset_conflict(ConflictPreset::Ask), None, "nothing preselected");
    }
}
