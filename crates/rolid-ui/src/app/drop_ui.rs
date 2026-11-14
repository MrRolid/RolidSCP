//! Dragging files and folders from the system file manager (Nemo, Nautilus,
//! Dolphin, Explorer…) into a panel. They are copied there – uploaded to a
//! server or copied locally – through the normal transfer queue (`.filepart`,
//! speed limit, resume after a lost connection). Sources are only read: never
//! moved, deleted or changed.
//!
//! Platform limits (winit 0.30, used by eframe):
//! - X11: while dragging, the cursor position is not passed on (XDND positions
//!   are dropped), so every panel that can take the files is highlighted; the
//!   exact target (panel, folder row) is decided at the drop, when the position
//!   arrives. If it does not arrive, the user picks the panel.
//! - Wayland: winit has no drag and drop support, nothing arrives.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use rolid_core::copy::{Conflict, Failure};
use rolid_core::vfs::validate_name;
use rolid_core::Vfs;
use rolid_local::LocalFs;
use rolid_i18n::rolid_text::txt;

use super::{App, CopySpec, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::Role;
use crate::pane::{human_size, Target};

/// Ask before starting when there are more files than this…
pub const CONFIRM_FILES: u64 = 200;
/// …or more bytes than this (1 GiB).
pub const CONFIRM_BYTES: u64 = 1 << 30;
/// After a drop, how long to wait for the cursor position (it comes with the
/// first pointer event after the drag ends).
const POS_WAIT: Duration = Duration::from_millis(400);
/// When the X server told us where the cursor is, wait only this long for the
/// exact pointer event.
const QUERIED_WAIT: Duration = Duration::from_millis(120);

/// Why files cannot be dropped into a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotConnected,
    Search,
    ReadOnly,
}

impl Refusal {
    /// For the user.
    pub fn text(self) -> String {
        match self {
            Refusal::NotConnected => tr!("The panel is not connected"),
            Refusal::Search => tr!("The panel shows search results"),
            Refusal::ReadOnly => tr!("The archive is open read-only"),
        }
    }
}

/// The folder row a drag would drop into: green (the cursor is blue, the
/// selection has its own color); translucent so the name stays readable.
const DROP_ROW_FILL: Color32 = Color32::from_rgba_premultiplied(30, 110, 45, 120);
const DROP_ROW_EDGE: Color32 = Color32::from_rgb(20, 150, 50);

/// Highlight a drop target: the panel (green, gray when refused), the folder
/// row under the cursor and a label saying what happens.
pub fn paint_target(painter: &egui::Painter, panel: Rect, row: Option<Rect>, refused: bool, text: &str) {
    let color = if refused { Color32::from_gray(120) } else { Color32::from_rgb(30, 130, 60) };
    painter.rect_filled(panel, 4.0, color.gamma_multiply(0.10));
    painter.rect_stroke(panel.shrink(1.0), 4.0, Stroke::new(3.0, color), StrokeKind::Inside);
    if let Some(r) = row {
        // The target folder row: solid green, unlike the cursor (blue) and selection.
        let (fill, edge) = if refused { (color.gamma_multiply(0.30), color) } else { (DROP_ROW_FILL, DROP_ROW_EDGE) };
        painter.rect_filled(r.expand(1.0), 3.0, fill);
        painter.rect_stroke(r.expand(1.0), 3.0, Stroke::new(3.0, edge), StrokeKind::Inside);
    }
    let at = row.map_or(panel.center(), |r| r.center() + Vec2::new(0.0, 26.0));
    let galley = painter.layout_no_wrap(text.to_string(), crate::fonts::prop(18.0), Color32::WHITE);
    let bg = Rect::from_center_size(at, galley.size() + Vec2::new(24.0, 12.0));
    painter.rect_filled(bg, 6.0, color);
    painter.galley(bg.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// Where a panel is on screen and where its (visible) folder rows are.
#[derive(Debug, Clone)]
pub struct PaneZone {
    pub at: PaneRef,
    pub rect: Rect,
    pub dirs: Vec<(Rect, String)>,
}

/// Where in a panel the files were dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spot {
    /// The panel's current directory.
    Here,
    /// Onto the row of this folder (or of "..", [`crate::pane::UP`]).
    Folder(String),
}

impl Spot {
    /// The spot when it is a real folder row (not "..").
    fn filter_real(self) -> Option<Spot> {
        match &self {
            Spot::Folder(n) if n == crate::pane::UP => None,
            _ => Some(self),
        }
    }
}

impl Spot {
    /// The folder the files go to, in a panel showing `path` on `fs`.
    pub fn dest(&self, fs: &dyn rolid_core::Vfs, path: &str) -> String {
        match self {
            Spot::Here => path.to_string(),
            Spot::Folder(n) if n == crate::pane::UP => fs.parent(path).unwrap_or_else(|| path.to_string()),
            Spot::Folder(n) => fs.join(path, n),
        }
    }
}

/// The panel (and folder row) under `pos`; `None` outside the panels.
pub fn hit(zones: &[PaneZone], pos: Pos2) -> Option<(PaneRef, Spot)> {
    let z = zones.iter().find(|z| z.rect.contains(pos))?;
    let spot = z
        .dirs
        .iter()
        .find(|(r, _)| r.contains(pos))
        .map_or(Spot::Here, |(_, n)| Spot::Folder(n.clone()));
    Some((z.at, spot))
}

/// One dropped file or folder, as the copy job takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropItem {
    pub dir: String,
    pub name: String,
    /// Bytes (for a folder: of everything in it that will be copied).
    pub size: u64,
}

/// What a drop will transfer.
#[derive(Debug, Default)]
pub struct DropPlan {
    pub items: Vec<DropItem>,
    /// Files in total (folders counted through).
    pub files: u64,
    pub bytes: u64,
    /// Dropped items that cannot be transferred (shown in the summary).
    pub problems: Vec<Failure>,
}

impl DropPlan {
    /// Large enough to ask first (besides production servers and conflicts).
    pub fn is_large(&self) -> bool {
        self.files > CONFIRM_FILES || self.bytes > CONFIRM_BYTES
    }
}

fn problem(plan: &mut DropPlan, path: &Path, error: rolid_i18n::Text) {
    plan.problems.push(Failure { path: path.to_string_lossy().into_owned(), error });
}

/// Build the transfer from dropped paths. Paths are taken as `Path` (names
/// that are not UTF-8 are reported, never mangled). Paths that do not exist
/// (links, text) are ignored silently. `local_dest` is the target directory
/// when it is on this computer: copying into itself is refused.
pub fn plan(paths: &[PathBuf], local_dest: Option<&Path>) -> DropPlan {
    let mut plan = DropPlan::default();
    let dest = local_dest.map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()));
    for p in paths {
        if !p.is_absolute() {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(p) else { continue };
        let (Some(name), Some(dir)) = (p.file_name(), p.parent()) else {
            problem(&mut plan, p, txt!("the whole root of a disk cannot be transferred"));
            continue;
        };
        let (Some(name), Some(dir)) = (name.to_str(), dir.to_str()) else {
            problem(&mut plan, p, txt!("the name is not valid UTF-8, it cannot be transferred"));
            continue;
        };
        if validate_name(name).is_err() {
            problem(&mut plan, p, txt!("name not allowed"));
            continue;
        }
        if plan.items.iter().any(|i| i.name == name) {
            problem(&mut plan, p, txt!("another dropped item has the same name (it would overwrite it)"));
            continue;
        }
        let target = std::fs::metadata(p).ok(); // follows a symlink, like F5
        let is_dir = target.as_ref().is_some_and(|m| m.is_dir());
        if let Some(d) = &dest {
            let src = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            let src_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
            if is_dir && d.starts_with(&src) {
                problem(&mut plan, p, txt!("a folder cannot be copied into itself"));
                continue;
            }
            if src_dir == *d {
                problem(&mut plan, p, txt!("the target is the same folder the item is already in"));
                continue;
            }
        }
        let size = if is_dir {
            let before = plan.bytes;
            tally(p, &mut plan);
            plan.bytes - before
        } else if meta.is_file() || meta.file_type().is_symlink() {
            let n = target.map(|m| m.len()).unwrap_or(0);
            plan.files += 1;
            plan.bytes += n;
            n
        } else {
            problem(&mut plan, p, txt!("not a regular file or folder"));
            continue;
        };
        plan.items.push(DropItem { dir: dir.to_owned(), name: name.to_owned(), size });
    }
    plan
}

/// Count what copying the folder will transfer, by the same rules as the copy:
/// subfolders recursively, symlinked folders inside not followed, special files
/// skipped. Unreadable folders are not an error here (the copy reports them).
fn tally(dir: &Path, plan: &mut DropPlan) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for de in rd.flatten() {
        let Ok(ft) = de.file_type() else { continue };
        if ft.is_dir() {
            tally(&de.path(), plan);
        } else if ft.is_symlink() {
            match std::fs::metadata(de.path()) {
                Ok(m) if m.is_dir() => {}
                Ok(m) => {
                    plan.files += 1;
                    plan.bytes += m.len();
                }
                Err(_) => plan.files += 1, // broken link: the copy will report it
            }
        } else if ft.is_file() {
            plan.files += 1;
            plan.bytes += de.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
}

/// The plan is ready (counted, conflicts in the target checked).
pub struct DropReady {
    pub at: PaneRef,
    pub dest: String,
    pub dest_label: String,
    pub dfs: Arc<dyn Vfs>,
    pub plan: DropPlan,
    /// Dropped names that already exist in the target.
    pub conflicts: Vec<String>,
    pub production: bool,
    pub remote: bool,
}

/// Confirmation of a drop (production, large, or something already exists).
pub struct DropAsk {
    pub ready: DropReady,
    /// `None` = nothing preselected (setting "no preset").
    pub conflict: Option<Conflict>,
}

/// Files that could not be transferred by a finished job.
pub struct Report {
    pub id: u64,
    pub title: String,
    pub failures: Vec<Failure>,
}

#[derive(Default)]
pub struct DropState {
    /// Panels on screen (from the last frame).
    pub zones: Vec<PaneZone>,
    /// Tabs on screen (from the last frame), for dragging inside the app.
    pub tabs: Vec<(PaneRef, Rect)>,
    /// Dropped paths waiting for the cursor position.
    pending: Option<(Vec<PathBuf>, Instant)>,
    pub reports: Vec<Report>,
    pub show_report: Option<u64>,
}

impl App {
    /// Why files cannot be dropped into this panel (`None` = they can).
    pub(super) fn drop_refusal(&self, at: PaneRef) -> Option<Refusal> {
        let p = self.pane(at)?;
        if p.offline || p.connecting || self.pane_disconnected(at) {
            Some(Refusal::NotConnected)
        } else if p.search.is_some() {
            Some(Refusal::Search)
        } else if p.fs.read_only() {
            Some(Refusal::ReadOnly)
        } else {
            None
        }
    }

    /// X11: while files are dragged over the window (or a drop waits for its
    /// position), keep the pointer query running.
    pub(super) fn x11_drag_frame(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty()) || self.drop.pending.is_some();
        if !dragging {
            if let Some(start) = self.drag_log.take() {
                self.log_drag_query(start);
            }
            return;
        }
        if self.x11.is_none() && !self.x11_tried {
            self.x11_tried = true;
            #[cfg(all(unix, not(target_os = "macos")))]
            if super::x11_drag::is_x11() {
                if let Some(win) = super::x11_drag::window_id(frame) {
                    self.x11 = Some(super::x11_drag::Tracker::spawn(super::x11_drag::x11_query(win)));
                } else {
                    self.x11_why = Some("the window has no X11 id");
                }
            } else {
                self.x11_why = Some("not an X11 session (Wayland or no DISPLAY)");
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            {
                let _ = frame;
                self.x11_why = Some("not used on this system");
            }
        }
        if self.drag_log.is_none() {
            self.drag_log = Some(self.x11.as_ref().map_or(0, |t| t.status().0));
        }
        if let Some(t) = self.x11.as_ref().filter(|t| !t.failed()) {
            t.want();
            ctx.request_repaint_after(super::x11_drag::PERIOD);
        }
    }

    /// Once per drag: did the pointer query (XQueryPointer) work, and if not, why.
    fn log_drag_query(&self, start: u64) {
        match &self.x11 {
            Some(t) => match t.status() {
                (n, None) if n > start => tracing::info!("drag: the cursor position query (XQueryPointer) works ({} replies)", n - start),
                (_, None) => tracing::info!("drag: the cursor position query (XQueryPointer) has not replied yet"),
                (n, Some(e)) => tracing::info!("drag: the cursor position query (XQueryPointer) does not work: {e} (earlier replies {n})"),
            },
            None => tracing::info!(
                "drag: the cursor position query (XQueryPointer) was not used: {}",
                self.x11_why.unwrap_or("unknown reason")
            ),
        }
    }

    /// Cursor position from the X server query (`None` = unknown).
    fn queried_pos(&self, ctx: &egui::Context) -> Option<Pos2> {
        let t = self.x11.as_ref().filter(|t| !t.failed())?;
        Some(super::x11_drag::to_points(t.latest()?, ctx.pixels_per_point()))
    }

    /// Per frame, before the panels: take dropped files, find where they went.
    pub(super) fn drop_frame(&mut self, ctx: &egui::Context) {
        let (dropped, moved, hover) = ctx.input(|i| {
            let dropped: Vec<PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
            let moved = i.events.iter().rev().find_map(|e| match e {
                egui::Event::PointerMoved(p) => Some(*p),
                _ => None,
            });
            (dropped, moved, i.pointer.hover_pos())
        });
        if !dropped.is_empty() {
            tracing::info!("dropped {} items", dropped.len());
            self.drop.pending = Some((dropped, Instant::now()));
        }
        let Some((_, since)) = &self.drop.pending else { return };
        // X11: the position comes with the first pointer event after the drop
        // (usually in the same frame). Elsewhere the pointer is known all along.
        // The exact event wins; the queried position (X11) is the fallback.
        let queried = self.queried_pos(ctx);
        let wait = if queried.is_some() { QUERIED_WAIT } else { POS_WAIT };
        let late = since.elapsed() > wait;
        let pos = moved.or(hover).or(queried.filter(|_| late));
        if pos.is_none() && !late {
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        let (paths, _) = self.drop.pending.take().unwrap();
        match pos {
            Some(pos) => match hit(&self.drop.zones, pos) {
                Some((at, spot)) => self.drop_into(at, spot, paths),
                None => tracing::info!("drop outside the panels: canceled"),
            },
            None => {
                if matches!(self.dialog, Dialog::None) {
                    self.dialog = Dialog::DropWhere { paths };
                    // Keyboard focus is still in the file manager the files came from.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else {
                    self.last_error = Some(tr!("Drop canceled: close the open window first."));
                }
            }
        }
    }

    /// Files dropped onto panel `at`: plan the transfer in the background.
    pub(super) fn drop_into(&mut self, at: PaneRef, spot: Spot, paths: Vec<PathBuf>) {
        if let Some(why) = self.drop_refusal(at) {
            self.last_error = Some(if why == Refusal::NotConnected {
                tr!("The panel is not connected – the files were not uploaded.")
            } else {
                tr!("The target panel shows search results – open a folder in it first.")
            });
            return;
        }
        let Some(p) = self.pane(at) else { return };
        let dest = spot.dest(p.fs.as_ref(), &p.path);
        let dfs = p.fs.clone();
        let remote = dfs.is_remote();
        let production = p.remote.as_ref().is_some_and(|r| r.production);
        let dest_label = format!("{}:{dest}", p.display_label());
        // Onto a folder row: the cursor goes to that folder at once.
        if let Some(Spot::Folder(n)) = spot.clone().filter_real() {
            if let Some(p) = self.pane_mut(at) {
                p.select_name(&n);
            }
        }
        tracing::info!("drop into {dest_label}");
        self.spawn(async move {
            let local_dest = (!remote).then(|| PathBuf::from(&dest));
            let plan = tokio::task::spawn_blocking(move || plan(&paths, local_dest.as_deref()))
                .await
                .unwrap_or_default();
            let mut conflicts = Vec::new();
            for it in &plan.items {
                if dfs.stat(&dfs.join(&dest, &it.name)).await.is_ok() {
                    conflicts.push(it.name.clone());
                }
            }
            Msg::DropReady(Box::new(DropReady { at, dest, dest_label, dfs, plan, conflicts, production, remote }))
        });
    }

    pub(super) fn on_drop_ready(&mut self, r: DropReady) {
        if r.plan.items.is_empty() {
            // Nothing transferable (links/text are ignored silently).
            if !r.plan.problems.is_empty() {
                let id = self.id();
                let title = tr!("Drag and drop");
                self.drop.reports.push(Report { id, title, failures: r.plan.problems });
            }
            return;
        }
        if self.pane(r.at).is_none() {
            return; // the tab was closed meanwhile
        }
        let ask = r.production || r.plan.is_large() || !r.conflicts.is_empty();
        if !ask {
            // Nothing exists in the target: "skip existing" overwrites nothing
            // even if something appears meanwhile.
            self.start_drop(r, Conflict::Skip);
        } else if matches!(self.dialog, Dialog::None) {
            let conflict = super::initial_conflict(self.settings.drop_preset(), r.production);
            self.dialog = Dialog::Drop(Box::new(DropAsk { ready: r, conflict }));
            // Keyboard focus is still in the file manager (Enter / Esc should work here).
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else {
            self.last_error = Some(tr!("Drop canceled: close the open window first."));
        }
    }

    fn start_drop(&mut self, r: DropReady, conflict: Conflict) {
        let items: Vec<Target> = r
            .plan
            .items
            .iter()
            .map(|i| Target { dir: i.dir.clone(), name: i.name.clone(), label: i.name.clone(), size: i.size })
            .collect();
        let title = if items.len() == 1 {
            format!("{} → {}", items[0].name, r.dest_label)
        } else {
            trn!("{n} item → {dest}", "{n} items → {dest}", items.len(), dest = &r.dest_label)
        };
        let links = self.pane_link(r.at).into_iter().collect();
        // Dropped into the shown folder: the cursor goes to the first new item.
        let (at, dest) = (r.at, r.dest.clone());
        let names: Vec<String> = items.iter().map(|t| t.name.clone()).collect();
        let then: super::Then = Box::new(move |a: &mut App| {
            if let Some(p) = a.pane_mut(at) {
                if p.path == dest && p.search.is_none() {
                    p.select_any = Some(names);
                    a.load(at, dest, None);
                }
            }
        });
        self.spawn_copy(CopySpec {
            sfs: Arc::new(LocalFs),
            dfs: r.dfs,
            dpath: r.dest,
            items,
            conflict,
            title,
            refresh: vec![r.at],
            links,
            auto_retry: !r.production || conflict == Conflict::Skip,
            state: None,
            keep_going: true,
            pre_failed: r.plan.problems,
            then: Some(then),
            temp: None,
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    /// Record the panels on screen (for finding the drop target).
    pub(super) fn drop_zone(&mut self, at: PaneRef, rect: Rect) {
        let dirs = self.pane(at).map(|p| p.drop_dirs.clone()).unwrap_or_default();
        self.drop.zones.push(PaneZone { at, rect, dirs });
    }

    /// While files are dragged over the window: highlight where they can go.
    pub(super) fn drop_hover_ui(&self, ctx: &egui::Context) {
        let n = ctx.input(|i| i.raw.hovered_files.len()) as u64;
        if n == 0 {
            return;
        }
        let pos = ctx.input(|i| i.pointer.hover_pos()).or_else(|| self.queried_pos(ctx));
        let target = pos.and_then(|p| hit(&self.drop.zones, p));
        if pos.is_some() && target.is_none() {
            return; // over the menu or the toolbar: nowhere to drop
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop-hover")));
        for z in &self.drop.zones {
            let row = match &target {
                // Position known: only the panel (and folder row) under the cursor.
                Some((at, spot)) if *at == z.at => match spot {
                    Spot::Folder(name) => z.dirs.iter().find(|(_, d)| d == name).map(|(r, _)| *r),
                    Spot::Here => None,
                },
                Some(_) => continue,
                // Position unknown (X11 while dragging): every panel.
                None => None,
            };
            let refusal = self.drop_refusal(z.at);
            let remote = self.pane(z.at).is_some_and(|p| p.fs.is_remote());
            let text = match refusal {
                Some(why) => why.text(),
                None if remote => trn!("Upload here: {n} item", "Upload here: {n} items", n),
                None => trn!("Copy here: {n} item", "Copy here: {n} items", n),
            };
            paint_target(&painter, z.rect, row, refusal.is_some(), &text);
        }
    }

    /// The drop dialogs: confirmation, and "where to?" when the position is unknown.
    pub(super) fn drop_dialog_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut start: Option<Conflict> = None;
        let mut into: Option<PaneRef> = None;
        let panes: Vec<(PaneRef, String, Option<Refusal>)> = self
            .drop
            .zones
            .iter()
            .filter_map(|z| self.pane(z.at).map(|p| (z.at, format!("{}:{}", p.display_label(), p.path), self.drop_refusal(z.at))))
            .collect();
        match &mut self.dialog {
            Dialog::Drop(ask) => {
                let r = &ask.ready;
                let (role, title) = if r.remote { (Role::Upload, tr!("Upload")) } else { (Role::Copy, trc!("verb", "Copy")) };
                dialog::window("drop-ask", title.clone()).icon(role).min_width(460.0).danger(r.production).show(ctx, |ui| {
                    let size = human_size(r.plan.bytes);
                    if r.remote {
                        dialog::lead(ui, trn!("Upload {n} item ({size}) to the server?", "Upload {n} items ({size}) to the server?", r.plan.files, size = &size));
                    } else {
                        dialog::lead(ui, trn!("Copy {n} item ({size}) here?", "Copy {n} items ({size}) here?", r.plan.files, size = &size));
                    }
                    ui.label(tr!("to  {dest}", dest = &r.dest_label));
                    if r.production {
                        dialog::warn(ui, tr!("⚠ PRODUCTION SERVER: Enter is disabled, confirm with the button."), true);
                    }
                    if r.plan.is_large() {
                        dialog::note(
                            ui,
                            tr!("Large transfer (more than {files} files or {size}).", files = CONFIRM_FILES, size = human_size(CONFIRM_BYTES)),
                        );
                    }
                    if !r.conflicts.is_empty() {
                        let mut names = r.conflicts.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
                        if r.conflicts.len() > 5 {
                            names += ", …";
                        }
                        let heading = tr!("Existing files");
                        dialog::group(ui, &heading, |ui| {
                            dialog::warn(ui, tr!("⚠ Already in the target: {names}. What to do with the existing files?", names = names), false);
                            super::conflict_choice(ui, &mut ask.conflict);
                        });
                    }
                    if !r.plan.problems.is_empty() {
                        dialog::note(
                            ui,
                            trn!(
                                "{n} item cannot be transferred – it will be in the error summary.",
                                "{n} items cannot be transferred – they will be in the error summary.",
                                r.plan.problems.len()
                            ),
                        );
                    }
                    dialog::note(ui, tr!("The source files are only read, nothing in them is changed."));
                    // Without conflicts nothing is overwritten; with them an answer is needed.
                    let choice = if r.conflicts.is_empty() { Some(Conflict::Skip) } else { ask.conflict };
                    let label = match (r.production, r.remote) {
                        (true, true) => tr!("Upload to production"),
                        (true, false) => tr!("Copy to production"),
                        _ => title.clone(),
                    };
                    let b = dialog::buttons(ui, [dialog::confirm(role, label, r.production).enabled(choice.is_some()), dialog::cancel()]);
                    if b[0] {
                        start = choice;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            Dialog::DropWhere { .. } => {
                dialog::window("drop-where", tr!("Where to copy the dropped files?")).icon(Role::Copy).show(ctx, |ui| {
                    dialog::lead(ui, tr!("Could not find out where the files were dropped – choose a panel."));
                    let enter = dialog::enter(ui);
                    for (i, (at, label, refusal)) in panes.iter().enumerate() {
                        let text = if at.side == 0 { tr!("◧ Left: {path}", path = label) } else { tr!("◨ Right: {path}", path = label) };
                        let b = crate::look::btn(Role::Folder, text).default(i == 0).enabled(refusal.is_none()).ui(ui);
                        let b = match refusal {
                            Some(why) => b.on_disabled_hover_text(why.text()),
                            None => b,
                        };
                        if b.clicked() || (i == 0 && enter && refusal.is_none()) {
                            into = Some(*at);
                        }
                    }
                    let b = dialog::buttons(ui, [dialog::cancel()]);
                    if b[0] {
                        close = true;
                    }
                });
            }
            _ => {}
        }
        if let Some(c) = start {
            if let Dialog::Drop(ask) = std::mem::replace(&mut self.dialog, Dialog::None) {
                self.start_drop(ask.ready, c);
            }
        } else if let Some(at) = into {
            if let Dialog::DropWhere { paths } = std::mem::replace(&mut self.dialog, Dialog::None) {
                self.drop_into(at, Spot::Here, paths);
            }
        } else if close {
            self.dialog = Dialog::None;
        }
    }

    /// Status bar: finished transfers with files that could not be copied.
    pub(super) fn reports_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        for r in &self.drop.reports {
            ui.horizontal(|ui| {
                ui.colored_label(
                    Color32::from_rgb(190, 90, 0),
                    trn!("⚠ {title}: {n} item could not be transferred", "⚠ {title}: {n} items could not be transferred", r.failures.len(), title = &r.title),
                );
                if ui.small_button(tr!("Details")).clicked() {
                    self.drop.show_report = Some(r.id);
                }
                if ui.small_button("✖").clicked() {
                    remove = Some(r.id);
                }
            });
        }
        if let Some(id) = remove {
            self.drop.reports.retain(|r| r.id != id);
        }
    }

    /// The window with the list of failures of one transfer.
    pub(super) fn report_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.drop.show_report else { return };
        let Some(r) = self.drop.reports.iter().find(|r| r.id == id) else {
            self.drop.show_report = None;
            return;
        };
        let mut open = true;
        egui::Window::new(tr!("Transfer error summary")).open(&mut open).default_width(640.0).show(ctx, |ui| {
            ui.label(&r.title);
            ui.weak(tr!("These items were not transferred, the rest of the transfer finished:"));
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for f in &r.failures {
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(&f.path);
                        ui.colored_label(Color32::from_rgb(190, 20, 20), rolid_i18n::t(&f.error));
                    });
                }
            });
        });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drop.show_report = None;
        }
    }
}

/// All failures of a job (dropped items refused up front + files that failed),
/// each path once.
pub fn merge_failures(mut a: Vec<Failure>, b: &[Failure]) -> Vec<Failure> {
    for f in b {
        if !a.iter().any(|x| x.path == f.path) {
            a.push(f.clone());
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(side: usize) -> PaneRef {
        PaneRef { tab: side as u64 + 1, side }
    }

    fn zones() -> Vec<PaneZone> {
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
        vec![
            PaneZone { at: at(0), rect: r(0.0, 0.0, 400.0, 600.0), dirs: vec![(r(4.0, 100.0, 396.0, 120.0), "www".into())] },
            PaneZone { at: at(1), rect: r(410.0, 0.0, 800.0, 600.0), dirs: vec![(r(414.0, 140.0, 796.0, 160.0), "docs".into())] },
        ]
    }

    #[test]
    fn target_is_the_folder_row_or_the_current_directory() {
        let z = zones();
        assert_eq!(hit(&z, Pos2::new(50.0, 110.0)), Some((at(0), Spot::Folder("www".into()))));
        assert_eq!(hit(&z, Pos2::new(50.0, 130.0)), Some((at(0), Spot::Here)), "below the folder row");
        assert_eq!(hit(&z, Pos2::new(500.0, 150.0)), Some((at(1), Spot::Folder("docs".into()))));
        assert_eq!(hit(&z, Pos2::new(500.0, 110.0)), Some((at(1), Spot::Here)), "row of the other panel");
        assert_eq!(hit(&z, Pos2::new(405.0, 300.0)), None, "the gap between panels");
        assert_eq!(hit(&z, Pos2::new(300.0, 700.0)), None, "below the panels (status bar)");
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rolid-drop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn plan_counts_folders_recursively_like_the_copy() {
        let d = tmp("plan");
        std::fs::create_dir_all(d.join("site/css/old")).unwrap();
        std::fs::write(d.join("site/index.html"), vec![0u8; 1000]).unwrap();
        std::fs::write(d.join("site/css/a.css"), vec![0u8; 200]).unwrap();
        std::fs::write(d.join("site/css/old/b.css"), vec![0u8; 30]).unwrap();
        std::fs::write(d.join("note.txt"), b"12345").unwrap();
        #[cfg(unix)]
        {
            // A symlinked folder inside is not followed (as with F5), a symlinked file is copied.
            std::os::unix::fs::symlink(d.join("site/css"), d.join("site/css-link")).unwrap();
            std::os::unix::fs::symlink(d.join("note.txt"), d.join("site/note-link")).unwrap();
        }
        let p = plan(&[d.join("site"), d.join("note.txt")], None);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        let ds = d.to_string_lossy().into_owned();
        let extra = if cfg!(unix) { 5 } else { 0 };
        assert_eq!(
            p.items,
            [
                DropItem { dir: ds.clone(), name: "site".into(), size: 1230 + extra },
                DropItem { dir: ds, name: "note.txt".into(), size: 5 },
            ]
        );
        assert_eq!(p.files, if cfg!(unix) { 5 } else { 4 });
        assert_eq!(p.bytes, 1235 + extra);
        assert!(!p.is_large());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn dangerous_or_odd_drops_are_refused() {
        let d = tmp("bad");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("a/same.txt"), b"1").unwrap();
        std::fs::write(d.join("b/same.txt"), b"2").unwrap();
        std::fs::write(d.join("ok.txt"), b"ok").unwrap();
        std::fs::create_dir_all(d.join("x")).unwrap();
        let paths = vec![
            PathBuf::from("/"),                         // the root: no name
            PathBuf::from("relative/file.txt"),         // not absolute: ignored
            d.join("does-not-exist"),                   // a link/text, not a file: ignored
            d.join("a/same.txt"),
            d.join("b/same.txt"),                       // the same name twice
            d.join("x/../ok.txt"),                      // `..` inside: resolves to ok.txt
        ];
        let p = plan(&paths, None);
        let names: Vec<&str> = p.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["same.txt", "ok.txt"]);
        assert_eq!(p.problems.len(), 2, "{:?}", p.problems);
        assert_eq!(p.problems[0].path, "/");
        assert!(p.problems[1].error.to_string().contains("same name"));
        // The name a copy job gets is always a plain name (no `..`, no slash).
        for i in &p.items {
            assert!(validate_name(&i.name).is_ok());
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_utf8_are_reported_not_mangled() {
        use std::os::unix::ffi::OsStrExt;
        let d = tmp("utf8");
        let bad = d.join(std::ffi::OsStr::from_bytes(b"invoice-\xe8.pdf"));
        std::fs::write(&bad, b"x").unwrap();
        let p = plan(std::slice::from_ref(&bad), None);
        assert!(p.items.is_empty());
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].error.to_string().contains("UTF-8"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn local_copy_into_itself_is_refused() {
        let d = tmp("self");
        std::fs::create_dir_all(d.join("proj/sub")).unwrap();
        std::fs::write(d.join("f.txt"), b"x").unwrap();
        // Folder into its own subfolder, and a file into the folder it is in.
        let p = plan(&[d.join("proj"), d.join("f.txt")], Some(&d.join("proj/sub")));
        assert_eq!(p.items.len(), 1, "f.txt into proj/sub is fine");
        assert!(p.problems[0].error.to_string().contains("into itself"));
        let p = plan(&[d.join("f.txt")], Some(&d));
        assert!(p.items.is_empty());
        assert!(p.problems[0].error.to_string().contains("same folder"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn large_drops_need_a_confirmation() {
        let p = DropPlan { files: CONFIRM_FILES + 1, ..Default::default() };
        assert!(p.is_large());
        let p = DropPlan { files: 3, bytes: CONFIRM_BYTES + 1, ..Default::default() };
        assert!(p.is_large());
        let p = DropPlan { files: CONFIRM_FILES, bytes: CONFIRM_BYTES, ..Default::default() };
        assert!(!p.is_large());
    }

    #[test]
    fn summary_lists_each_failure_once() {
        let f = |p: &str, e: &str| Failure { path: p.into(), error: rolid_core::vfs::raw_text(e) };
        let all = merge_failures(
            vec![f("/a/x", "name not allowed")],
            &[f("/a/secret.txt", "Permission denied"), f("/a/x", "again"), f("/a/locked", "Permission denied")],
        );
        let paths: Vec<&str> = all.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/a/x", "/a/secret.txt", "/a/locked"]);
    }
}
