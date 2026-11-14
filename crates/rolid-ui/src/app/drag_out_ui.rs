//! The app side of dragging files out to other applications (X11, see
//! [`super::drag_out`]): starting the source when the dragged rows leave the
//! window, downloading files of a server or an archive (into the target's
//! folder with XDS, else into a private temporary folder), and deleting the
//! temporary folders when they are no longer used.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use rolid_core::copy::Conflict;
use rolid_core::Vfs;
use rolid_edit::recover::{self, DragDir, ProcId};

use super::drag_out::{self, Action, Cmd, End, Event, Need, Offer, Outcome, Source};
use super::{App, CopySpec, Dialog, Msg, PaneRef};
use crate::archive_fs::ArchiveFs;
use crate::dialog;
use crate::look::Role;
use crate::pane::Target;

/// Above this a download into a temporary folder needs a confirmation.
pub const LARGE: u64 = 1 << 30;
/// A temporary folder goes when nothing has used it this long after the drop.
const TEMP_IDLE: Duration = Duration::from_secs(30);
const TEMP_CHECK: Duration = Duration::from_secs(10);

/// Where the dragged files are.
pub enum From {
    Local,
    Remote(Arc<dyn Vfs>),
    /// The archive and the folder in it.
    Archive(Arc<ArchiveFs>, String),
}

/// A drag out in progress.
pub struct DragOut {
    src: Source,
    pane: PaneRef,
    items: Vec<Target>,
    from: From,
    /// The button was released outside: the target has the drop.
    pub dropping: bool,
    /// The drag came back and ended inside the window.
    inside: bool,
    /// The download into a temporary folder (for a target without XDS).
    temp: Option<DragDir>,
    job: Option<u64>,
    answered: bool,
}

impl DragOut {
    pub fn status(&self) -> drag_out::Status {
        self.src.status()
    }
}

/// A temporary folder of a finished drag, deleted when unused.
pub struct DragTemp {
    dir: DragDir,
    idle_since: Option<Instant>,
    next_check: Instant,
    checking: bool,
}

/// "Download before handing over?" (more than [`LARGE`]).
pub struct LargeAsk {
    bytes: u64,
}

/// Is a file in `dir` open in some process (Linux: `/proc/*/fd`)? Processes of
/// other users cannot be seen, but they cannot read our 0700 folder either.
pub fn in_use(dir: &Path) -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else { return false };
    for p in procs.flatten() {
        if !p.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(p.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path()).is_ok_and(|l| l.starts_with(dir)) {
                return true;
            }
        }
    }
    false
}

/// The conflict rule for files saved into another application's folder
/// (XDS): the drag-and-drop preset; "ask" cannot ask there, so it skips.
fn xds_conflict(p: rolid_config::ConflictPreset) -> Conflict {
    match p {
        rolid_config::ConflictPreset::Overwrite => Conflict::Overwrite,
        rolid_config::ConflictPreset::Newer => Conflict::Newer,
        rolid_config::ConflictPreset::Skip | rolid_config::ConflictPreset::Ask => Conflict::Skip,
    }
}

impl App {
    /// The dragged rows left the window: hand the drag to the X server.
    pub(super) fn start_drag_out(&mut self, ctx: &egui::Context) {
        if self.drag_out.is_some() || !super::x11_drag::is_x11() {
            return;
        }
        let (Some(win), Some(d)) = (self.x11_win, self.inner_drag.as_ref()) else { return };
        let Some(p) = self.pane(d.from) else { return };
        let items = d.items.clone();
        let from = match (&p.archive, p.fs.is_local_disk()) {
            (Some(st), _) => From::Archive(st.fs.clone(), p.path.clone()),
            (None, true) => From::Local,
            (None, false) => From::Remote(p.fs.clone()),
        };
        let offer = match &from {
            From::Local => {
                let paths: Vec<PathBuf> = items.iter().map(|t| PathBuf::from(p.fs.join(&t.dir, &t.name))).collect();
                Offer { uri_list: Some(drag_out::uri_list(&paths)), xds_name: None, allow_move: !p.fs.read_only() }
            }
            _ => Offer { uri_list: None, xds_name: Some(items[0].name.clone()), allow_move: false },
        };
        let c = ctx.clone();
        let src = Source::start(win, offer, move || c.request_repaint());
        tracing::info!("drag out of the window: {} items", items.len());
        self.drag_out = Some(DragOut { src, pane: d.from, items, from, dropping: false, inside: false, temp: None, job: None, answered: false });
    }

    /// Esc while dragging: the target is told to forget the drag.
    pub(super) fn cancel_drag_out(&mut self) {
        if let Some(o) = self.drag_out.as_ref().filter(|o| !o.dropping) {
            o.src.send(Cmd::Cancel);
        }
    }

    /// The drag ended inside the window (a drop there or nowhere).
    pub(super) fn drag_out_back_inside(&mut self) {
        if let Some(o) = self.drag_out.as_mut().filter(|o| !o.dropping) {
            o.inside = true;
            o.src.send(Cmd::Cancel);
        }
    }

    /// Every frame: the thread's requests, a failed download, the end.
    pub(super) fn drag_out_frame(&mut self, ctx: &egui::Context) {
        self.drag_temps_frame(ctx);
        let Some(o) = &mut self.drag_out else { return };
        // The temporary download failed or was canceled.
        if let Some(id) = o.job {
            if !o.answered && !self.jobs.iter().any(|j| j.id == id) {
                o.answered = true;
                o.src.send(Cmd::Data(None));
            }
        }
        while let Some(ev) = self.drag_out.as_ref().and_then(|o| o.src.poll()) {
            match ev {
                Event::Need(Need::UriList) => self.drag_out_need_files(),
                Event::Need(Need::SaveTo(dir)) => self.drag_out_save_to(dir),
                Event::Ended(end) => self.drag_out_ended(end),
            }
        }
        if self.drag_out.as_ref().is_some_and(|o| o.dropping) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn drag_out_ended(&mut self, end: End) {
        let Some(o) = self.drag_out.take() else { return };
        let result = if o.inside && end.result == Outcome::Canceled { Outcome::Inside } else { end.result.clone() };
        if !matches!(result, Outcome::Inside | Outcome::Canceled) {
            // Ended outside (the thread may see the release before the window does).
            self.inner_drag = None;
        }
        // Once per drag; no file names, only the application and the method.
        tracing::info!(
            "drag out: target {}, method {}, result: {}",
            end.target.as_deref().unwrap_or("none"),
            end.method.map_or("none", |m| m.label()),
            result.label()
        );
        if let Some(t) = o.temp {
            if matches!(result, Outcome::Done(_)) {
                self.drag_temps.push(DragTemp { dir: t, idle_since: None, next_check: Instant::now() + TEMP_CHECK, checking: false });
            } else if let Err(e) = t.remove() {
                tracing::warn!("cannot delete a temporary folder: {e}");
            }
        }
        match result {
            Outcome::Done(Action::Move) if matches!(o.from, From::Local) => {
                // The other application moved the files away.
                self.refresh_all(&[o.pane]);
                for t in &o.items {
                    let dir = t.dir.clone();
                    let refs: Vec<PaneRef> = self.local_panes_at(&dir);
                    self.refresh_all(&refs);
                }
            }
            Outcome::Failed(e) => self.last_error = Some(tr!("Dragging out to the other application failed: {error}", error = e)),
            _ => {}
        }
    }

    /// Local panels (not archives, not search results) showing `dir`.
    fn local_panes_at(&self, dir: &str) -> Vec<PaneRef> {
        let dir = dir.trim_end_matches('/');
        self.all_panes()
            .filter(|(_, p)| p.archive.is_none() && p.search.is_none() && p.fs.is_local_disk() && p.path.trim_end_matches('/') == dir)
            .map(|(r, _)| r)
            .collect()
    }

    /// A target without XDS needs the files on this computer.
    fn drag_out_need_files(&mut self) {
        let Some(o) = &self.drag_out else { return };
        let bytes: u64 = o.items.iter().map(|t| t.size).sum();
        if bytes > LARGE && matches!(self.dialog, Dialog::None) {
            self.dialog = Dialog::DragOutLarge(Box::new(LargeAsk { bytes }));
        } else {
            self.drag_out_download();
        }
    }

    pub(super) fn drag_out_large_ui(&mut self, ctx: &egui::Context) {
        let Dialog::DragOutLarge(d) = &self.dialog else { return };
        let size = crate::pane::human_size(d.bytes);
        let mut choice = None;
        dialog::window("drag-out-large", tr!("Download for the other application?")).icon(Role::Copy).show(ctx, |ui| {
            dialog::lead(
                ui,
                tr!("The other application needs the files on this computer: {size} will be downloaded into a temporary folder first.", size = size.as_str()),
            );
            dialog::warn(ui, tr!("Some applications stop waiting for the files after about 30 seconds."), false);
            let b = dialog::buttons(ui, [dialog::btn(Role::Copy, tr!("Download")).default(true), dialog::cancel()]);
            if b[0] {
                choice = Some(true);
            } else if b[1] {
                choice = Some(false);
            }
        });
        if let Some(go) = choice {
            self.dialog = Dialog::None;
            if go {
                self.drag_out_download();
            } else if let Some(o) = &mut self.drag_out {
                o.answered = true;
                o.src.send(Cmd::Data(None));
            }
        }
    }

    fn drag_out_fail(&mut self, why: String) {
        self.last_error = Some(why);
        if let Some(o) = &mut self.drag_out {
            o.answered = true;
            o.src.send(Cmd::Data(None));
        }
    }

    /// Download (or extract) into a private temporary folder, then give the
    /// target the uri-list of the copies.
    fn drag_out_download(&mut self) {
        let base = rolid_edit::temp::base_dir();
        let dir = match recover::create_drag_dir(&base, &ProcId::current()) {
            Ok(d) => d,
            Err(e) => return self.drag_out_fail(tr!("Cannot create a temporary folder: {error}", error = rolid_i18n::t(&e))),
        };
        let links: Vec<_> = self.drag_out.as_ref().and_then(|o| self.pane_link(o.pane)).into_iter().collect();
        let Some(o) = &mut self.drag_out else { return };
        let paths: Vec<PathBuf> = o.items.iter().map(|t| dir.dir.join(&t.name)).collect();
        let list = drag_out::uri_list(&paths);
        let out = dir.dir.clone();
        o.temp = Some(dir);
        let items = o.items.clone();
        let label = super::archive_ui::items_label(&items);
        let title = tr!("{items} → other application", items = label.as_str());
        let ready: super::Then = Box::new(move |a: &mut App| {
            if let Some(o) = &mut a.drag_out {
                o.answered = true;
                o.src.send(Cmd::Data(Some(list)));
            }
        });
        match &o.from {
            From::Local => {}
            From::Remote(fs) => {
                let sfs = fs.clone();
                self.spawn_copy(CopySpec {
                    sfs,
                    dfs: Arc::new(rolid_local::LocalFs),
                    dpath: out.to_string_lossy().into_owned(),
                    items,
                    conflict: Conflict::Overwrite,
                    title,
                    refresh: vec![],
                    links,
                    auto_retry: true,
                    state: None,
                    keep_going: false,
                    pre_failed: Vec::new(),
                    then: Some(ready),
                    temp: None,
                    moving: false,
                    verify: false,
                    target_name: None,
                });
                let id = self.jobs.last().map(|j| j.id);
                if let Some(o) = &mut self.drag_out {
                    o.job = id;
                }
            }
            From::Archive(fs, path) => {
                let (fs, path) = (fs.clone(), path.clone());
                let id = self.extract_for_drag(fs, path, items, out, rolid_archive::Exist::Overwrite, title, ready);
                if let Some(o) = &mut self.drag_out {
                    o.job = id;
                }
            }
        }
    }

    /// XDS: the target chose `dir`; download there through the normal queue.
    fn drag_out_save_to(&mut self, dir: PathBuf) {
        let Some(o) = &self.drag_out else { return };
        if !dir.is_dir() {
            o.src.send(Cmd::Saved(false));
            return;
        }
        let dpath = dir.to_string_lossy().into_owned();
        let conflict = xds_conflict(self.settings.drop_preset());
        let items = o.items.clone();
        let label = super::archive_ui::items_label(&items);
        let title = format!("{label} → {dpath}");
        let refresh = self.local_panes_at(&dpath);
        let pane = o.pane;
        let ok = match &o.from {
            From::Local => false,
            From::Remote(fs) => {
                let sfs = fs.clone();
                let links = [self.pane_link(pane)].into_iter().flatten().collect();
                self.spawn_copy(CopySpec {
                    sfs,
                    dfs: Arc::new(rolid_local::LocalFs),
                    dpath,
                    items,
                    conflict,
                    title,
                    refresh,
                    links,
                    auto_retry: true,
                    state: None,
                    keep_going: false,
                    pre_failed: Vec::new(),
                    then: None,
                    temp: None,
                    moving: false,
                    verify: false,
                    target_name: None,
                });
                true
            }
            From::Archive(fs, path) => {
                let (fs, path) = (fs.clone(), path.clone());
                let done: super::Then = Box::new(move |a: &mut App| a.refresh_all(&refresh));
                self.extract_for_drag(fs, path, items, dir, super::unpack_ui::exist_of(conflict), title, done).is_some()
            }
        };
        if let Some(o) = &self.drag_out {
            o.src.send(Cmd::Saved(ok));
        }
    }

    /// Extract archive items into a local folder as a queued job; `done` runs
    /// after success. The job id, or `None` when it could not start.
    #[allow(clippy::too_many_arguments)]
    fn extract_for_drag(
        &mut self,
        fs: Arc<ArchiveFs>,
        path: String,
        items: Vec<Target>,
        out: PathBuf,
        exist: rolid_archive::Exist,
        title: String,
        done: super::Then,
    ) -> Option<u64> {
        if fs.encrypted && !fs.has_password() {
            self.last_error = Some(tr!("The archive is password protected: open a file in it first (to enter the password), then drag again."));
            return None;
        }
        let names: Vec<String> = items.iter().map(|t| t.name.clone()).collect();
        let sel = fs.select(&path, &names);
        let limits = rolid_archive::Limits {
            max_total: self.settings.unpack_max_bytes(),
            max_entries: self.settings.unpack_items(),
            max_ratio: Some(self.settings.unpack_ratio()),
        };
        let total: u64 = items.iter().map(|t| t.size).sum();
        let report = title.clone();
        let id = self.archive_job(
            title,
            move |ctl| {
                ctl.tick(rolid_archive::Tick::Total { items: sel.items.len() as u64, bytes: total });
                let rep = rolid_archive::extract(&fs.archive, fs.format, &out, &fs.read_opts(), limits, exist, Some(&sel), ctl)?;
                Ok(rep.failures)
            },
            move |a: &mut App, res| match res {
                Ok(failures) if failures.is_empty() => done(a),
                Ok(failures) => {
                    let id = a.id();
                    a.drop.reports.push(super::drop_ui::Report {
                        id,
                        title: report,
                        failures: failures.into_iter().map(|(path, error)| rolid_core::copy::Failure { path, error }).collect(),
                    });
                }
                Err(e) => a.last_error = Some(rolid_i18n::t(&e.text())),
            },
        );
        Some(id)
    }

    /// Delete temporary folders of finished drags once nothing uses them.
    fn drag_temps_frame(&mut self, ctx: &egui::Context) {
        if self.drag_temps.is_empty() {
            return;
        }
        ctx.request_repaint_after(TEMP_CHECK);
        let now = Instant::now();
        for t in self.drag_temps.iter_mut().filter(|t| !t.checking && t.next_check <= now) {
            t.checking = true;
            let dir = t.dir.dir.clone();
            let d2 = dir.clone();
            let tx = self.tx.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let busy = in_use(&d2);
                let _ = tx.send(Msg::Then { job: None, f: Box::new(move |a: &mut App| a.drag_temp_checked(&dir, busy)) });
                ctx2.request_repaint();
            });
        }
    }

    fn drag_temp_checked(&mut self, dir: &Path, busy: bool) {
        let now = Instant::now();
        let Some(i) = self.drag_temps.iter().position(|t| t.dir.dir == dir) else { return };
        let t = &mut self.drag_temps[i];
        t.checking = false;
        t.next_check = now + TEMP_CHECK;
        if busy {
            t.idle_since = None;
            return;
        }
        let since = *t.idle_since.get_or_insert(now);
        if now.duration_since(since) >= TEMP_IDLE {
            let t = self.drag_temps.remove(i);
            match t.dir.remove() {
                Ok(()) => tracing::info!("drag out: temporary folder deleted"),
                Err(e) => tracing::warn!("cannot delete a temporary folder: {e}"),
            }
        }
    }

    /// At exit: every temporary folder goes.
    pub(super) fn drag_out_exit(&mut self) {
        if let Some(o) = self.drag_out.take() {
            o.src.send(Cmd::Cancel);
            if let Some(t) = o.temp {
                let _ = t.remove();
            }
        }
        for t in self.drag_temps.drain(..) {
            let _ = t.dir.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_files_keep_the_temporary_folder() {
        let t = tempfile::tempdir().unwrap();
        let me = ProcId::current();
        let d = recover::create_drag_dir(&t.path().join("edit"), &me).unwrap();
        let f = d.dir.join("a b.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(!in_use(&d.dir), "nothing open");
        let open = std::fs::File::open(&f).unwrap();
        assert!(in_use(&d.dir), "a process reads the copy");
        drop(open);
        assert!(!in_use(&d.dir));
        d.remove().unwrap();
        assert!(!d.dir.exists());
    }

    #[test]
    fn saving_into_another_application_never_asks() {
        use rolid_config::ConflictPreset as P;
        assert_eq!(xds_conflict(P::Skip), Conflict::Skip);
        assert_eq!(xds_conflict(P::Ask), Conflict::Skip, "no dialog there: never overwrite");
        assert_eq!(xds_conflict(P::Newer), Conflict::Newer);
        assert_eq!(xds_conflict(P::Overwrite), Conflict::Overwrite);
    }
}
