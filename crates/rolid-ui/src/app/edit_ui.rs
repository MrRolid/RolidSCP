//! F4: edit a file in an external editor. Local files are opened directly; remote
//! files are downloaded into a private temporary directory, watched, and every
//! save is uploaded back through the transfer queue (with conflict check and, for
//! production servers, confirmation).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use rolid_core::copy::Progress;
use rolid_core::{Entry, Vfs};
use rolid_config::RemoteTarget;
use rolid_edit::conflict::{aside_name, RemoteStamp};
use rolid_edit::recover::{self, EditMeta, Orphan};
use rolid_edit::temp::{self, EditDir};
use rolid_edit::transfer::{self, Ctl};
use rolid_edit::watch::{fingerprint, Debounce, FileWatcher, Fingerprint};
use rolid_edit::apps::{self, AppDb};
use rolid_edit::{editor, LARGE_FILE};
use rolid_local::LocalFs;

use rolid_i18n::rolid_text::txt;

use super::{App, Job, Msg, PaneRef, TabId};
use crate::dialog;
use crate::look::Role;
use crate::pane::{Opener, Target};

const RED: Color32 = Color32::from_rgb(190, 20, 20);
const GREEN: Color32 = Color32::from_rgb(20, 120, 40);
const ORANGE: Color32 = Color32::from_rgb(170, 90, 0);

fn lost_text() -> String {
    tr!("The connection to the server was interrupted – it will be uploaded once it is restored")
}

/// A backend error as the user reads it.
fn err_text(e: &rolid_core::VfsError) -> String {
    rolid_i18n::t(&e.text())
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditState {
    Downloading,
    /// Opened, waiting for saves.
    Watching,
    /// Checking the remote file before upload.
    Checking,
    Uploading,
    /// Last upload succeeded at `time` (HH:MM); `aside` = saved next to the
    /// original under this name.
    Uploaded { time: String, aside: Option<String> },
    /// A change is waiting (conflict, confirmation…).
    NotUploaded(String),
    Failed(String),
}

impl EditState {
    fn text(&self) -> (String, Color32) {
        match self {
            EditState::Downloading => (tr!("downloading…"), ORANGE),
            EditState::Watching => (tr!("being edited"), Color32::GRAY),
            EditState::Checking => (tr!("checking the server…"), ORANGE),
            EditState::Uploading => (tr!("uploading…"), ORANGE),
            EditState::Uploaded { time, aside: None } => (tr!("Uploaded at {time}", time = time.as_str()), GREEN),
            EditState::Uploaded { time, aside: Some(n) } => {
                (tr!("Saved alongside as “{name}” at {time}", name = n.as_str(), time = time.as_str()), GREEN)
            }
            EditState::NotUploaded(m) => (tr!("not uploaded: {reason}", reason = m.as_str()), RED),
            EditState::Failed(m) => (m.clone(), RED),
        }
    }

    fn needs_attention(&self) -> bool {
        matches!(self, EditState::NotUploaded(_) | EditState::Failed(_))
    }
}

pub struct EditSession {
    pub id: u64,
    pub name: String,
    /// Panel / tab the file was opened from.
    pub at: PaneRef,
    /// `user@host:/dir` for display.
    pub location: String,
    pub dir: EditDir,
    fs: Arc<dyn Vfs>,
    remote_path: String,
    remote_dir: String,
    stamp: Option<RemoteStamp>,
    mode: Option<u32>,
    production: bool,
    /// Server the file came from (for the crash-recovery metadata).
    target: Option<RemoteTarget>,
    /// This instance (owner in the metadata); `None` = no metadata is written.
    owner: Option<recover::ProcId>,
    /// Content that is on the server (after download / last upload).
    uploaded: Option<Fingerprint>,
    pub state: EditState,
    debounce: Debounce,
    watcher: Option<FileWatcher>,
    job: Option<u64>,
    /// Finish (delete the temp copy) once the next upload succeeded.
    end_after_upload: bool,
    /// The last upload was cut off by a lost connection: retry once it is back.
    wait_link: bool,
    /// Automatic retries after lost connections (since the last success).
    auto_retries: u32,
    /// What the file is opened with (F4 = the text editor).
    pub opener: Opener,
    /// Opened with another application ("Open with"): ask before uploading a change.
    ask_upload: bool,
    /// A new file (Shift+F4) not on the server yet: it is created by the first upload.
    new_file: bool,
}

pub enum EditMsg {
    Changed(u64),
    /// A local file open in the external editor changed (its folder).
    LocalChanged(String),
    Downloaded { id: u64, result: Result<(RemoteStamp, Option<u32>), String> },
    Preflight {
        id: u64,
        result: Result<RemoteStamp, String>,
        lost: bool,
        /// The file is not on the server.
        missing: bool,
    },
    Uploaded {
        id: u64,
        fp: Option<Fingerprint>,
        /// SHA-256 of the uploaded content (crash-recovery metadata).
        hash: Option<String>,
        aside: Option<String>,
        result: Result<RemoteStamp, String>,
        /// Failed because the connection was lost.
        lost: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AfterUnsaved {
    CloseTab(TabId),
    Quit,
    End,
}

pub enum EditPrompt {
    Large { at: PaneRef, target: Target, entry: Entry, opener: Opener },
    /// A file from a server that could run something (executable, script,
    /// launcher) is about to be opened by an application other than the editor.
    Risky { id: u64, reason: &'static str },
    /// A file opened with another application changed: upload it?
    OfferUpload { id: u64 },
    BinaryLocal { path: String, name: String },
    /// Enter on a local program or script: Run / Open in editor / Cancel.
    LocalExec { path: String, name: String, reason: &'static str, executable: bool },
    BinaryRemote { id: u64 },
    Conflict { id: u64, remote: RemoteStamp },
    Production { id: u64 },
    Unsaved { ids: Vec<u64>, then: AfterUnsaved },
    Editor { cmd: String },
    /// An edit left behind by a crashed instance, with changes not on the server.
    Recovered { orphan: Box<Orphan> },
}

#[derive(Default)]
pub struct EditsState {
    pub sessions: Vec<EditSession>,
    pub prompts: Vec<EditPrompt>,
    pub show_list: bool,
    /// Quit once the uploads marked `end_after_upload` are done.
    pub quit_pending: bool,
    /// Closing the window is allowed (unsaved changes were dealt with).
    pub quit_ok: bool,
    esc: bool,
    /// This instance's marker in the temp directory (removed on exit).
    instance: Option<recover::Instance>,
    /// Crash leftovers to upload once their server is connected.
    adopt: Vec<Orphan>,
    /// Local files opened in the external editor, watched so that the panels
    /// showing their folder follow a save (size, time). The newest last.
    local: Vec<(std::path::PathBuf, FileWatcher)>,
    /// The newest change of each watched folder (a refresh waits for quiet).
    local_gen: std::collections::HashMap<String, u64>,
}

/// How many local files are watched at most (the oldest is dropped).
const LOCAL_WATCH_MAX: usize = 32;
/// The quiet time after a change of a local file before the panels refresh.
const LOCAL_QUIET: std::time::Duration = std::time::Duration::from_millis(300);

fn now_hm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

impl App {
    fn editor_command(&self) -> String {
        self.settings.editor_command.clone().filter(|c| !c.trim().is_empty()).unwrap_or_else(editor::detect_default)
    }

    pub(super) fn launch_editor(&mut self, path: &std::path::Path) -> bool {
        let cmd = self.editor_command();
        match editor::launch(&cmd, path) {
            Ok(()) => {
                tracing::info!("editor started: {}", cmd.split_whitespace().next().unwrap_or(""));
                self.watch_local_edit(path);
                true
            }
            Err(e) => {
                self.last_error = Some(tr!("{error} (Settings → External editor…)", error = e));
                false
            }
        }
    }

    /// Watch a local file opened in the editor (not a temporary copy of a
    /// server file: those have their own session).
    fn watch_local_edit(&mut self, path: &std::path::Path) {
        if path.starts_with(temp::base_dir()) || !path.is_absolute() {
            return;
        }
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return };
        let list = &mut self.edits.local;
        if let Some(i) = list.iter().position(|(p, _)| p == path) {
            let w = list.remove(i);
            list.push(w);
            return;
        }
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let d = dir.to_string_lossy().into_owned();
        match FileWatcher::new(dir, name.to_os_string(), move || {
            let _ = tx.send(Msg::Edit(Box::new(EditMsg::LocalChanged(d.clone()))));
            ctx.request_repaint();
        }) {
            Ok(w) => {
                list.push((path.to_path_buf(), w));
                if list.len() > LOCAL_WATCH_MAX {
                    list.remove(0);
                }
            }
            Err(e) => tracing::warn!("cannot watch an edited local file: {e}"),
        }
    }

    /// A watched local file changed: refresh the panels of its folder once
    /// the writes are over.
    fn on_local_changed(&mut self, dir: String) {
        let g = self.edits.local_gen.entry(dir.clone()).or_default();
        *g += 1;
        let gen = *g;
        self.spawn(async move {
            tokio::time::sleep(LOCAL_QUIET).await;
            Msg::Then {
                job: None,
                f: Box::new(move |a: &mut App| {
                    if a.edits.local_gen.get(&dir) != Some(&gen) {
                        return; // a newer change will refresh
                    }
                    a.edits.local_gen.remove(&dir);
                    let panes: Vec<_> = a
                        .all_panes()
                        .filter(|(_, p)| p.fs.is_local_disk() && p.search.is_none() && p.archive.is_none() && p.path == dir)
                        .map(|(r, _)| r)
                        .collect();
                    for r in panes {
                        a.refresh(r);
                    }
                }),
            }
        });
    }

    fn session(&mut self, id: u64) -> Option<&mut EditSession> {
        self.edits.sessions.iter_mut().find(|s| s.id == id)
    }

    // ------------------------------------------------------- crash recovery

    /// At start-up: register this instance and process leftovers of crashed ones.
    pub(super) fn edits_startup(&mut self) {
        let base = temp::base_dir();
        if !base.exists() {
            return; // nothing was ever edited; the marker is written with the first edit
        }
        self.register_instance(&base);
        let Some(me) = self.edits.instance.as_ref().map(|i| i.id) else { return };
        let scan = recover::scan(&base, &me, recover::is_running);
        if scan.removed > 0 {
            tracing::info!("cleanup after a crash: deleted {} unchanged temporary copies", scan.removed);
        }
        for o in scan.orphans {
            tracing::warn!("an unsaved edit was left after a crash: {}", o.meta.remote_path);
            self.edits.prompts.push(EditPrompt::Recovered { orphan: Box::new(o) });
        }
    }

    fn register_instance(&mut self, base: &std::path::Path) {
        if self.edits.instance.is_some() {
            return;
        }
        match temp::ensure_base(base).and_then(|_| recover::Instance::register(base)) {
            Ok(i) => self.edits.instance = Some(i),
            Err(e) => tracing::warn!("{}: {e}", base.display()),
        }
    }

    /// "Upload" for a crash leftover: continue it as a normal edit on its server
    /// (conflict check and production confirmation as usual).
    fn adopt_orphan(&mut self, o: Orphan) {
        let target = RemoteTarget {
            profile: o.meta.profile.clone(),
            host: o.meta.host.clone(),
            port: o.meta.port,
            user: o.meta.user.clone(),
            key_path: o.meta.key_path.clone(),
        };
        let key = target.key();
        let connected = self
            .all_panes()
            .find(|(_, p)| !p.offline && p.remote.as_ref().is_some_and(|r| r.target.key() == key))
            .map(|(r, _)| r);
        self.edits.adopt.push(o);
        match connected {
            Some(at) => self.adopt_pending(at),
            None => {
                // Open the file's directory on its server in a new tab.
                let dir = rolid_ssh::posix_parent(&self.edits.adopt.last().unwrap().meta.remote_path)
                    .unwrap_or_else(|| "/".into());
                let prof = target.profile.as_ref().and_then(|id| self.profiles.get(id)).cloned();
                let production = prof.as_ref().map(|p| p.production).unwrap_or(self.edits.adopt.last().unwrap().meta.production);
                let name = prof.as_ref().map(|p| p.name.clone());
                let pane = super::session::offline_pane(target, name.clone(), production, dir);
                let side = self.active_side;
                let color = prof.as_ref().and_then(|p| p.color.as_deref()).and_then(rolid_core::color::Rgb::from_hex);
                let at = self.push_tab(side, pane, color, name);
                self.activate_tab(side, self.sides[side].tabs.len() - 1);
                self.connect_pane(at);
            }
        }
    }

    /// A panel got connected: take over waiting crash leftovers for its server.
    pub(super) fn adopt_pending(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        let Some(r) = p.remote.clone() else { return };
        if p.offline {
            return;
        }
        let fs = p.fs.clone();
        let key = r.target.key();
        let (mine, rest): (Vec<Orphan>, Vec<Orphan>) = std::mem::take(&mut self.edits.adopt).into_iter().partition(|o| {
            format!("{}@{}:{}", o.meta.user, o.meta.host, o.meta.port) == key
        });
        self.edits.adopt = rest;
        for o in mine {
            let id = self.id();
            let remote_dir = rolid_ssh::posix_parent(&o.meta.remote_path).unwrap_or_else(|| "/".into());
            self.edits.sessions.push(EditSession {
                id,
                name: o.meta.file.clone(),
                at,
                location: format!("{}@{}:{remote_dir}", o.meta.user, o.meta.host),
                dir: EditDir { dir: o.dir.clone(), file: o.file.clone() },
                fs: fs.clone(),
                remote_path: o.meta.remote_path.clone(),
                remote_dir,
                // No size: a new file (Shift+F4) that never reached the server.
                stamp: o.meta.remote_size.map(|size| RemoteStamp { size, mtime: o.meta.remote_mtime }),
                mode: o.meta.mode,
                production: r.production || o.meta.production,
                target: Some(r.target.clone()),
                owner: self.edits.instance.as_ref().map(|i| i.id),
                uploaded: None, // the local copy differs from the server
                state: EditState::NotUploaded(tr!("recovered after a crash")),
                debounce: Debounce::default(),
                watcher: None,
                job: None,
                end_after_upload: false,
                wait_link: false,
                auto_retries: 0,
                opener: Opener::Editor,
                ask_upload: false,
                new_file: o.meta.remote_size.is_none(),
            });
            if self.start_watch(id) {
                self.preflight(id);
            }
            self.edits.show_list = true;
        }
    }

    // ------------------------------------------------------------ start (F4)

    pub(super) fn start_edit_active(&mut self) {
        let at = self.active_ref();
        self.start_edit(at);
    }

    pub(super) fn start_edit(&mut self, at: PaneRef) {
        if self.refuse_read_only(at, &tr!("A file in an archive cannot be edited (it can be opened with Enter)")) {
            return;
        }
        let Some((target, entry)) = self.pane(at).and_then(|p| p.current_target()) else { return };
        self.edit_target(at, target, entry);
    }

    /// F4 on a given file of the panel `at` (also from the viewer).
    pub(super) fn edit_target(&mut self, at: PaneRef, target: Target, entry: Entry) {
        if entry.is_dir_like() {
            self.last_error = Some(tr!("F4 edits files – it does nothing on a folder."));
            return;
        }
        if entry.size > LARGE_FILE {
            self.edits.prompts.push(EditPrompt::Large { at, target, entry, opener: Opener::Editor });
            return;
        }
        self.open_edit(at, target, entry, Opener::Editor);
    }

    /// Open the file under the cursor with `opener` ("Open", "Open with").
    /// Local files directly; remote ones through a private temporary copy as
    /// with F4 (a change is offered for upload).
    pub(super) fn open_with(&mut self, at: PaneRef, opener: Opener) {
        let Some((target, entry)) = self.pane(at).and_then(|p| p.current_target()) else { return };
        if entry.is_dir_like() {
            self.activate_entry(at);
            return;
        }
        let remote = self.pane(at).is_some_and(|p| p.fs.is_remote());
        if remote && entry.size > LARGE_FILE {
            self.edits.prompts.push(EditPrompt::Large { at, target, entry, opener });
            return;
        }
        self.open_edit(at, target, entry, opener);
    }

    /// "Run" a local program (in its own folder), or open a script that is
    /// not executable with the system's default application.
    pub(super) fn run_local(&mut self, path: &std::path::Path, executable: bool) {
        if !executable {
            self.launch_with(&Opener::System, path);
            return;
        }
        let mut cmd = std::process::Command::new(path);
        if let Some(d) = path.parent() {
            cmd.current_dir(d);
        }
        match cmd.stdin(std::process::Stdio::null()).spawn() {
            Ok(mut child) => {
                tracing::info!("program started by the user");
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => self.last_error = Some(tr!("{path}: cannot be started: {error}", path = path, error = e.to_string())),
        }
    }

    /// Start `opener` on a local file.
    pub(super) fn launch_with(&mut self, opener: &Opener, path: &std::path::Path) -> bool {
        let r = match opener {
            Opener::Editor => return self.launch_editor(path),
            Opener::System => apps::spawn(&apps::system_open_argv(path)),
            Opener::Desktop(id) => match AppDb::system().app(id) {
                Some(a) => apps::exec_argv(&a.exec, path).and_then(|v| apps::spawn(&v)),
                None => Err(txt!("application “{id}” is not in the system", id = id.as_str())),
            },
            Opener::Command(c) => editor::build(c, path).and_then(|(p, a)| {
                let mut v = vec![p];
                v.extend(a);
                apps::spawn(&v)
            }),
            Opener::WindowsChooser => apps::spawn(&apps::windows_chooser_argv(path)),
        };
        match r {
            Ok(()) => {
                tracing::info!("file opened ({})", opener.describe());
                true
            }
            Err(e) => {
                self.last_error = Some(rolid_i18n::t(&e));
                false
            }
        }
    }

    fn open_edit(&mut self, at: PaneRef, target: Target, entry: Entry, opener: Opener) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let path = fs.join(&target.dir, &target.name);
        if fs.read_only() {
            // Inside an archive: a temporary read-only copy is opened.
            if opener == Opener::Editor {
                self.refuse_read_only(at, &tr!("A file in an archive cannot be edited"));
            } else {
                self.open_from_archive(at, target, opener);
            }
            return;
        }
        if fs.is_local_disk() && opener != Opener::Editor {
            self.launch_with(&opener, std::path::Path::new(&path));
            return;
        }
        if fs.is_local_disk() {
            // Local file: open it directly, no copy.
            match rolid_edit::file_looks_binary(std::path::Path::new(&path)) {
                Ok(true) => self.edits.prompts.push(EditPrompt::BinaryLocal { path, name: target.name }),
                Ok(false) => {
                    self.launch_editor(std::path::Path::new(&path));
                }
                Err(e) => self.last_error = Some(format!("{path}: {e}")),
            }
            return;
        }
        // The same remote file is already being edited: just show the editor again.
        if let Some(s) = self.edits.sessions.iter().find(|s| s.remote_path == path && Arc::ptr_eq(&s.fs, &fs)) {
            let (file, id) = (s.dir.file.clone(), s.id);
            if opener != Opener::Editor {
                if let Some(s) = self.session(id) {
                    s.ask_upload = true;
                }
            }
            self.launch_with(&opener, &file);
            return;
        }
        let (label, production, rtarget) = match &p.remote {
            Some(r) => (format!("{}@{}", r.target.user, r.target.host), r.production, Some(r.target.clone())),
            None => (crate::pane::fs_label(&*fs), false, None),
        };
        let dir = match temp::create(&temp::base_dir(), &target.name) {
            Ok(d) => {
                self.register_instance(&temp::base_dir());
                d
            }
            Err(e) => {
                self.last_error = Some(tr!("Cannot start editing: {error}", error = e));
                return;
            }
        };
        let id = self.id();
        let local_path = dir.file.to_string_lossy().into_owned();
        self.edits.sessions.push(EditSession {
            id,
            name: target.name.clone(),
            at,
            location: format!("{label}:{}", target.dir),
            dir,
            fs: fs.clone(),
            remote_path: path.clone(),
            remote_dir: target.dir.clone(),
            stamp: None,
            mode: entry.mode,
            production,
            target: rtarget,
            owner: self.edits.instance.as_ref().map(|i| i.id),
            uploaded: None,
            state: EditState::Downloading,
            debounce: Debounce::default(),
            watcher: None,
            job: None,
            end_after_upload: false,
            wait_link: false,
            auto_retries: 0,
            ask_upload: opener != Opener::Editor,
            opener,
            new_file: false,
        });
        let job = self.transfer(format!("✎ {} ← {label}", target.name), entry.size, move |ctl| async move {
            let result = async {
                let st = fs.stat(&path).await.map_err(|e| err_text(&e))?;
                transfer::download(fs.as_ref(), &path, &LocalFs, &local_path, &ctl.ctl())
                    .await
                    .map_err(|e| err_text(&e))?;
                Ok((RemoteStamp::from(&st), st.mode))
            }
            .await;
            Msg::Edit(Box::new(EditMsg::Downloaded { id, result }))
        });
        if let Some(s) = self.session(id) {
            s.job = Some(job);
        }
    }

    /// Shift+F4 on a server: a new, empty file is opened in the editor from a
    /// private temporary folder. Nothing is created on the server until the
    /// first save (then uploaded as with F4, confirmed on production servers);
    /// closing the editor without saving leaves the server untouched.
    pub(super) fn open_new_remote(&mut self, at: PaneRef, dir: String, name: String) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let path = fs.join(&dir, &name);
        let (label, production, rtarget) = match &p.remote {
            Some(r) => (format!("{}@{}", r.target.user, r.target.host), r.production, Some(r.target.clone())),
            None => (crate::pane::fs_label(&*fs), false, None),
        };
        let edir = match temp::create(&temp::base_dir(), &name) {
            Ok(d) => {
                self.register_instance(&temp::base_dir());
                d
            }
            Err(e) => {
                self.last_error = Some(tr!("Cannot start editing: {error}", error = e));
                return;
            }
        };
        if let Err(e) = create_private_file(&edir.file) {
            self.last_error = Some(tr!("Cannot start editing: {error}", error = e.to_string()));
            let _ = std::fs::remove_dir_all(&edir.dir);
            return;
        }
        let id = self.id();
        self.edits.sessions.push(EditSession {
            id,
            name: name.clone(),
            at,
            location: format!("{label}:{dir}"),
            dir: edir,
            fs,
            remote_path: path,
            remote_dir: dir,
            stamp: None,
            mode: None,
            production,
            target: rtarget,
            owner: self.edits.instance.as_ref().map(|i| i.id),
            // Unknown on the server: the first save is uploaded even if empty.
            uploaded: None,
            state: EditState::Watching,
            debounce: Debounce::default(),
            watcher: None,
            job: None,
            end_after_upload: false,
            wait_link: false,
            auto_retries: 0,
            opener: Opener::Editor,
            ask_upload: false,
            new_file: true,
        });
        if let Some(s) = self.session(id) {
            let hash = recover::hash_file(&s.dir.file).ok();
            Self::write_meta(s, hash);
        }
        tracing::info!("new file on a server: opened in the editor, created on the first save");
        self.watch_and_open(id);
    }

    /// Run a transfer through the queue (shown with progress, limited by the
    /// shared speed limit). Returns the job id.
    fn transfer<F, Fut>(&mut self, title: String, total: u64, work: F) -> u64
    where
        F: FnOnce(TransferCtl) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Msg> + Send + 'static,
    {
        let id = self.id();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.push(Job {
            id,
            title,
            progress: progress.clone(),
            total_bytes: total,
            cancel: cancel.clone(),
            note: Arc::default(),
            count: None,
            // One file.
            totals: Arc::new(Mutex::new(rolid_core::copy::Totals { files: 1, bytes: total, done: true })),
        });
        let slots = self.transfer_slots.clone();
        let limit = self.rate_limit.clone();
        let ctx = self.ctx.clone();
        self.spawn(async move {
            let _permit = loop {
                if let Ok(p) = slots.clone().try_acquire_owned() {
                    break Some(p);
                }
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break None; // the work sees the cancel flag and stops at once
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            progress.lock().unwrap().started = Some(Instant::now());
            let p2 = progress.clone();
            let cb: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |n| {
                p2.lock().unwrap().bytes_done = n;
                ctx.request_repaint_after(Duration::from_millis(100));
            });
            work(TransferCtl { limit, cancel, cb }).await
        });
        id
    }

    // ------------------------------------------------------------- messages

    pub(super) fn on_edit_msg(&mut self, m: EditMsg) {
        match m {
            EditMsg::LocalChanged(dir) => self.on_local_changed(dir),
            EditMsg::Changed(id) => {
                if let Some(s) = self.session(id) {
                    s.debounce.event(Instant::now());
                }
            }
            EditMsg::Downloaded { id, result } => {
                let Some(s) = self.session(id) else { return };
                let job = s.job.take();
                match result {
                    Ok((stamp, mode)) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&s.dir.file, std::fs::Permissions::from_mode(0o600));
                        }
                        s.stamp = Some(stamp);
                        s.mode = mode.or(s.mode);
                        s.uploaded = fingerprint(&s.dir.file).ok();
                        let hash = recover::hash_file(&s.dir.file).ok();
                        Self::write_meta(s, hash);
                        if s.opener == Opener::Editor {
                            let binary = rolid_edit::file_looks_binary(&s.dir.file).unwrap_or(false);
                            if binary {
                                self.edits.prompts.push(EditPrompt::BinaryRemote { id });
                            } else {
                                self.watch_and_open(id);
                            }
                        } else {
                            // Never let another application run a file from a server.
                            let head = read_head(&s.dir.file);
                            match apps::risky(&s.name, s.mode, &head) {
                                Some(reason) => self.edits.prompts.push(EditPrompt::Risky { id, reason }),
                                None => self.watch_and_open(id),
                            }
                        }
                    }
                    Err(e) => {
                        self.last_error = Some(tr!("Cannot download the file for editing: {error}", error = e));
                        self.end_edit(id);
                    }
                }
                self.jobs.retain(|j| Some(j.id) != job);
            }
            EditMsg::Preflight { id, result, lost, missing } => {
                let Some(s) = self.session(id) else { return };
                match preflight_next(s.new_file, s.production, s.stamp, &result, lost, missing) {
                    Next::WaitLink => {
                        s.state = EditState::Failed(lost_text());
                        s.wait_link = true;
                        self.edit_failed_during_quit();
                    }
                    Next::Conflict(now) => {
                        s.state = EditState::NotUploaded(if s.new_file {
                            tr!("a file of the same name was created on the server in the meantime")
                        } else {
                            tr!("the file has changed on the server in the meantime")
                        });
                        self.edits.prompts.push(EditPrompt::Conflict { id, remote: now });
                    }
                    Next::Confirm => {
                        s.state = EditState::NotUploaded(tr!("waiting for confirmation (production server)"));
                        self.edits.prompts.push(EditPrompt::Production { id });
                    }
                    Next::Upload => self.upload(id, None),
                    Next::Fail => {
                        let e = result.err().unwrap_or_default();
                        s.state = EditState::Failed(tr!("Cannot find out the state of the file on the server: {error}", error = e));
                        self.edit_failed_during_quit();
                    }
                }
            }
            EditMsg::Uploaded { id, fp, hash, aside, result, lost } => {
                let Some(s) = self.session(id) else { return };
                let job = s.job.take();
                match result {
                    Ok(stamp) => {
                        s.uploaded = fp;
                        s.auto_retries = 0;
                        // The new file now exists: the panel's cursor goes to it.
                        let created = (s.new_file && aside.is_none()).then(|| (s.at, s.remote_dir.clone(), s.name.clone()));
                        if aside.is_none() {
                            s.stamp = Some(stamp);
                            s.new_file = false;
                            Self::write_meta(s, hash);
                        }
                        s.state = EditState::Uploaded { time: now_hm(), aside: aside.clone() };
                        // Changes saved during the upload are picked up by the next check.
                        s.debounce.event(Instant::now());
                        tracing::info!("edit uploaded to the server");
                        let (at, end) = (s.at, s.end_after_upload && aside.is_none());
                        for r in self.same_location(at) {
                            match &created {
                                Some((cat, dir, name)) if *cat == r && self.pane(r).is_some_and(|p| p.path == *dir && p.search.is_none()) => {
                                    self.load(r, dir.clone(), Some(name.clone()));
                                }
                                _ => self.refresh(r),
                            }
                        }
                        if end {
                            self.end_edit(id);
                        }
                        self.maybe_quit();
                    }
                    Err(e) if lost => {
                        tracing::warn!("edit upload interrupted by a lost connection: {e}");
                        s.state = EditState::Failed(lost_text());
                        s.wait_link = true;
                        self.edit_failed_during_quit();
                    }
                    Err(e) => {
                        tracing::warn!("edit upload failed: {e}");
                        s.state = EditState::Failed(tr!("Upload failed: {error}", error = e));
                        self.edit_failed_during_quit();
                    }
                }
                self.jobs.retain(|j| Some(j.id) != job);
            }
        }
    }

    fn watch_and_open(&mut self, id: u64) {
        if self.start_watch(id) {
            if let Some((file, opener)) = self.session(id).map(|s| (s.dir.file.clone(), s.opener.clone())) {
                self.launch_with(&opener, &file);
            }
        }
    }

    /// Watch the edit's directory for saves. False if that is not possible.
    fn start_watch(&mut self, id: u64) -> bool {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let Some(s) = self.session(id) else { return false };
        let name = s.dir.file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        match FileWatcher::new(&s.dir.dir, name, move || {
            let _ = tx.send(Msg::Edit(Box::new(EditMsg::Changed(id))));
            ctx.request_repaint();
        }) {
            Ok(w) => s.watcher = Some(w),
            Err(e) => {
                s.state = EditState::Failed(tr!("Cannot watch the file for changes: {error}", error = e.to_string()));
                return false;
            }
        }
        s.state = EditState::Watching;
        true
    }

    /// An item was renamed or moved from `old` to `new` on backend `key`
    /// (`"local"` or the server's `user@host:port`): files being edited in it
    /// are uploaded to the new place from now on. Returns warnings for files
    /// whose upload was running at that moment.
    pub(super) fn edits_relocate(&mut self, key: &str, old: &str, new: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        for s in &mut self.edits.sessions {
            let k = match &s.target {
                Some(t) => t.key(),
                None if s.fs.is_local_disk() => "local".to_string(),
                None => continue,
            };
            if k != key {
                continue;
            }
            let Some(path) = rolid_core::rename::moved_path(&s.remote_path, old, new) else { continue };
            if s.job.is_some() {
                warnings.push(tr!(
                    "The edited file “{name}” was just being uploaded to its original location. The next save will go to {path}; check that the newest content is there.",
                    name = s.name.as_str(),
                    path = path.as_str()
                ));
            }
            tracing::info!("edited file {} was moved: {path}", s.remote_path);
            s.remote_dir = s.fs.parent(&path).unwrap_or_else(|| "/".into());
            if let Some(i) = s.location.find(":/") {
                s.location = format!("{}:{}", &s.location[..i], s.remote_dir);
            }
            s.remote_path = path.clone();
            // Crash recovery uploads to the new place as well.
            if let Ok(mut m) = recover::read_meta(&s.dir.dir) {
                m.remote_path = path;
                if let Err(e) = recover::write_meta(&s.dir.dir, &m) {
                    tracing::warn!("cannot write the edit metadata: {e}");
                }
            }
        }
        warnings
    }

    /// Crash-recovery metadata next to the edit directory: owner, origin and
    /// `hash` of the content now on the server.
    fn write_meta(s: &EditSession, hash: Option<String>) {
        let (Some(t), Some(instance), Some(hash)) = (&s.target, s.owner, hash) else { return };
        let meta = EditMeta {
            owner: instance,
            profile: t.profile.clone(),
            host: t.host.clone(),
            port: t.port,
            user: t.user.clone(),
            key_path: t.key_path.clone(),
            remote_path: s.remote_path.clone(),
            file: s.name.clone(),
            remote_size: s.stamp.map(|st| st.size),
            remote_mtime: s.stamp.and_then(|st| st.mtime),
            mode: s.mode,
            production: s.production,
            hash,
        };
        if let Err(e) = recover::write_meta(&s.dir.dir, &meta) {
            tracing::warn!("cannot write the edit metadata: {e}");
        }
    }

    // --------------------------------------------------------------- upload

    /// Per frame: run due debounce checks.
    pub(super) fn edit_frame(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut due = Vec::new();
        for s in &mut self.edits.sessions {
            if s.debounce.fire(now) {
                due.push(s.id);
            } else if let Some(r) = s.debounce.remaining(now) {
                ctx.request_repaint_after(r + Duration::from_millis(10));
            }
        }
        for id in due {
            self.check_saved(id);
        }
        self.edits.esc = if self.edits.prompts.is_empty() {
            false
        } else {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        };
        // Window close: ask about changes that are not on the server yet.
        if ctx.input(|i| i.viewport().close_requested()) && !self.edits.quit_ok {
            let dirty = self.dirty_edits(|_| true);
            if self.edits.quit_pending || !dirty.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !self.edits.quit_pending && !self.has_unsaved_prompt() {
                    self.edits.prompts.push(EditPrompt::Unsaved { ids: dirty, then: AfterUnsaved::Quit });
                }
            }
        }
    }

    fn has_unsaved_prompt(&self) -> bool {
        self.edits.prompts.iter().any(|p| matches!(p, EditPrompt::Unsaved { .. }))
    }

    fn prompt_open_for(&self, id: u64) -> bool {
        self.edits.prompts.iter().any(|p| match p {
            EditPrompt::Conflict { id: i, .. }
            | EditPrompt::Production { id: i }
            | EditPrompt::BinaryRemote { id: i }
            | EditPrompt::Risky { id: i, .. }
            | EditPrompt::OfferUpload { id: i } => *i == id,
            _ => false,
        })
    }

    /// The editor saved (debounced): upload if the content really changed.
    fn check_saved(&mut self, id: u64) {
        let busy = self.prompt_open_for(id);
        let Some(s) = self.session(id) else { return };
        if busy || matches!(s.state, EditState::Downloading | EditState::Checking | EditState::Uploading) {
            return; // the running step re-checks when it finishes
        }
        let Ok(fp) = fingerprint(&s.dir.file) else {
            // Mid atomic save (file briefly missing): look again shortly.
            s.debounce.event(Instant::now());
            return;
        };
        if s.uploaded.as_ref().is_none_or(|u| fp.content_differs(u)) {
            if s.ask_upload {
                // Opened with another application: the change is offered, not uploaded silently.
                s.state = EditState::NotUploaded(tr!("changed – waiting for the decision about uploading"));
                self.edits.prompts.push(EditPrompt::OfferUpload { id });
            } else {
                self.preflight(id);
            }
        }
    }

    /// Check the remote file (conflict) before uploading.
    fn preflight(&mut self, id: u64) {
        let Some(s) = self.session(id) else { return };
        s.state = EditState::Checking;
        let (fs, path) = (s.fs.clone(), s.remote_path.clone());
        self.spawn(async move {
            let r = fs.stat(&path).await;
            let lost = r.as_ref().is_err_and(|e| e.is_connection_lost());
            let missing = r.as_ref().is_err_and(|e| e.is_not_found());
            let result = r.map(|e| RemoteStamp::from(&e)).map_err(|e| err_text(&e));
            Msg::Edit(Box::new(EditMsg::Preflight { id, result, lost, missing }))
        });
    }

    /// Upload the local copy (to the original name, or `aside` next to it).
    fn upload(&mut self, id: u64, aside: Option<String>) {
        let Some(s) = self.session(id) else { return };
        let fp = fingerprint(&s.dir.file).ok();
        let hash = recover::hash_file(&s.dir.file).ok();
        s.state = EditState::Uploading;
        let fs = s.fs.clone();
        let dst = match &aside {
            Some(n) => fs.join(&s.remote_dir, n),
            None => s.remote_path.clone(),
        };
        let local = s.dir.file.to_string_lossy().into_owned();
        let mode = s.mode;
        let size = fp.as_ref().map(|f| f.size).unwrap_or(0);
        let title = format!("✎ {} → {}", s.name, s.location.split(':').next().unwrap_or(""));
        let job = self.transfer(title, size, move |ctl| async move {
            let result = async {
                transfer::upload_replace(&LocalFs, &local, fs.as_ref(), &dst, mode, &ctl.ctl()).await?;
                let st = fs.stat(&dst).await?;
                Ok::<_, rolid_core::VfsError>(RemoteStamp::from(&st))
            }
            .await;
            let lost = result.as_ref().is_err_and(|e| e.is_connection_lost());
            let result = result.map_err(|e| err_text(&e));
            Msg::Edit(Box::new(EditMsg::Uploaded { id, fp, hash, aside, result, lost }))
        });
        if let Some(s) = self.session(id) {
            s.job = Some(job);
        }
    }

    fn retry(&mut self, id: u64) {
        if let Some(s) = self.session(id) {
            s.wait_link = false;
            s.auto_retries = 0;
        }
        self.preflight(id);
    }

    pub(super) fn edits_use_link(&self, key: &str) -> bool {
        self.edits.sessions.iter().any(|s| s.target.as_ref().is_some_and(|t| t.key() == key))
    }

    /// The connection to `key` is back: upload edits that were cut off – if they
    /// still have changes that are not on the server (conflict check and
    /// production confirmation as always).
    pub(super) fn edits_link_restored(&mut self, key: &str) {
        let ids: Vec<u64> = self
            .edits
            .sessions
            .iter()
            .filter(|s| s.wait_link && s.target.as_ref().is_some_and(|t| t.key() == key))
            .map(|s| s.id)
            .collect();
        for id in ids {
            let dirty = !self.dirty_edits(|s| s.id == id).is_empty();
            let Some(s) = self.session(id) else { continue };
            s.wait_link = false;
            if !dirty {
                s.state = EditState::Watching;
                continue;
            }
            if s.auto_retries >= rolid_core::copy::MAX_AUTO_RETRIES {
                s.state = EditState::Failed(trn!(
                    "The upload did not succeed even after {n} restored connection – try “Upload again”",
                    "The upload did not succeed even after {n} restored connections – try “Upload again”",
                    rolid_core::copy::MAX_AUTO_RETRIES
                ));
                continue;
            }
            s.auto_retries += 1;
            tracing::info!("connection restored: uploading the edit again");
            self.preflight(id);
        }
    }

    // ---------------------------------------------------------- ending edits

    /// Not on the server yet: uploading, waiting, failed, or saved since the last upload.
    fn is_dirty(s: &EditSession) -> bool {
        match s.state {
            EditState::Downloading => false,
            EditState::Checking | EditState::Uploading | EditState::NotUploaded(_) | EditState::Failed(_) => {
                s.uploaded.as_ref().is_none_or(|u| fingerprint(&s.dir.file).map(|f| f.content_differs(u)).unwrap_or(true))
            }
            _ => match (&s.uploaded, fingerprint(&s.dir.file)) {
                (Some(u), Ok(f)) => f.content_differs(u),
                _ => false,
            },
        }
    }

    fn dirty_edits(&self, filter: impl Fn(&EditSession) -> bool) -> Vec<u64> {
        self.edits.sessions.iter().filter(|s| filter(s) && Self::is_dirty(s)).map(|s| s.id).collect()
    }

    /// Stop editing: stop watching, delete the temporary copy.
    pub(super) fn end_edit(&mut self, id: u64) {
        if let Some(i) = self.edits.sessions.iter().position(|s| s.id == id) {
            let s = self.edits.sessions.remove(i);
            if let Some(j) = s.job {
                if let Some(job) = self.jobs.iter().find(|x| x.id == j) {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            drop(s.watcher);
            if let Err(e) = s.dir.remove() {
                tracing::warn!("cannot delete the temporary copy: {e}");
            }
        }
        self.edits.prompts.retain(|p| match p {
            EditPrompt::Conflict { id: i, .. }
            | EditPrompt::Production { id: i }
            | EditPrompt::BinaryRemote { id: i }
            | EditPrompt::Risky { id: i, .. }
            | EditPrompt::OfferUpload { id: i } => *i != id,
            _ => true,
        });
    }

    fn request_end(&mut self, id: u64) {
        if self.dirty_edits(|s| s.id == id).is_empty() {
            self.end_edit(id);
        } else {
            self.edits.prompts.push(EditPrompt::Unsaved { ids: vec![id], then: AfterUnsaved::End });
        }
    }

    /// Before closing a tab: true = wait (asked the user about unsaved changes).
    pub(super) fn edits_block_tab_close(&mut self, tab: TabId) -> bool {
        let dirty = self.dirty_edits(|s| s.at.tab == tab && !s.end_after_upload);
        if !dirty.is_empty() {
            if !self.has_unsaved_prompt() {
                self.edits.prompts.push(EditPrompt::Unsaved { ids: dirty, then: AfterUnsaved::CloseTab(tab) });
            }
            return true;
        }
        let ids: Vec<u64> =
            self.edits.sessions.iter().filter(|s| s.at.tab == tab && !s.end_after_upload).map(|s| s.id).collect();
        for id in ids {
            self.end_edit(id);
        }
        false
    }

    /// App exit: delete all temporary copies.
    pub(super) fn end_all_edits(&mut self) {
        let ids: Vec<u64> = self.edits.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            self.end_edit(id);
        }
        // Leftovers the user did not decide about stay for the next start (their
        // metadata names this instance, which is no longer running then).
        self.edits.instance = None;
    }

    fn maybe_quit(&mut self) {
        if self.edits.quit_pending && !self.edits.sessions.iter().any(|s| s.end_after_upload) {
            self.edits.quit_pending = false;
            self.edits.quit_ok = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn edit_failed_during_quit(&mut self) {
        if self.edits.quit_pending {
            self.edits.quit_pending = false;
            self.last_error = Some(tr!("Quitting canceled: the edit could not be uploaded (see Edited files)."));
            self.edits.show_list = true;
        }
    }

    fn close_tab_by_id(&mut self, tab: TabId) {
        for side in 0..2 {
            if let Some(i) = self.sides[side].tabs.iter().position(|t| t.id == tab) {
                self.close_tab(side, i);
                return;
            }
        }
    }

    fn after_unsaved(&mut self, ids: Vec<u64>, then: AfterUnsaved, upload: bool) {
        for id in &ids {
            if upload {
                if let Some(s) = self.session(*id) {
                    s.end_after_upload = true;
                }
                self.preflight(*id);
            } else {
                self.end_edit(*id);
            }
        }
        match then {
            AfterUnsaved::End => {}
            AfterUnsaved::CloseTab(tab) => {
                // Uploads continue in the background; then the copies are deleted.
                self.close_tab_by_id(tab);
            }
            AfterUnsaved::Quit if upload => {
                self.edits.quit_pending = true;
                self.maybe_quit();
            }
            AfterUnsaved::Quit => {
                self.edits.quit_ok = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(super) fn ask_editor_command(&mut self) {
        let cmd = self.settings.editor_command.clone().unwrap_or_default();
        self.edits.prompts.push(EditPrompt::Editor { cmd });
    }

    // ------------------------------------------------------------------- UI

    pub(super) fn edits_blocks_input(&self) -> bool {
        !self.edits.prompts.is_empty()
    }

    /// Status line indicator (own row above the function keys). Returns true when clicked.
    pub(super) fn edit_indicator(&self, ui: &mut egui::Ui, stroke: egui::Stroke) -> bool {
        let ss = &self.edits.sessions;
        if ss.is_empty() {
            return false;
        }
        let (text, color) = if ss.iter().any(|s| s.state == EditState::Uploading) {
            (tr!("⏫ Uploading…"), ORANGE)
        } else if ss.iter().any(|s| s.state.needs_attention()) {
            (trn!("⚠ Edits: {n} file – not uploaded", "⚠ Edits: {n} files – not uploaded", ss.len()), RED)
        } else {
            let last = ss.iter().rev().find_map(|s| match &s.state {
                EditState::Uploaded { time, aside: None } => Some(time.clone()),
                _ => None,
            });
            let t = match last {
                Some(time) => trn!(
                    "✎ Editing: {n} file · uploaded at {time}",
                    "✎ Editing: {n} files · uploaded at {time}",
                    ss.len(),
                    time = time
                ),
                None => trn!("✎ Editing: {n} file", "✎ Editing: {n} files", ss.len()),
            };
            (t, ui.visuals().text_color())
        };
        let b = egui::Button::new(RichText::new(text).size(crate::fonts::fs(12.5)).color(color))
            .stroke(stroke)
            .min_size(egui::Vec2::new(0.0, 24.0));
        ui.add(b).on_hover_text(tr!("Edited files (F4) – list, upload again, end")).clicked()
    }

    pub(super) fn edits_list_ui(&mut self, ctx: &egui::Context) {
        if !self.edits.show_list {
            return;
        }
        let mut open = true;
        let mut act: Option<(u64, u8)> = None; // 0 = editor, 1 = retry, 2 = end
        egui::Window::new(tr!("Edited files"))
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .resizable(false)
            .show(ctx, |ui| {
                if self.edits.sessions.is_empty() {
                    ui.weak(tr!("No file is being edited. Select a file and press F4."));
                }
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for s in &self.edits.sessions {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(540.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&s.name).strong());
                                let (t, c) = s.state.text();
                                ui.add(egui::Label::new(RichText::new(t).color(c)).truncate());
                            });
                            ui.add(egui::Label::new(RichText::new(&s.location).weak().size(crate::fonts::fs(11.5))).truncate())
                                .on_hover_text(&s.location);
                            ui.horizontal(|ui| {
                                if crate::look::btn(crate::look::Role::Edit, tr!("Open in editor")).ui(ui).clicked() {
                                    act = Some((s.id, 0));
                                }
                                if s.state.needs_attention() && crate::look::btn(crate::look::Role::Upload, tr!("Upload again")).ui(ui).clicked() {
                                    act = Some((s.id, 1));
                                }
                                if crate::look::btn(crate::look::Role::Close, tr!("End editing")).ui(ui).clicked() {
                                    act = Some((s.id, 2));
                                }
                            });
                        });
                    }
                });
                ui.add_space(4.0);
                ui.weak(tr!("Every save in the editor is uploaded to the server automatically. Ending deletes the temporary copy."));
            });
        self.edits.show_list = open;
        match act {
            Some((id, 0)) => {
                if let Some(f) = self.edits.sessions.iter().find(|s| s.id == id).map(|s| s.dir.file.clone()) {
                    self.launch_editor(&f);
                }
            }
            Some((id, 1)) => self.retry(id),
            Some((id, _)) => self.request_end(id),
            None => {}
        }
    }

    pub(super) fn edit_prompt_ui(&mut self, ctx: &egui::Context) {
        if self.edits.prompts.is_empty() {
            return;
        }
        let esc = self.edits.esc;
        let mut p = self.edits.prompts.remove(0);
        let mut done = false;
        let mut act: Option<super::Action> = None;
        let name_of = |a: &App, id: u64| {
            a.edits.sessions.iter().find(|s| s.id == id).map(|s| (s.name.clone(), s.location.clone())).unwrap_or_default()
        };
        // Esc was consumed for the prompts (see `edit_frame`), so it is
        // passed on here together with the ✕ of the dialog.
        let win = |title: String, role: Role| dialog::window("edit-prompt", title).icon(role).max_width(520.0);
        match &mut p {
            EditPrompt::Large { at, target, entry, opener } => {
                let remote = self.pane(*at).is_some_and(|p| p.fs.is_remote());
                win(tr!("Large file"), Role::Edit).show(ctx, |ui| {
                    let (name, size) = (target.name.as_str(), rolid_i18n::size(entry.size));
                    dialog::lead(ui, if remote {
                        tr!(
                            "“{name}” is {size}. Editing a large file in the editor may be slow and the download will take a while.",
                            name = name,
                            size = size
                        )
                    } else {
                        tr!("“{name}” is {size}. Editing a large file in the editor may be slow.", name = name, size = size)
                    });
                    let b = dialog::buttons(ui, [dialog::btn(Role::Edit, tr!("Edit anyway")), dialog::cancel()]);
                    if b[0] {
                        let (at, t, e, o) = (*at, target.clone(), entry.clone(), opener.clone());
                        act = Some(Box::new(move |a: &mut App| a.open_edit(at, t, e, o)));
                        done = true;
                    }
                    if b[1] || esc {
                        done = true;
                    }
                });
            }
            EditPrompt::LocalExec { path, name, reason, executable } => {
                win(tr!("Run the program?"), Role::Overwrite).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        dialog::lead(
                            ui,
                            tr!("“{name}” {reason}. Enter does not run it by itself.", name = name.as_str(), reason = rolid_i18n::t_str(reason)),
                        );
                        dialog::help(ui, tr!("Run it only if you know what it does. You can look at it safely in the text editor."));
                    });
                    let run = if *executable { tr!("Run") } else { tr!("Open with the default application") };
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Edit, tr!("Open in editor")).default(true),
                        dialog::btn(Role::Overwrite, run),
                        dialog::cancel(),
                    ]);
                    if b[0] {
                        let p = path.clone();
                        act = Some(Box::new(move |a: &mut App| {
                            a.launch_editor(std::path::Path::new(&p));
                        }));
                        done = true;
                    } else if b[1] {
                        let (p, executable) = (path.clone(), *executable);
                        act = Some(Box::new(move |a: &mut App| a.run_local(std::path::Path::new(&p), executable)));
                        done = true;
                    }
                    if b[2] || esc {
                        done = true;
                    }
                });
            }
            EditPrompt::BinaryLocal { path, name } => {
                win(tr!("Binary file"), Role::Open).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!("“{name}” looks like a binary file (it contains zero bytes). The text editor may damage it.", name = name.as_str()),
                    );
                    let b = dialog::buttons(ui, [dialog::btn(Role::Open, tr!("Open anyway")), dialog::cancel()]);
                    if b[0] {
                        let path = path.clone();
                        act = Some(Box::new(move |a: &mut App| {
                            a.launch_editor(std::path::Path::new(&path));
                        }));
                        done = true;
                    }
                    if b[1] || esc {
                        done = true;
                    }
                });
            }
            EditPrompt::BinaryRemote { id } => {
                let (n, _) = name_of(self, *id);
                let id = *id;
                win(tr!("Binary file"), Role::Open).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!(
                            "“{name}” looks like a binary file (it contains zero bytes). The text editor may \
                             damage it and saving would upload it to the server.",
                            name = n.as_str()
                        ),
                    );
                    let b = dialog::buttons(ui, [dialog::btn(Role::Open, tr!("Open anyway")), dialog::cancel()]);
                    if b[0] {
                        act = Some(Box::new(move |a: &mut App| a.watch_and_open(id)));
                        done = true;
                    } else if b[1] || esc {
                        act = Some(Box::new(move |a: &mut App| a.end_edit(id)));
                        done = true;
                    }
                });
            }
            EditPrompt::Risky { id, reason } => {
                let id = *id;
                let (n, loc) = name_of(self, id);
                let how = self.edits.sessions.iter().find(|s| s.id == id).map(|s| s.opener.describe()).unwrap_or_default();
                win(tr!("⚠ A file from the server can run something"), Role::Overwrite).danger(true).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!("“{name}” ({location}) {reason}.", name = n.as_str(), location = loc.as_str(), reason = rolid_i18n::t_str(reason)),
                    );
                    dialog::warn(
                        ui,
                        tr!(
                            "Opening it with an application other than the text editor may run its content \
                             on this computer. It is safe to look at it in the text editor."
                        ),
                        false,
                    );
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Edit, tr!("Open in editor")).default(true),
                        dialog::btn(Role::Overwrite, tr!("Open anyway ({app})", app = how)),
                        dialog::cancel(),
                    ]);
                    if b[0] {
                        act = Some(Box::new(move |a: &mut App| {
                            if let Some(s) = a.session(id) {
                                s.opener = Opener::Editor;
                            }
                            a.watch_and_open(id);
                        }));
                        done = true;
                    } else if b[1] {
                        act = Some(Box::new(move |a: &mut App| a.watch_and_open(id)));
                        done = true;
                    } else if b[2] || esc {
                        act = Some(Box::new(move |a: &mut App| a.end_edit(id)));
                        done = true;
                    }
                });
            }
            EditPrompt::OfferUpload { id } => {
                let id = *id;
                let (n, loc) = name_of(self, id);
                let prod = self.edits.sessions.iter().any(|s| s.id == id && s.production);
                win(tr!("The file has changed"), Role::Upload).danger(prod).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        dialog::lead(
                            ui,
                            tr!(
                                "“{name}” opened from {location} has changed in the application. Upload the change to the server?",
                                name = n.as_str(),
                                location = loc.as_str()
                            ),
                        );
                        dialog::help(ui, tr!("Before uploading, it is checked whether the file has changed on the server in the meantime."));
                    });
                    if prod {
                        dialog::warn(ui, tr!("⚠ Production server – it will be confirmed once more."), true);
                    }
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Upload, tr!("Upload to server")).default(true),
                        dialog::btn(Role::Close, tr!("Not now")),
                    ]);
                    if b[0] {
                        act = Some(Box::new(move |a: &mut App| a.preflight(id)));
                        done = true;
                    } else if b[1] || esc {
                        act = Some(Box::new(move |a: &mut App| {
                            if let Some(s) = a.session(id) {
                                s.state = EditState::NotUploaded(tr!("change not uploaded (can be uploaded from the list of edits)"));
                            }
                        }));
                        done = true;
                    }
                });
            }
            EditPrompt::Conflict { id, remote } => {
                let id = *id;
                let (n, loc) = name_of(self, id);
                let prod = self.edits.sessions.iter().any(|s| s.id == id && s.production);
                let new_file = self.edits.sessions.iter().any(|s| s.id == id && s.new_file);
                win(tr!("The file has changed on the server"), Role::Overwrite).danger(prod).show(ctx, |ui| {
                    let lead = if new_file {
                        tr!(
                            "A file “{name}” ({location}) was created on the server in the meantime (it is {size}). \
                             Your new file has not been uploaded yet.",
                            name = n.as_str(),
                            location = loc.as_str(),
                            size = rolid_i18n::size(remote.size)
                        )
                    } else {
                        tr!(
                            "“{name}” ({location}) has changed on the server since you opened it \
                             (it is now {size}). Your edit has not been uploaded yet.",
                            name = n.as_str(),
                            location = loc.as_str(),
                            size = rolid_i18n::size(remote.size)
                        )
                    };
                    dialog::lead(ui, lead);
                    if prod {
                        dialog::warn(ui, tr!("⚠ Production server."), true);
                    }
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Overwrite, tr!("Overwrite the version on the server")),
                        dialog::btn(Role::Save, tr!("Save alongside under another name")),
                        dialog::cancel(),
                    ]);
                    if b[0] {
                        act = Some(Box::new(move |a: &mut App| a.upload(id, None)));
                        done = true;
                    } else if b[1] {
                        let aside = aside_name(&n, &chrono::Local::now().format("%Y-%m-%d %H%M").to_string());
                        let aside = rolid_i18n::t(&aside);
                        act = Some(Box::new(move |a: &mut App| a.upload(id, Some(aside))));
                        done = true;
                    } else if b[2] || esc {
                        act = Some(Box::new(move |a: &mut App| {
                            if let Some(s) = a.session(id) {
                                s.end_after_upload = false;
                            }
                            a.edit_failed_during_quit();
                        }));
                        done = true;
                    }
                });
            }
            EditPrompt::Production { id } => {
                let id = *id;
                let (n, loc) = name_of(self, id);
                win(tr!("⚠ Upload to the production server?"), Role::Upload).danger(true).show(ctx, |ui| {
                    dialog::lead(ui, tr!("The saved edit of “{name}” will be uploaded to {location}.", name = n.as_str(), location = loc.as_str()));
                    let b = dialog::buttons(ui, [dialog::btn(Role::Upload, tr!("Upload")), dialog::btn(Role::Close, tr!("Not now"))]);
                    if b[0] {
                        act = Some(Box::new(move |a: &mut App| a.upload(id, None)));
                        done = true;
                    } else if b[1] || esc {
                        act = Some(Box::new(move |a: &mut App| {
                            if let Some(s) = a.session(id) {
                                s.end_after_upload = false;
                            }
                            a.edit_failed_during_quit();
                        }));
                        done = true;
                    }
                });
            }
            EditPrompt::Unsaved { ids, then } => {
                let names: Vec<String> = ids.iter().map(|i| name_of(self, *i).0).collect();
                win(tr!("Changes not uploaded"), Role::Upload).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!(
                            "These edited files are not on the server yet: {names}.",
                            names = names.iter().map(|n| tr!("“{name}”", name = n.as_str())).collect::<Vec<_>>().join(", ")
                        ),
                    );
                    let (up, drop_) = match then {
                        AfterUnsaved::CloseTab(_) => (tr!("Upload and close the tab"), tr!("Discard changes and close")),
                        AfterUnsaved::Quit => (tr!("Upload and quit"), tr!("Quit without uploading")),
                        AfterUnsaved::End => (tr!("Upload and end editing"), tr!("Discard changes")),
                    };
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Upload, up),
                        dialog::btn(Role::Delete, drop_),
                        dialog::btn(Role::Close, tr!("Back")),
                    ]);
                    let then = *then;
                    if b[0] || b[1] {
                        let (ids, upload) = (ids.clone(), b[0]);
                        act = Some(Box::new(move |a: &mut App| a.after_unsaved(ids, then, upload)));
                        done = true;
                    } else if b[2] || esc {
                        done = true;
                    }
                });
            }
            EditPrompt::Recovered { orphan } => {
                let m = &orphan.meta;
                win(tr!("Unsaved edits were left after a crash"), Role::Upload).danger(true).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!(
                            "“{name}” ({user}@{host}:{path}) was edited in an application that did not end properly. \
                             This version is not on the server.",
                            name = m.file.as_str(),
                            user = m.user.as_str(),
                            host = m.host.as_str(),
                            path = m.remote_path.as_str()
                        ),
                    );
                    if m.production {
                        dialog::warn(ui, tr!("⚠ Production server."), true);
                    }
                    ui.horizontal(|ui| {
                        if crate::look::btn(Role::Folder, tr!("Open folder")).ui(ui).clicked() {
                            let dir = orphan.dir.clone();
                            act = Some(Box::new(move |a: &mut App| {
                                let cmd = if cfg!(windows) { "explorer {file}" } else if cfg!(target_os = "macos") { "open {file}" } else { "xdg-open {file}" };
                                if let Err(e) = editor::launch(cmd, &dir) {
                                    a.last_error = Some(rolid_i18n::t(&e));
                                }
                            }));
                            // the question stays open
                        }
                        dialog::note(ui, orphan.dir.display().to_string());
                    });
                    dialog::note(ui, tr!("Esc = decide later (it asks again at the next start)."));
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Upload, tr!("Upload")).tooltip(tr!("Connects to the server and uploads (with a conflict check)")),
                        dialog::btn(Role::Delete, tr!("Discard")),
                    ]);
                    if b[0] {
                        let o = (**orphan).clone();
                        act = Some(Box::new(move |a: &mut App| a.adopt_orphan(o)));
                        done = true;
                    } else if b[1] {
                        let o = (**orphan).clone();
                        act = Some(Box::new(move |a: &mut App| {
                            if let Err(e) = o.discard() {
                                a.last_error = Some(format!("{}: {e}", o.dir.display()));
                            }
                        }));
                        done = true;
                    } else if esc || dialog::cancel_pressed(ui) {
                        done = true;
                    }
                });
            }
            EditPrompt::Editor { cmd } => {
                win(tr!("External editor (F4)"), Role::Edit).show(ctx, |ui| {
                    dialog::lead(ui, tr!("The command that opens the file. {{file}} is replaced by the path to the file."));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(cmd).desired_width(dialog::width() - 30.0).hint_text(editor::detect_default()));
                        dialog::help(
                            ui,
                            format!(
                                "{}\n\n{}",
                                tr!("Empty = automatic (now: {command}).", command = editor::detect_default()),
                                tr!("A terminal editor only through a terminal, e.g.: x-terminal-emulator -e nano {{file}}")
                            ),
                        );
                    });
                    let check = if cmd.trim().is_empty() {
                        Ok(())
                    } else {
                        editor::build(cmd, std::path::Path::new("/tmp/x")).map(|_| ())
                    };
                    if let Err(e) = &check {
                        dialog::warn(ui, rolid_i18n::t(e), true);
                    }
                    let b = dialog::buttons(ui, [dialog::btn(Role::Save, tr!("Save")).default(true).enabled(check.is_ok()), dialog::cancel()]);
                    if b[0] {
                        let c = cmd.trim().to_string();
                        act = Some(Box::new(move |a: &mut App| {
                            a.settings.editor_command = (!c.is_empty()).then_some(c);
                            a.save_settings();
                        }));
                        done = true;
                    } else if b[1] || esc {
                        done = true;
                    }
                });
            }
        }
        if !done {
            self.edits.prompts.insert(0, p);
        }
        if let Some(a) = act {
            a(self);
        }
    }
}

/// What a queued transfer gets: speed limit, cancel flag, progress callback.
pub struct TransferCtl {
    limit: Arc<rolid_core::rate::RateLimit>,
    cancel: Arc<AtomicBool>,
    cb: Arc<dyn Fn(u64) + Send + Sync>,
}

impl TransferCtl {
    fn ctl(&self) -> Ctl<'_> {
        Ctl { limit: Some(self.limit.clone()), cancel: self.cancel.clone(), progress: &*self.cb }
    }
}

/// The first bytes of a file (to recognize scripts by `#!`).
/// What follows the check of the server before an upload.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Next {
    /// The connection is lost: upload once it is back.
    WaitLink,
    /// The file on the server is not the one that was opened (or, for a new
    /// file, one of the same name appeared): ask.
    Conflict(RemoteStamp),
    /// A production server: ask first.
    Confirm,
    Upload,
    Fail,
}

/// Decide after `stat` of the server's file: `result` is its state, `lost` a
/// lost connection, `missing` a file that is not there. A new file (Shift+F4)
/// is created only now, by the upload; an edited file must still be there.
fn preflight_next(new_file: bool, production: bool, stamp: Option<RemoteStamp>, result: &Result<RemoteStamp, String>, lost: bool, missing: bool) -> Next {
    match result {
        Err(_) if lost => Next::WaitLink,
        Ok(now) if new_file => Next::Conflict(*now),
        Err(_) if new_file && missing && production => Next::Confirm,
        Err(_) if new_file && missing => Next::Upload,
        Ok(now) if stamp.is_some_and(|st| st.changed(now)) => Next::Conflict(*now),
        Ok(_) if production => Next::Confirm,
        Ok(_) => Next::Upload,
        Err(_) => Next::Fail,
    }
}

/// An empty file only the owner can read (the edit copy of a new file).
pub(super) fn create_private_file(path: &std::path::Path) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path).map(|_| ())
}

pub(super) fn read_head(path: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; 64];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    buf.truncate(n);
    buf
}

#[cfg(test)]
mod new_file_tests {
    use super::*;

    #[test]
    fn a_new_file_reaches_the_server_only_by_an_upload() {
        let gone: Result<RemoteStamp, String> = Err("No such file".into());
        let there = Ok(RemoteStamp { size: 12, mtime: Some(1_700_000_000) });
        // The first save of a new file: not on the server yet, so it is uploaded
        // (created) now; on a production server only after a confirmation.
        assert_eq!(preflight_next(true, false, None, &gone, false, true), Next::Upload);
        assert_eq!(preflight_next(true, true, None, &gone, false, true), Next::Confirm);
        // Someone created a file of that name in the meantime: never overwritten silently.
        assert_eq!(preflight_next(true, false, None, &there, false, false), Next::Conflict(there.clone().unwrap()));
        // Lost connection: waits, creates nothing.
        assert_eq!(preflight_next(true, false, None, &gone, true, false), Next::WaitLink);
        // Another error (permissions…): nothing is uploaded.
        assert_eq!(preflight_next(true, false, None, &Err("Permission denied".into()), false, false), Next::Fail);
        // F4 on an existing file that disappeared from the server: not re-created.
        let st = there.clone().unwrap();
        assert_eq!(preflight_next(false, false, Some(st), &gone, false, true), Next::Fail);
        assert_eq!(preflight_next(false, false, Some(st), &there, false, false), Next::Upload);
    }

    #[test]
    fn the_edit_copy_of_a_new_file_is_private_and_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("new file.txt");
        create_private_file(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().len(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(create_private_file(&f).is_err(), "an existing file is never replaced");
        // Nothing saved in the editor: the copy is still the empty file, so no
        // change is ever seen and nothing is uploaded.
        let fp = fingerprint(&f).unwrap();
        assert_eq!(fp.size, 0);
    }
}
