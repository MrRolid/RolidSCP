//! An archive opened as a read-only folder (Enter on an archive).
//!
//! Local archives are listed in the background and the panel switches to an
//! [`ArchiveFs`]; remote ones (up to a size set in Settings) are downloaded
//! into a private temporary folder first, as are archives inside an archive.
//! ".." at the archive root returns to the folder containing it. F5 out
//! extracts the selection directly (a remote target gets it through a
//! temporary folder and the transfer queue). Writing is refused.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use rolid_archive::{Encoding, Format, Limits, Listing, ReadOpts, Tick};
use rolid_core::copy::{Conflict, Failure};
use rolid_core::Vfs;
use rolid_local::LocalFs;
use secrecy::SecretString;
use zeroize::Zeroizing;

use super::archive_ui::{new_temp, ArchiveDialog, TempDir};
use super::unpack_ui::{exist_of, ArchFile, Upload};
use super::{App, CopySpec, Dialog, PaneRef};
use crate::archive_fs::ArchiveFs;
use crate::dialog;
use crate::look::Role;
use crate::pane::{human_size, ArchiveState, Target};

/// What happens once the password is entered.
type AfterPassword = Box<dyn FnOnce(&mut App, SecretString) + Send>;

pub struct DirPassword {
    title: String,
    text: String,
    password: Zeroizing<String>,
    error: Option<String>,
    then: Option<AfterPassword>,
}

/// "Download and open" a remote archive.
pub struct RemoteOpen {
    at: PaneRef,
    target: Target,
}

impl App {
    /// The panel shows the inside of an archive: say so and refuse.
    pub(super) fn refuse_read_only(&mut self, at: PaneRef, what: &str) -> bool {
        if self.pane(at).is_some_and(|p| p.fs.read_only()) {
            self.last_error = Some(tr!("{action}: the archive is open read-only, it cannot be written to.", action = what));
            return true;
        }
        false
    }

    /// Enter on an archive file.
    pub(super) fn enter_archive(&mut self, at: PaneRef, target: Target) {
        let Some(p) = self.pane(at) else { return };
        let Some(format) = Format::from_name(&target.name) else { return };
        let fs = p.fs.clone();
        if fs.is_local_disk() {
            let arch = ArchFile { name: target.name.clone(), path: PathBuf::from(fs.join(&target.dir, &target.name)), format };
            self.open_archive_dir(at, arch, None, None);
        } else if fs.read_only() {
            // An archive inside an archive: unpacked into a temporary folder first.
            self.download_archive(at, target, format);
        } else if target.size <= self.settings.remote_archive_max() {
            self.dialog = Dialog::Archive(Box::new(ArchiveDialog::RemoteOpen(RemoteOpen { at, target })));
        } else {
            self.last_error = Some(tr!(
                "“{name}” is {size}: remote archives over {limit} are not opened as a folder (the limit is in Settings). You can extract it with Alt+F9.",
                name = target.name.as_str(),
                size = human_size(target.size),
                limit = human_size(self.settings.remote_archive_max())
            ));
        }
    }

    /// Download a (remote or nested) archive into a temporary folder and open it.
    fn download_archive(&mut self, at: PaneRef, target: Target, format: Format) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let temp = match new_temp() {
            Ok(t) => t,
            Err(e) => {
                self.last_error = Some(tr!("Cannot create a temporary folder: {error}", error = e.to_string()));
                return;
            }
        };
        let dir = temp.path.join("src");
        if let Err(e) = std::fs::create_dir(&dir) {
            self.last_error = Some(format!("{}: {e}", dir.display()));
            return;
        }
        let arch = ArchFile { name: target.name.clone(), path: dir.join(&target.name), format };
        let links = self.pane_link(at).into_iter().collect();
        let t2 = temp.clone();
        let title = tr!("{name} ⇣ to open", name = target.name.as_str());
        self.spawn_copy(CopySpec {
            sfs: fs,
            dfs: Arc::new(LocalFs),
            dpath: dir.to_string_lossy().into_owned(),
            title,
            items: vec![target],
            conflict: Conflict::Overwrite,
            refresh: vec![],
            links,
            auto_retry: true,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: Some(Box::new(move |a: &mut App| a.open_archive_dir(at, arch, Some(t2), None))),
            temp: Some(temp),
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    /// List the archive in the background, then show it in the panel.
    fn open_archive_dir(&mut self, at: PaneRef, arch: ArchFile, temp: Option<Arc<TempDir>>, password: Option<SecretString>) {
        let max_entries = self.settings.unpack_items();
        let (path, format) = (arch.path.clone(), arch.format);
        let pw = password.clone();
        self.archive_job(
            tr!("Open {name}", name = arch.name.as_str()),
            move |ctl| {
                ctl.check()?;
                let opts = ReadOpts { password: pw, encoding: Encoding::default(), max_entries };
                rolid_archive::list(&path, format, &opts)
            },
            move |a: &mut App, res| match res {
                Ok(l) => a.show_archive_dir(at, arch, l, password, temp),
                Err(e) if e.is_cancel() => {}
                Err(e @ (rolid_archive::Error::NeedPassword | rolid_archive::Error::BadPassword)) => {
                    let name = arch.name.clone();
                    a.ask_dir_password(
                        tr!("Open {name}", name = name.as_str()),
                        tr!("The archive has encrypted file names too; it cannot be listed without the password."),
                        matches!(e, rolid_archive::Error::BadPassword),
                        Box::new(move |a: &mut App, pw| a.open_archive_dir(at, arch, temp, Some(pw))),
                    );
                }
                Err(e) => a.last_error = Some(tr!("Cannot open “{name}”: {error}", name = arch.name.as_str(), error = e.text())),
            },
        );
    }

    pub(super) fn ask_dir_password(&mut self, title: String, text: String, wrong: bool, then: AfterPassword) {
        self.dialog = Dialog::Archive(Box::new(ArchiveDialog::DirPassword(DirPassword {
            title,
            text,
            password: Zeroizing::default(),
            error: wrong.then(|| tr!("Wrong password, try again.")),
            then: Some(then),
        })));
    }

    /// Switch the panel into the listed archive.
    fn show_archive_dir(&mut self, at: PaneRef, arch: ArchFile, l: Listing, password: Option<SecretString>, temp: Option<Arc<TempDir>>) {
        let rejected = l.rejected.clone();
        let fs = Arc::new(ArchiveFs::new(arch.path.clone(), arch.name.clone(), arch.format, &l, Encoding::default(), password, temp));
        let root = arch.name.clone();
        let Some(p) = self.pane_mut(at) else { return };
        p.exit_search();
        let state = ArchiveState {
            fs: fs.clone(),
            back_fs: std::mem::replace(&mut p.fs, fs),
            back_remote: p.remote.take(),
            back_path: p.path.clone(),
            back_name: arch.name.clone(),
            back_history: std::mem::take(&mut p.history),
            back_offline: p.offline,
            back_archive: p.archive.take(),
        };
        p.archive = Some(Box::new(state));
        p.jump_to(root.clone());
        self.load(at, root, None);
        if !rejected.is_empty() {
            let id = self.id();
            let title = tr!("“{name}”: hidden items with an unsafe path", name = arch.name.as_str());
            self.drop.reports.push(super::drop_ui::Report {
                id,
                title,
                failures: rejected.into_iter().map(|r| Failure { path: r.name, error: r.reason }).collect(),
            });
        }
    }

    /// ".." at the archive root (or a path outside it): back where it was opened.
    pub(super) fn leave_archive(&mut self, at: PaneRef) -> bool {
        let Some(p) = self.pane_mut(at) else { return false };
        let Some(st) = p.archive.take() else { return false };
        let st = *st;
        p.exit_search();
        p.fs = st.back_fs;
        p.remote = st.back_remote;
        p.offline = st.back_offline;
        p.history = st.back_history;
        p.archive = st.back_archive;
        p.jump_to(st.back_path.clone());
        self.load(at, st.back_path, Some(st.back_name));
        true
    }

    // ------------------------------------------------------------- F5 out

    /// Copy the selection out of the archive into the other panel.
    pub(super) fn copy_out_of_archive(&mut self, from: PaneRef, to: PaneRef, dest: String, items: Vec<Target>, conflict: Conflict) {
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        let Some(st) = &src.archive else { return };
        let fs = st.fs.clone();
        if fs.encrypted && !fs.has_password() {
            self.ask_dir_password(
                tr!("Copy from {name}", name = fs.name.as_str()),
                tr!("The archive is password protected ({items}).", items = super::archive_ui::items_label(&items)),
                false,
                Box::new(move |a: &mut App, pw| {
                    if let Some(st) = a.pane(from).and_then(|p| p.archive.as_ref()) {
                        st.fs.set_password(Some(pw));
                    }
                    a.copy_out_of_archive(from, to, dest, items, conflict);
                }),
            );
            return;
        }
        let names: Vec<String> = items.iter().map(|t| t.name.clone()).collect();
        let sel = fs.select(&src.path, &names);
        let dfs = dst.fs.clone();
        let dpath = dest.clone();
        let up = Upload {
            dfs: dfs.clone(),
            dpath: dpath.clone(),
            conflict,
            dst_label: dst.display_label(),
            to,
            production: dst.remote.as_ref().is_some_and(|r| r.production),
            links: [self.pane_link(to)].into_iter().flatten().collect(),
        };
        let local_out = dfs.is_local_disk();
        let (out_dir, temp) = if local_out {
            (PathBuf::from(&dpath), None)
        } else {
            let t = match new_temp() {
                Ok(t) => t,
                Err(e) => {
                    self.last_error = Some(tr!("Cannot create a temporary folder: {error}", error = e.to_string()));
                    return;
                }
            };
            let d = t.path.join("out");
            if let Err(e) = std::fs::create_dir_all(&d) {
                self.last_error = Some(format!("{}: {e}", d.display()));
                return;
            }
            (d, Some(t))
        };
        if let Some(p) = self.pane_mut(from) {
            p.marked.clear();
        }
        let title = format!("{} ⇠ {}", super::archive_ui::items_label(&items), fs.name);
        let limits = Limits {
            max_total: self.settings.unpack_max_bytes(),
            max_entries: self.settings.unpack_items(),
            max_ratio: Some(self.settings.unpack_ratio()),
        };
        let exist = exist_of(conflict);
        let job_fs = fs.clone();
        let out2 = out_dir.clone();
        let total: u64 = items.iter().map(|t| t.size).sum();
        self.archive_job(
            title.clone(),
            move |ctl| {
                ctl.tick(Tick::Total { items: sel.items.len() as u64, bytes: total });
                let rep = rolid_archive::extract(&job_fs.archive, job_fs.format, &out2, &job_fs.read_opts(), limits, exist, Some(&sel), ctl)?;
                Ok(rep.failures)
            },
            move |a: &mut App, res| match res {
                Ok(failures) => {
                    if !failures.is_empty() {
                        let id = a.id();
                        a.drop.reports.push(super::drop_ui::Report {
                            id,
                            title,
                            failures: failures.into_iter().map(|(path, error)| Failure { path, error }).collect(),
                        });
                    }
                    if local_out {
                        a.refresh_all(&[to]);
                    } else {
                        a.upload_dir(up, out_dir, temp);
                    }
                }
                Err(e) if e.is_cancel() => {}
                Err(e @ (rolid_archive::Error::NeedPassword | rolid_archive::Error::BadPassword)) => {
                    fs.set_password(None);
                    a.ask_dir_password(
                        tr!("Copy from {name}", name = fs.name.as_str()),
                        tr!("The archive is password protected."),
                        matches!(e, rolid_archive::Error::BadPassword),
                        Box::new(move |a: &mut App, pw| {
                            fs.set_password(Some(pw));
                            a.copy_out_of_archive(from, to, dest, items, conflict);
                        }),
                    );
                }
                Err(e) => a.last_error = Some(format!("{title}: {}", rolid_i18n::t(&e.text()))),
            },
        );
    }

    // ------------------------------------------------------------- opening

    /// "Open" a file inside an archive: unpacked into a temporary copy
    /// (read-only, changes are not written back).
    pub(super) fn open_from_archive(&mut self, at: PaneRef, target: Target, opener: crate::pane::Opener) {
        let Some(st) = self.pane(at).and_then(|p| p.archive.as_ref()) else { return };
        let fs = st.fs.clone();
        let path = fs.join(&target.dir, &target.name);
        if fs.is_encrypted_item(&path) && !fs.has_password() {
            self.ask_dir_password(
                tr!("Open {name}", name = target.name.as_str()),
                tr!("The file is encrypted in the archive."),
                false,
                Box::new(move |a: &mut App, pw| {
                    fs.set_password(Some(pw));
                    a.open_from_archive(at, target, opener);
                }),
            );
            return;
        }
        let job_fs = fs.clone();
        let p2 = path.clone();
        self.archive_job(
            tr!("Open {name}", name = target.name.as_str()),
            move |ctl| {
                ctl.check()?;
                let f = job_fs.extract_file(&p2)?;
                // A copy: make it read-only so editing it is not mistaken for changing the archive.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444));
                }
                Ok(f)
            },
            move |a: &mut App, res| match res {
                Ok(f) => {
                    a.launch_with(&opener, &f);
                }
                Err(e) if e.is_cancel() => {}
                Err(e @ (rolid_archive::Error::NeedPassword | rolid_archive::Error::BadPassword)) => {
                    fs.set_password(None);
                    a.ask_dir_password(
                        tr!("Open {name}", name = target.name.as_str()),
                        tr!("The file is encrypted in the archive."),
                        matches!(e, rolid_archive::Error::BadPassword),
                        Box::new(move |a: &mut App, pw| {
                            fs.set_password(Some(pw));
                            a.open_from_archive(at, target, opener);
                        }),
                    );
                }
                Err(e) => a.last_error = Some(tr!("Cannot open “{name}”: {error}", name = target.name.as_str(), error = e.text())),
            },
        );
    }

    // ------------------------------------------------------------- dialogs

    pub(super) fn archdir_dialogs_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Archive(d) = &mut self.dialog else { return };
        let mut close = false;
        let mut run: Option<(AfterPassword, SecretString)> = None;
        let mut download: Option<(PaneRef, Target)> = None;
        match d.as_mut() {
            ArchiveDialog::DirPassword(f) => {
                dialog::window("archdir-pw", f.title.clone()).icon(Role::Unlock).show(ctx, |ui| {
                    dialog::lead(ui, f.text.as_str());
                    dialog::form(ui, "archdir-pw-form", |ui| {
                        ui.label(tr!("Password:"));
                        let r = crate::secret::edit(ui, &mut f.password, true, &tr!("archive password"), 280.0);
                        r.request_focus();
                        ui.end_row();
                    });
                    if let Some(e) = &f.error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    let ok = !f.password.is_empty();
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, tr!("Continue")).default(true).enabled(ok), dialog::cancel()]);
                    if b[0] && ok {
                        if let Some(t) = f.then.take() {
                            run = Some((t, SecretString::from(f.password.to_string())));
                        }
                        close = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            ArchiveDialog::RemoteOpen(f) => {
                dialog::window("archdir-remote", tr!("Open {name}", name = f.target.name.as_str())).icon(Role::Download).show(ctx, |ui| {
                    dialog::lead(
                        ui,
                        tr!(
                            "The archive is on the server ({size}). To browse it, it is downloaded into a temporary folder that is deleted when the archive is closed.",
                            size = human_size(f.target.size)
                        ),
                    );
                    let b = dialog::buttons(ui, [dialog::btn(Role::Download, tr!("Download and open")).default(true), dialog::cancel()]);
                    if b[0] {
                        download = Some((f.at, f.target.clone()));
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
        if let Some((f, pw)) = run {
            f(self, pw);
        }
        if let Some((at, t)) = download {
            if let Some(fmt) = Format::from_name(&t.name) {
                self.download_archive(at, t, fmt);
            }
        }
    }
}
