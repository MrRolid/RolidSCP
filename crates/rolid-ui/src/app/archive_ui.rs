//! Archives: packing (Alt+F5), extraction (Alt+F9), test (Alt+Shift+F9).
//!
//! The archive work itself (`rolid_archive`) is blocking and runs on a
//! blocking thread as a queued job with progress and "Cancel". Remote sources
//! are first downloaded into a private temporary folder, remote targets are
//! built locally and uploaded through the normal transfer queue (`.filepart`,
//! production rules, continuing after an outage). The temporary folder is
//! deleted when the last step using it is gone.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use rolid_archive::{Format, Level, Tick};
use rolid_core::copy::{Conflict, Failure, Progress, Totals};
use rolid_core::vfs::validate_name;
use rolid_core::Vfs;
use rolid_local::LocalFs;
use secrecy::SecretString;
use zeroize::Zeroizing;

use super::{find_pane, App, CopySpec, Dialog, Job, Msg, PaneRef, WARN_ORANGE};
use crate::dialog;
use crate::look::Role;
use crate::pane::Target;

// ------------------------------------------------------------ temp folders

/// A private temporary folder, deleted (in the background) when dropped.
#[derive(Debug)]
pub struct TempDir {
    pub path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let p = self.path.clone();
        std::thread::spawn(move || {
            let _ = std::fs::remove_dir_all(&p);
        });
    }
}

/// Base of the archive temp folders: on the disk (not the RAM-backed runtime
/// dir – archives can be big): `$ROLIDSCP_ARCHIVE_DIR` or `<cache>/rolidscp/archive`.
pub fn temp_base() -> PathBuf {
    if let Some(d) = std::env::var_os("ROLIDSCP_ARCHIVE_DIR") {
        return PathBuf::from(d);
    }
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("rolidscp").join("archive")
}

/// New private (0700) temporary folder.
pub fn new_temp() -> Result<Arc<TempDir>, String> {
    let d = rolid_edit::temp::create(&temp_base(), "x").map_err(|e| rolid_i18n::t(&e))?;
    Ok(Arc::new(TempDir { path: d.dir }))
}

/// Leftovers of a crashed run (older than two days) are removed at start.
pub fn clean_old_temps() {
    let Ok(rd) = std::fs::read_dir(temp_base()) else { return };
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(2 * 24 * 3600));
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

// ------------------------------------------------------------------ dialogs

pub enum ArchiveDialog {
    Pack(PackForm),
    Extract(Box<super::unpack_ui::ExtractForm>),
    TestPassword(super::unpack_ui::TestPassword),
    TestResult(super::unpack_ui::TestResult),
    /// Password for an archive opened as a folder.
    DirPassword(super::archdir_ui::DirPassword),
    /// "Download and open" a remote archive.
    RemoteOpen(super::archdir_ui::RemoteOpen),
}

pub struct PackForm {
    from: PaneRef,
    items: Vec<Target>,
    name: String,
    format: Format,
    level: Level,
    /// zip: store already-compressed files as they are.
    store_compressed: bool,
    pw1: Zeroizing<String>,
    pw2: Zeroizing<String>,
    show_pw: bool,
    to_other: bool,
    /// What to do when an archive of that name exists in the target.
    conflict: Option<Conflict>,
    focus: bool,
}

/// Everything a packing needs once the dialog is confirmed.
struct PackPlan {
    sfs: Arc<dyn Vfs>,
    items: Vec<Target>,
    dfs: Arc<dyn Vfs>,
    dpath: String,
    dst_label: String,
    name: String,
    format: Format,
    level: Level,
    store_compressed: bool,
    threads: usize,
    password: Option<SecretString>,
    refresh: Vec<PaneRef>,
    production: bool,
    links: Vec<Arc<rolid_ssh::Link>>,
}

/// Default archive name (without extension): the single item without its
/// extension, otherwise the name of the current folder.
pub fn default_pack_name(items: &[(String, bool)], folder: &str) -> String {
    let base = match items {
        [(n, is_dir)] => {
            if *is_dir {
                n.clone()
            } else {
                let s = rolid_archive::strip_archive_ext(n);
                match s.rsplit_once('.') {
                    Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                    _ => s,
                }
            }
        }
        _ => folder.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("").to_string(),
    };
    if base.is_empty() {
        trc!("file name", "archive")
    } else {
        base
    }
}

/// `name` with the extension of `format` (any archive extension replaced).
pub fn with_ext(name: &str, format: Format) -> String {
    format!("{}{}", rolid_archive::strip_archive_ext(name.trim()), format.ext())
}

impl App {
    /// Alt+F5: ask how to pack the selected items.
    pub(super) fn ask_pack(&mut self) {
        let from = self.active_ref();
        let Some(src) = self.pane(from) else { return };
        let items = src.target_items();
        if items.is_empty() {
            self.last_error = Some(tr!("Nothing is selected to pack."));
            return;
        }
        let kinds: Vec<(String, bool)> = items
            .iter()
            .map(|t| {
                let dir = src.entries.iter().find(|e| e.name == t.name).is_some_and(|e| e.is_dir_like());
                (t.name.clone(), dir)
            })
            .collect();
        let folder = if src.search.is_some() { String::new() } else { src.path.clone() };
        let format = self.settings.pack_format.as_deref().and_then(Format::from_key).filter(|f| Format::PACK.contains(f));
        let format = format.unwrap_or(Format::Zip);
        let level = self.settings.pack_level.as_deref().and_then(Level::from_key).unwrap_or_default();
        let level = if format.levels().contains(&level) { level } else { Level::Fast };
        self.dialog = Dialog::Archive(Box::new(ArchiveDialog::Pack(PackForm {
            from,
            items,
            name: with_ext(&default_pack_name(&kinds, &folder), format),
            format,
            level,
            store_compressed: !self.settings.pack_compress_all,
            pw1: Zeroizing::default(),
            pw2: Zeroizing::default(),
            show_pw: false,
            to_other: true,
            conflict: super::preset_conflict(self.settings.copy_preset()).map(|c| match c {
                // The new archive is always newer than the old one.
                Conflict::Newer | Conflict::Resume => Conflict::Overwrite,
                c => c,
            }),
            focus: true,
        })));
    }

    pub(super) fn archive_dialog_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Archive(d) = &mut self.dialog else { return };
        match d.as_ref() {
            ArchiveDialog::Extract(_) => return self.extract_dialog_ui(ctx),
            ArchiveDialog::TestPassword(_) | ArchiveDialog::TestResult(_) => return self.test_dialogs_ui(ctx),
            ArchiveDialog::DirPassword(_) | ArchiveDialog::RemoteOpen(_) => return self.archdir_dialogs_ui(ctx),
            ArchiveDialog::Pack(_) => {}
        }
        let Dialog::Archive(d) = &mut self.dialog else { return };
        let mut close = false;
        let mut act: Option<super::Action> = None;
        if let ArchiveDialog::Pack(f) = d.as_mut() {
            {
                let to = if f.to_other {
                    let side = 1 - f.from.side;
                    PaneRef { tab: self.sides[side].current().id, side }
                } else {
                    f.from
                };
                let src = find_pane(&self.sides, f.from);
                let dst = find_pane(&self.sides, to);
                let label = |p: Option<&crate::pane::Pane>| {
                    p.map(|p| format!("{}:{}", p.display_label(), p.path)).unwrap_or_default()
                };
                let src_label = label(src);
                let other_label = label(find_pane(&self.sides, {
                    let side = 1 - f.from.side;
                    PaneRef { tab: self.sides[side].current().id, side }
                }));
                let production = dst.and_then(|d| d.remote.as_ref()).is_some_and(|r| r.production);
                let dst_search = dst.is_some_and(|d| d.search.is_some());
                let dst_readonly = dst.is_some_and(|d| d.fs.read_only());
                let name = f.name.trim().to_string();
                let exists = dst.is_some_and(|d| d.entries.iter().any(|e| e.name == name));
                let pw_on = f.format.can_encrypt();
                let pw_mismatch = pw_on && *f.pw1 != *f.pw2;
                let name_err = if name.is_empty() {
                    Some(tr!("Enter the archive name."))
                } else {
                    validate_name(&name).err().map(|e| rolid_i18n::t(&e.text()))
                };
                dialog::window("pack", tr!("Pack")).icon(Role::Pack).min_width(520.0).danger(production).show(ctx, |ui| {
                    if f.items.len() == 1 {
                        dialog::lead(ui, tr!("“{name}”  from  {source}", name = &f.items[0].label, source = &src_label));
                    } else {
                        dialog::lead(ui, trn!("{n} item  from  {source}", "{n} items  from  {source}", f.items.len(), source = &src_label));
                    }
                    let heading = tr!("Archive");
                    dialog::group(ui, &heading, |ui| {
                        dialog::form(ui, "pack-form", |ui| {
                            ui.label(tr!("Archive name:"));
                            let r = ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(360.0));
                            if f.focus {
                                r.request_focus();
                                f.focus = false;
                            }
                            ui.end_row();
                            ui.label(tr!("Format:"));
                            ui.horizontal(|ui| {
                                for fm in Format::PACK {
                                    if ui.radio(f.format == fm, fm.label()).clicked() && f.format != fm {
                                        f.format = fm;
                                        f.name = with_ext(&f.name, fm);
                                        if !fm.levels().contains(&f.level) {
                                            f.level = Level::Fast;
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(tr!("Compression:"));
                            ui.horizontal(|ui| {
                                for &l in f.format.levels() {
                                    if ui.radio(f.level == l, rolid_i18n::tc_str("compression", l.label())).clicked() {
                                        f.level = l;
                                    }
                                }
                            });
                            ui.end_row();
                        });
                        if let Some(e) = &name_err {
                            dialog::warn(ui, e.as_str(), false);
                        }
                        // Only zip; the row keeps its place for the other formats.
                        ui.scope(|ui| {
                            if f.format != Format::Zip {
                                ui.set_invisible();
                            }
                            ui.add_enabled_ui(f.level != Level::Store, |ui| {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut f.store_compressed, tr!("Do not compress already compressed files"));
                                    dialog::help(ui, tr!("Videos, music, jpg/png images, archives, Office documents… are only stored in the zip: compression would not make them smaller, only slower."));
                                });
                            });
                        });
                    });
                    let heading = tr!("Password");
                    dialog::group(ui, &heading, |ui| {
                        // Always the same rows (the dialog does not jump when the format changes).
                        ui.add_enabled_ui(pw_on, |ui| {
                            dialog::form(ui, "pack-pw", |ui| {
                                ui.label(tr!("Password (optional):"));
                                crate::secret::edit(ui, &mut f.pw1, !f.show_pw, &tr!("no password"), 240.0);
                                ui.end_row();
                                ui.label(tr!("Repeat password:"));
                                crate::secret::edit(ui, &mut f.pw2, !f.show_pw, "", 240.0);
                                ui.end_row();
                            });
                            ui.checkbox(&mut f.show_pw, tr!("Show password"));
                        });
                        if !pw_on {
                            dialog::note(ui, tr!("The tar format does not support a password – choose zip or 7z for encryption."));
                        } else if pw_mismatch {
                            dialog::warn(ui, tr!("The passwords do not match."), false);
                        } else if !f.pw1.is_empty() {
                            dialog::note(ui, match f.format {
                                Format::SevenZ => tr!("AES-256 encryption, the file names are encrypted too. The password is not saved anywhere."),
                                _ => tr!("AES-256 encryption (file contents; the names stay readable). The password is not saved anywhere."),
                            });
                        } else {
                            dialog::note(ui, tr!("Without a password the archive is not encrypted."));
                        }
                    });
                    let heading = tr!("Target");
                    dialog::group(ui, &heading, |ui| {
                        ui.horizontal(|ui| {
                            if ui.radio(f.to_other, tr!("to the other panel  {location}", location = &other_label)).clicked() {
                                f.to_other = true;
                            }
                            if dst.is_some_and(|d| d.fs.is_remote()) || src.is_some_and(|s| !s.fs.is_local_disk()) {
                                dialog::help(ui, tr!("The archive is built in a temporary folder on this computer and transferred through the transfer queue."));
                            }
                        });
                        if ui.radio(!f.to_other, tr!("to this panel  {location}", location = &src_label)).clicked() {
                            f.to_other = false;
                        }
                        if dst_search {
                            dialog::warn(ui, tr!("The target panel shows search results – open a folder in it first."), false);
                        }
                        if dst_readonly {
                            dialog::warn(ui, tr!("Cannot write into an archive – choose a different target."), false);
                        }
                        if exists {
                            dialog::warn(ui, tr!("⚠ “{name}” already exists in the target.", name = &name), false);
                            ui.horizontal(|ui| {
                                if ui.radio(f.conflict == Some(Conflict::Skip), tr!("Do not pack (keep the existing one)")).clicked() {
                                    f.conflict = Some(Conflict::Skip);
                                }
                                let sel = f.conflict == Some(Conflict::Overwrite);
                                let t = if sel {
                                    egui::RichText::new(tr!("Overwrite  ⚠ the existing archive will be replaced")).color(WARN_ORANGE).strong()
                                } else {
                                    egui::RichText::new(tr!("Overwrite"))
                                };
                                if ui.radio(sel, t).clicked() {
                                    f.conflict = Some(Conflict::Overwrite);
                                }
                            });
                        }
                    });
                    if production {
                        dialog::warn(ui, tr!("⚠ The target is a PRODUCTION server – Enter is disabled, confirm with the button."), true);
                    }
                    let ok_enabled = name_err.is_none()
                        && !pw_mismatch
                        && !dst_search
                        && !dst_readonly
                        && dst.is_some()
                        && (!exists || f.conflict.is_some());
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Pack, tr!("Pack")).default(!production).enabled(ok_enabled),
                        dialog::cancel(),
                    ]);
                    if b[0] && ok_enabled {
                        let form = std::mem::replace(f, PackForm::empty(f.from));
                        act = Some(Box::new(move |a: &mut App| a.start_pack(form, to, exists)));
                        close = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
        }
        if close {
            self.dialog = Dialog::None;
        }
        if let Some(a) = act {
            a(self);
        }
    }

    fn start_pack(&mut self, f: PackForm, to: PaneRef, exists: bool) {
        self.settings.pack_format = Some(f.format.key().into());
        self.settings.pack_level = Some(f.level.key().into());
        if f.format == Format::Zip {
            self.settings.pack_compress_all = !f.store_compressed;
        }
        self.save_settings();
        let name = f.name.trim().to_string();
        if exists && f.conflict == Some(Conflict::Skip) {
            self.last_error = Some(tr!("Archive “{name}” already exists in the target – not packing.", name = &name));
            return;
        }
        let (Some(src), Some(dst)) = (self.pane(f.from), self.pane(to)) else { return };
        let password = (f.format.can_encrypt() && !f.pw1.is_empty()).then(|| SecretString::from(f.pw1.to_string()));
        let plan = PackPlan {
            sfs: src.fs.clone(),
            items: f.items,
            dfs: dst.fs.clone(),
            dpath: dst.path.clone(),
            dst_label: dst.display_label(),
            name,
            format: f.format,
            level: f.level,
            store_compressed: f.store_compressed,
            threads: self.settings.pack_threads() as usize,
            password,
            refresh: vec![to],
            production: dst.remote.as_ref().is_some_and(|r| r.production),
            links: [self.pane_link(f.from), self.pane_link(to)].into_iter().flatten().collect(),
        };
        if let Some(p) = self.pane_mut(f.from) {
            p.marked.clear();
        }
        if plan.sfs.is_local_disk() {
            let paths = plan.items.iter().map(|t| PathBuf::from(plan.sfs.join(&t.dir, &t.name))).collect();
            self.pack_stage(plan, paths, None);
            return;
        }
        // Remote (or virtual) source: download into a private temp folder first.
        let temp = match new_temp() {
            Ok(t) => t,
            Err(e) => {
                self.last_error = Some(tr!("Cannot create a temporary folder: {error}", error = e));
                return;
            }
        };
        let dir = temp.path.join("src");
        if let Err(e) = std::fs::create_dir(&dir) {
            self.last_error = Some(format!("{}: {e}", dir.display()));
            return;
        }
        let paths: Vec<PathBuf> = plan.items.iter().map(|t| dir.join(&t.name)).collect();
        let what = items_label(&plan.items);
        let (sfs, items, links) = (plan.sfs.clone(), plan.items.clone(), plan.links.clone());
        let t2 = temp.clone();
        let title = tr!("{what} ⇣ to pack", what = &what);
        self.spawn_copy(CopySpec {
            sfs,
            dfs: Arc::new(LocalFs),
            dpath: dir.to_string_lossy().into_owned(),
            items,
            conflict: Conflict::Overwrite,
            title,
            refresh: vec![],
            links,
            auto_retry: true,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: Some(Box::new(move |a: &mut App| a.pack_stage(plan, paths, Some(t2)))),
            temp: Some(temp),
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    fn pack_stage(&mut self, plan: PackPlan, paths: Vec<PathBuf>, temp: Option<Arc<TempDir>>) {
        let local_out = plan.dfs.is_local_disk();
        let (out, temp) = if local_out {
            (PathBuf::from(plan.dfs.join(&plan.dpath, &plan.name)), temp)
        } else {
            let t = match temp.map(Ok).unwrap_or_else(new_temp) {
                Ok(t) => t,
                Err(e) => {
                    self.last_error = Some(tr!("Cannot create a temporary folder: {error}", error = e));
                    return;
                }
            };
            let dir = t.path.join("out");
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.last_error = Some(format!("{}: {e}", dir.display()));
                return;
            }
            (dir.join(&plan.name), Some(t))
        };
        let opts = rolid_archive::PackOpts {
            format: plan.format,
            level: plan.level,
            password: plan.password.clone(),
            threads: plan.threads,
            store_compressed: plan.store_compressed,
        };
        let out2 = out.clone();
        let title = format!("📦 {}", plan.name);
        self.archive_job(
            title.clone(),
            move |ctl| rolid_archive::pack_with(&paths, &out2, &opts, ctl),
            move |a: &mut App, res| match res {
                Ok(rep) => {
                    if !rep.failures.is_empty() {
                        let id = a.id();
                        let failures = rep.failures.into_iter().map(|(path, error)| Failure { path, error }).collect();
                        a.drop.reports.push(super::drop_ui::Report { id, title, failures });
                    }
                    if local_out {
                        a.refresh_all(&plan.refresh);
                    } else {
                        a.upload_result(plan, out, temp);
                    }
                }
                Err(e) if e.is_cancel() => {}
                Err(e) => a.last_error = Some(tr!("Packing “{name}” failed: {error}", name = &plan.name, error = e.text())),
            },
        );
    }

    /// Upload the archive built in a temp folder through the transfer queue.
    fn upload_result(&mut self, plan: PackPlan, out: PathBuf, temp: Option<Arc<TempDir>>) {
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let dir = out.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        self.spawn_copy(CopySpec {
            sfs: Arc::new(LocalFs),
            dfs: plan.dfs,
            dpath: plan.dpath,
            items: vec![Target { dir, name: plan.name.clone(), label: plan.name.clone(), size }],
            // The user already chose to replace an existing archive.
            conflict: Conflict::Overwrite,
            title: format!("{} → {}", plan.name, plan.dst_label),
            refresh: plan.refresh,
            links: plan.links,
            auto_retry: !plan.production,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: None,
            temp,
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    /// Refresh every panel showing the same place as one of `at`.
    pub(super) fn refresh_all(&mut self, at: &[PaneRef]) {
        let all: Vec<PaneRef> = at.iter().flat_map(|r| self.same_location(*r)).collect();
        for r in all {
            self.refresh(r);
        }
    }

    /// Run blocking archive work as a queued job with progress and "Cancel";
    /// `done` gets the result on the UI thread.
    pub(super) fn archive_job<T: Send + 'static>(
        &mut self,
        title: String,
        work: impl FnOnce(&rolid_archive::Ctl) -> rolid_archive::Result<T> + Send + 'static,
        done: impl FnOnce(&mut App, rolid_archive::Result<T>) + Send + 'static,
    ) -> u64 {
        let id = self.id();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let totals = Arc::new(Mutex::new(Totals::default()));
        self.jobs.push(Job {
            id,
            title,
            progress: progress.clone(),
            total_bytes: 0,
            cancel: cancel.clone(),
            note: Arc::default(),
            count: None,
            totals: totals.clone(),
        });
        let slots = self.transfer_slots.clone();
        let ctx = self.ctx.clone();
        self.spawn(async move {
            let permit = loop {
                if cancel.load(Ordering::Relaxed) {
                    break None;
                }
                if let Ok(p) = slots.clone().try_acquire_owned() {
                    break Some(p);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            if permit.is_none() {
                return Msg::Then { job: Some(id), f: Box::new(move |a| done(a, Err(rolid_archive::Error::Canceled))) };
            }
            progress.lock().unwrap().started = Some(Instant::now());
            let res = tokio::task::spawn_blocking(move || {
                let on = |t: Tick<'_>| {
                    match t {
                        Tick::Total { items, bytes } => {
                            *totals.lock().unwrap() = Totals { files: items, bytes, done: true };
                        }
                        Tick::Item(n) => progress.lock().unwrap().current = n.to_string(),
                        Tick::Bytes(n) => progress.lock().unwrap().bytes_done += n,
                        Tick::Done => progress.lock().unwrap().files_done += 1,
                        Tick::Read { done, total } => progress.lock().unwrap().read = Some((done, total)),
                    }
                    ctx.request_repaint_after(Duration::from_millis(150));
                };
                work(&rolid_archive::Ctl { cancel: &cancel, on: &on })
            })
            .await
            .unwrap_or_else(|e| Err(rolid_archive::Error::Io(std::io::Error::other(e.to_string()))));
            drop(permit);
            Msg::Then { job: Some(id), f: Box::new(move |a| done(a, res)) }
        });
        id
    }
}

impl PackForm {
    fn empty(from: PaneRef) -> PackForm {
        PackForm {
            from,
            items: Vec::new(),
            name: String::new(),
            format: Format::Zip,
            level: Level::Normal,
            store_compressed: true,
            pw1: Zeroizing::default(),
            pw2: Zeroizing::default(),
            show_pw: false,
            to_other: true,
            conflict: None,
            focus: false,
        }
    }
}

/// "“a.txt”" or "5 items".
pub fn items_label(items: &[Target]) -> String {
    match items {
        [t] => tr!("“{text}”", text = &t.label),
        _ => trn!("{n} item", "{n} items", items.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names() {
        assert_eq!(default_pack_name(&[("photo.jpg".into(), false)], "/home/a"), "photo");
        assert_eq!(default_pack_name(&[("backup.tar.gz".into(), false)], "/home/a"), "backup");
        assert_eq!(default_pack_name(&[("project.v2".into(), true)], "/home/a"), "project.v2");
        assert_eq!(default_pack_name(&[("a".into(), false), ("b".into(), true)], "/home/documents/"), "documents");
        assert_eq!(default_pack_name(&[("a".into(), false), ("b".into(), false)], "/"), "archive");
        assert_eq!(default_pack_name(&[(".bashrc".into(), false)], "/"), ".bashrc");
        assert_eq!(with_ext("photo", Format::SevenZ), "photo.7z");
        assert_eq!(with_ext("photo.zip", Format::TarGz), "photo.tar.gz");
        assert_eq!(with_ext(" photo.tar.zst ", Format::Zip), "photo.zip");
    }
}
