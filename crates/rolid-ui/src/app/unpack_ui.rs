//! Extraction (Alt+F9) and archive test (Alt+Shift+F9).
//!
//! Remote archives are downloaded into a private temporary folder first. The
//! archives are listed before the dialog (number of items at the root decides
//! the "own subfolder" default, encrypted names ask for the password, names
//! in an old code page offer an encoding, a suspicious compression ratio asks).
//! A remote target gets the files unpacked into a temporary folder and
//! uploaded through the transfer queue.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use rolid_archive::{Encoding, Exist, Format, Limits, Listing, ReadOpts, Tick};
use rolid_core::copy::{Conflict, Failure};
use rolid_core::Vfs;
use rolid_i18n::{t, t_str, Text};
use rolid_local::LocalFs;
use secrecy::SecretString;
use zeroize::Zeroizing;

use super::archive_ui::{new_temp, ArchiveDialog, TempDir};
use super::{find_pane, App, CopySpec, Dialog, PaneRef, WARN_ORANGE};
use crate::dialog;
use crate::look::Role;
use crate::pane::{human_size, Target};

/// Space always left free on the target disk.
const KEEP_FREE: u64 = 1_000_000_000;

/// An archive available as a local file.
#[derive(Clone, Debug)]
pub struct ArchFile {
    /// Name shown (as in the panel).
    pub name: String,
    pub path: PathBuf,
    pub format: Format,
}

/// What the user picked an archive command for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    Extract,
    Test,
}

pub struct ExtractForm {
    from: PaneRef,
    to_other: bool,
    archives: Vec<ArchFile>,
    temp: Option<Arc<TempDir>>,
    /// One per archive once listed.
    listings: Option<Vec<Result<Listing, Text>>>,
    /// Listing request (newer answers win).
    req: u64,
    password: Zeroizing<String>,
    show_pw: bool,
    /// Names are encrypted: the listing needs the password.
    need_pw: bool,
    pw_error: Option<String>,
    encoding: Encoding,
    subfolder: bool,
    subfolder_touched: bool,
    conflict: Option<Conflict>,
    ratio_ok: bool,
    /// Free space where the files are written (local target or temp disk).
    free: Option<u64>,
    focus_pw: bool,
}

pub struct TestPassword {
    archives: Vec<ArchFile>,
    temp: Option<Arc<TempDir>>,
    password: Zeroizing<String>,
    error: Option<String>,
}

/// Archive name, damaged items (name, reason), error that stopped the test.
pub type TestLine = (String, Vec<(String, Text)>, Option<Text>);

pub struct TestResult {
    pub lines: Vec<TestLine>,
}

/// Everything an extraction needs once the dialog is confirmed.
struct ExtractPlan {
    from: PaneRef,
    archives: Vec<ArchFile>,
    temp: Option<Arc<TempDir>>,
    password: Option<SecretString>,
    encoding: Encoding,
    limits: Limits,
    exist: Exist,
    conflict: Conflict,
    subfolder: bool,
    dfs: Arc<dyn Vfs>,
    dpath: String,
    dst_label: String,
    to: PaneRef,
    production: bool,
    links: Vec<Arc<rolid_ssh::Link>>,
}

/// Where a local folder's contents are uploaded to.
pub(super) struct Upload {
    pub dfs: Arc<dyn Vfs>,
    pub dpath: String,
    pub conflict: Conflict,
    pub dst_label: String,
    pub to: PaneRef,
    pub production: bool,
    pub links: Vec<Arc<rolid_ssh::Link>>,
}

pub(super) fn exist_of(c: Conflict) -> Exist {
    match c {
        Conflict::Skip => Exist::Skip,
        Conflict::Newer => Exist::Newer,
        Conflict::Overwrite | Conflict::Resume => Exist::Overwrite,
    }
}

/// Name of the own subfolder for an archive.
pub fn subfolder_name(archive: &str) -> String {
    let s = rolid_archive::strip_archive_ext(archive);
    if s.is_empty() || s == archive {
        format!("{archive}.d")
    } else {
        s
    }
}

/// An item name from an archive report for the user: the markers
/// [`rolid_archive::REST_OF_ARCHIVE`] / [`rolid_archive::WHOLE_ARCHIVE`] translated.
fn item_name(p: &str) -> String {
    if p == rolid_archive::REST_OF_ARCHIVE {
        t_str(rolid_archive::REST_OF_ARCHIVE)
    } else if p == rolid_archive::WHOLE_ARCHIVE {
        t_str(rolid_archive::WHOLE_ARCHIVE)
    } else {
        p.to_string()
    }
}

/// Names that would appear in the target folder.
fn top_names(a: &ArchFile, l: &Listing, subfolder: bool) -> Vec<String> {
    if subfolder {
        vec![subfolder_name(&a.name)]
    } else {
        l.root_names()
    }
}

impl App {
    /// The selected archives of the active panel (`None` + message if none).
    fn selected_archives(&mut self) -> Option<(PaneRef, Vec<Target>)> {
        let at = self.active_ref();
        let p = self.pane(at)?;
        let items: Vec<Target> = p.target_items().into_iter().filter(|t| Format::from_name(&t.name).is_some()).collect();
        if items.is_empty() {
            self.last_error = Some(tr!("Select an archive (zip, 7z, tar, tar.gz, tar.zst)."));
            return None;
        }
        Some((at, items))
    }

    /// Alt+F9 / Alt+Shift+F9: local archives go on at once, remote ones are
    /// downloaded into a temporary folder first.
    pub(super) fn ask_archive(&mut self, purpose: Purpose) {
        let Some((at, items)) = self.selected_archives() else { return };
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        if fs.is_local_disk() {
            let archives = items
                .iter()
                .map(|t| ArchFile {
                    name: t.name.clone(),
                    path: PathBuf::from(fs.join(&t.dir, &t.name)),
                    format: Format::from_name(&t.name).unwrap(),
                })
                .collect();
            self.archives_ready(purpose, at, archives, None);
            return;
        }
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
        let archives: Vec<ArchFile> = items
            .iter()
            .map(|t| ArchFile { name: t.name.clone(), path: dir.join(&t.name), format: Format::from_name(&t.name).unwrap() })
            .collect();
        let what = super::archive_ui::items_label(&items);
        let t2 = temp.clone();
        let links = self.pane_link(at).into_iter().collect();
        self.spawn_copy(CopySpec {
            sfs: fs,
            dfs: Arc::new(LocalFs),
            dpath: dir.to_string_lossy().into_owned(),
            items,
            conflict: Conflict::Overwrite,
            title: if purpose == Purpose::Test { tr!("{what} ⇣ for testing", what = what) } else { tr!("{what} ⇣ for extraction", what = what) },
            refresh: vec![],
            links,
            auto_retry: true,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: Some(Box::new(move |a: &mut App| a.archives_ready(purpose, at, archives, Some(t2)))),
            temp: Some(temp),
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    fn archives_ready(&mut self, purpose: Purpose, from: PaneRef, archives: Vec<ArchFile>, temp: Option<Arc<TempDir>>) {
        match purpose {
            Purpose::Test => self.run_test(archives, temp, None),
            Purpose::Extract => {
                let form = ExtractForm {
                    from,
                    to_other: true,
                    archives,
                    temp,
                    listings: None,
                    req: 0,
                    password: Zeroizing::default(),
                    show_pw: false,
                    need_pw: false,
                    pw_error: None,
                    encoding: Encoding::default(),
                    subfolder: false,
                    subfolder_touched: false,
                    conflict: super::preset_conflict(self.settings.copy_preset()),
                    ratio_ok: false,
                    free: None,
                    focus_pw: false,
                };
                self.dialog = Dialog::Archive(Box::new(ArchiveDialog::Extract(Box::new(form))));
                self.relist();
            }
        }
    }

    /// List the archives of the open extraction dialog (in the background).
    fn relist(&mut self) {
        let Dialog::Archive(d) = &mut self.dialog else { return };
        let ArchiveDialog::Extract(f) = d.as_mut() else { return };
        f.req += 1;
        f.listings = None;
        let req = f.req;
        let archives = f.archives.clone();
        let opts_pw = (!f.password.is_empty()).then(|| SecretString::from(f.password.to_string()));
        let encoding = f.encoding;
        let max_entries = self.settings.unpack_items();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let res = tokio::task::spawn_blocking(move || {
                let opts = ReadOpts { password: opts_pw, encoding, max_entries };
                archives
                    .iter()
                    .map(|a| rolid_archive::list(&a.path, a.format, &opts).map_err(|e| (matches!(e, rolid_archive::Error::NeedPassword), matches!(e, rolid_archive::Error::BadPassword), e.text())))
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let f: super::Then = Box::new(move |a: &mut App| {
                let Dialog::Archive(d) = &mut a.dialog else { return };
                let ArchiveDialog::Extract(f) = d.as_mut() else { return };
                if f.req != req {
                    return;
                }
                let was_need = f.need_pw;
                f.need_pw = res.iter().any(|r| matches!(r, Err((true, _, _)) | Err((_, true, _))));
                if res.iter().any(|r| matches!(r, Err((_, true, _)))) {
                    f.pw_error = Some(tr!("Wrong password, try again."));
                } else if was_need {
                    // Encrypted names could be read: the password is right.
                    f.pw_error = None;
                }
                if f.need_pw {
                    f.focus_pw = true;
                }
                let listings: Vec<Result<Listing, Text>> = res.into_iter().map(|r| r.map_err(|(_, _, m)| m)).collect();
                if !f.subfolder_touched {
                    f.subfolder = listings.iter().any(|l| l.as_ref().is_ok_and(|l| l.root_names().len() > 1));
                }
                if listings.iter().any(|l| l.as_ref().is_ok_and(|l| l.encrypted)) && f.password.is_empty() {
                    f.focus_pw = true;
                }
                f.listings = Some(listings);
                a.update_free_space();
            });
            let _ = tx.send(super::Msg::Then { job: None, f });
            ctx.request_repaint();
        });
    }

    /// Free space of the disk the files are written to.
    fn update_free_space(&mut self) {
        let Dialog::Archive(d) = &self.dialog else { return };
        let ArchiveDialog::Extract(f) = d.as_ref() else { return };
        let to = self.extract_target(f);
        let Some(dst) = self.pane(to) else { return };
        let path = if dst.fs.is_local_disk() {
            dst.path.clone()
        } else {
            let base = super::archive_ui::temp_base();
            let _ = std::fs::create_dir_all(&base);
            base.to_string_lossy().into_owned()
        };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let free = LocalFs.space(&path).await.ok().flatten().map(|s| s.avail);
            let f: super::Then = Box::new(move |a: &mut App| {
                if let Dialog::Archive(d) = &mut a.dialog {
                    if let ArchiveDialog::Extract(f) = d.as_mut() {
                        f.free = free;
                    }
                }
            });
            let _ = tx.send(super::Msg::Then { job: None, f });
            ctx.request_repaint();
        });
    }

    fn extract_target(&self, f: &ExtractForm) -> PaneRef {
        if f.to_other {
            self.other(f.from)
        } else {
            f.from
        }
    }

    pub(super) fn extract_dialog_ui(&mut self, ctx: &egui::Context) {
        let max_bytes = self.settings.unpack_max_bytes();
        let max_ratio = self.settings.unpack_ratio();
        let other = self.other_ref_of_dialog();
        let Dialog::Archive(d) = &mut self.dialog else { return };
        let ArchiveDialog::Extract(f) = d.as_mut() else { return };
        let to = if f.to_other { other } else { f.from };
        let mut close = false;
        let mut relist = false;
        let mut space = false;
        let mut start: Option<ExtractPlan> = None;
        let src = find_pane(&self.sides, f.from);
        let dst = find_pane(&self.sides, to);
        let label = |p: Option<&crate::pane::Pane>| p.map(|p| format!("{}:{}", p.display_label(), p.path)).unwrap_or_default();
        let (src_label, other_label) = (label(src), label(find_pane(&self.sides, other)));
        let production = dst.and_then(|d| d.remote.as_ref()).is_some_and(|r| r.production);
        let dst_search = dst.is_some_and(|d| d.search.is_some());
        let dst_readonly = dst.is_some_and(|d| d.fs.read_only());
        dialog::window("extract", tr!("Extract")).icon(Role::Unpack).min_width(560.0).danger(production).show(ctx, |ui| {
            let names: Vec<&str> = f.archives.iter().map(|a| a.name.as_str()).collect();
            let quoted = names.iter().map(|n| tr!("“{name}”", name = *n)).collect::<Vec<_>>().join(", ");
            dialog::lead(ui, tr!("{archives}  from  {location}", archives = quoted, location = &src_label));
            // Contents.
            let mut ok = true;
            let mut total = 0u64;
            let mut packed = 0u64;
            let mut encrypted = false;
            let mut raw: Vec<Vec<u8>> = Vec::new();
            let mut rejected = 0usize;
            let mut existing = 0usize;
            match &f.listings {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr!("Loading the archive contents…"));
                    });
                    ok = false;
                }
                Some(ls) => {
                    for (a, l) in f.archives.iter().zip(ls) {
                        match l {
                            Ok(l) => {
                                total += l.total_size;
                                packed += l.archive_size;
                                encrypted |= l.encrypted;
                                rejected += l.rejected.len();
                                raw.extend(l.raw_names.iter().cloned());
                                if let Some(d) = dst {
                                    existing += top_names(a, l, f.subfolder)
                                        .iter()
                                        .filter(|n| d.entries.iter().any(|e| &e.name == *n))
                                        .count();
                                }
                                if f.archives.len() > 1 {
                                    ui.label(trn!("“{archive}”: {n} item, {size}", "“{archive}”: {n} items, {size}", l.items.len(), archive = &a.name, size = human_size(l.total_size)));
                                } else {
                                    ui.label(trn!("{n} item, {size} when extracted", "{n} items, {size} when extracted", l.items.len(), size = human_size(l.total_size)));
                                }
                            }
                            Err(e) if f.need_pw => {
                                let _ = e;
                                ui.label(tr!("“{archive}”: the file names are encrypted.", archive = &a.name));
                                ok = false;
                            }
                            Err(e) => {
                                dialog::warn(ui, tr!("“{archive}”: {error}", archive = &a.name, error = e), true);
                                ok = false;
                            }
                        }
                    }
                }
            }
            if rejected > 0 {
                dialog::warn(
                    ui,
                    trn!(
                        "⚠ {n} item has an unsafe path (outside the target) – it will not be extracted and will be listed in the summary.",
                        "⚠ {n} items have an unsafe path (outside the target) – they will not be extracted and will be listed in the summary.",
                        rejected
                    ),
                    false,
                );
            }
            // Password.
            if encrypted || f.need_pw {
                let heading = tr!("Password");
                dialog::group(ui, &heading, |ui| {
                    dialog::form(ui, "extract-pw", |ui| {
                        ui.label(tr!("Password:"));
                        ui.horizontal(|ui| {
                            let r = crate::secret::edit(ui, &mut f.password, !f.show_pw, &tr!("archive password"), 240.0);
                            if f.focus_pw {
                                r.request_focus();
                                f.focus_pw = false;
                            }
                            ui.checkbox(&mut f.show_pw, trc!("password", "Show"));
                            if f.need_pw && ui.button(tr!("Load contents")).clicked() {
                                relist = true;
                            }
                        });
                        ui.end_row();
                    });
                    if let Some(e) = &f.pw_error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                });
                if f.password.is_empty() {
                    ok = false;
                }
            }
            // Encoding of old zip names.
            if !raw.is_empty() {
                let heading = tr!("Encoding");
                dialog::group(ui, &heading, |ui| {
                    ui.label(tr!("The file names are not in UTF-8. Encoding:"));
                    let before = f.encoding;
                    ui.horizontal(|ui| {
                        for e in Encoding::ALL {
                            ui.radio_value(&mut f.encoding, e, e.label());
                        }
                    });
                    if f.encoding != before {
                        relist = true;
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        for r in raw.iter().take(5) {
                            ui.monospace(rolid_archive::decode_name(r, f.encoding));
                        }
                    });
                });
            }
            // Limits.
            let free_cap = f.free.map(|a| a.saturating_sub(KEEP_FREE));
            let cap = free_cap.map_or(max_bytes, |c| c.min(max_bytes));
            if f.listings.is_some() && total > cap {
                dialog::warn(
                    ui,
                    if free_cap.is_some_and(|c| c < max_bytes) {
                        tr!("Not enough space on the disk: {needed} needed, {free} free (1 GB is kept in reserve).", needed = human_size(total), free = human_size(f.free.unwrap_or(0)))
                    } else {
                        tr!("The extracted archive would take {size} – more than the set limit of {limit}.", size = human_size(total), limit = human_size(max_bytes))
                    },
                    true,
                );
                ok = false;
            }
            let ratio = total / packed.max(1);
            let suspicious = f.listings.is_some() && ratio > max_ratio;
            if suspicious {
                dialog::warn(ui, tr!("⚠ The compression ratio {ratio} : 1 is suspiciously high (possibly an archive bomb).", ratio = ratio), false);
                ui.checkbox(&mut f.ratio_ok, tr!("Extract anyway"));
                if !f.ratio_ok {
                    ok = false;
                }
            }
            let heading = tr!("Target");
            dialog::group(ui, &heading, |ui| {
                ui.horizontal(|ui| {
                    if ui.radio(f.to_other, tr!("to the other panel  {location}", location = &other_label)).clicked() {
                        f.to_other = true;
                        space = true;
                    }
                    if dst.is_some_and(|d| d.fs.is_remote()) {
                        dialog::help(ui, tr!("The files are extracted into a temporary folder on this computer and uploaded through the transfer queue."));
                    }
                });
                if ui.radio(!f.to_other, tr!("to this panel  {location}", location = &src_label)).clicked() {
                    f.to_other = false;
                    space = true;
                }
                if dst_search {
                    dialog::warn(ui, tr!("The target panel shows search results – open a folder in it first."), false);
                    ok = false;
                }
                if dst_readonly {
                    dialog::warn(ui, tr!("An archive cannot be written to – choose another target."), false);
                    ok = false;
                }
                let sub = ui.checkbox(&mut f.subfolder, tr!("Each archive into its own subfolder"));
                if sub.changed() {
                    f.subfolder_touched = true;
                }
            });
            let heading = tr!("Existing files");
            dialog::group(ui, &heading, |ui| {
                if existing > 0 {
                    dialog::warn(
                        ui,
                        trn!(
                            "⚠ {n} item already exists in the target. What to do with existing files?",
                            "⚠ {n} items already exist in the target. What to do with existing files?",
                            existing
                        ),
                        false,
                    );
                }
                for (c, l) in [
                    (Conflict::Skip, tr!("Skip existing")),
                    (Conflict::Newer, tr!("Overwrite only older (by modification date)")),
                    (Conflict::Overwrite, tr!("Overwrite")),
                ] {
                    let sel = f.conflict == Some(c);
                    let t = if sel && c == Conflict::Overwrite {
                        egui::RichText::new(tr!("{option}  ⚠ existing files will be overwritten", option = &l)).color(WARN_ORANGE).strong()
                    } else {
                        egui::RichText::new(l)
                    };
                    if ui.radio(sel, t).clicked() {
                        f.conflict = Some(c);
                    }
                }
                if f.conflict.is_none() {
                    dialog::warn(ui, tr!("Choose what to do with existing files – nothing is preselected."), false);
                    ok = false;
                }
            });
            if production {
                dialog::warn(ui, tr!("⚠ The target is a PRODUCTION server – Enter is disabled, confirm with the button."), true);
            }
            ok &= dst.is_some();
            // While the names are encrypted, Enter loads the contents with the password.
            let pw_enter = f.need_pw && dialog::enter(ui);
            let b = dialog::buttons(ui, [
                dialog::btn(Role::Unpack, tr!("Extract")).default(!production && !f.need_pw).enabled(ok),
                dialog::cancel(),
            ]);
            if pw_enter {
                relist = true;
            } else if ok && b[0] {
                let d = dst.unwrap();
                start = Some(ExtractPlan {
                    from: f.from,
                    archives: f.archives.clone(),
                    temp: f.temp.clone(),
                    password: (!f.password.is_empty()).then(|| SecretString::from(f.password.to_string())),
                    encoding: f.encoding,
                    limits: Limits {
                        max_total: cap,
                        max_entries: 0,
                        max_ratio: if f.ratio_ok { None } else { Some(max_ratio) },
                    },
                    exist: exist_of(f.conflict.unwrap_or(Conflict::Skip)),
                    conflict: f.conflict.unwrap_or(Conflict::Skip),
                    subfolder: f.subfolder,
                    dfs: d.fs.clone(),
                    dpath: d.path.clone(),
                    dst_label: d.display_label(),
                    to,
                    production,
                    links: vec![],
                });
                close = true;
            }
            if b[1] {
                close = true;
            }
        });
        if close {
            self.dialog = Dialog::None;
        }
        if relist {
            self.relist();
        } else if space {
            self.update_free_space();
        }
        if let Some(mut plan) = start {
            plan.limits.max_entries = self.settings.unpack_items();
            plan.links = [self.pane_link(plan.from), self.pane_link(plan.to)].into_iter().flatten().collect();
            self.start_extract(plan);
        }
    }

    /// The panel opposite to the one the open archive dialog came from.
    fn other_ref_of_dialog(&self) -> PaneRef {
        match &self.dialog {
            Dialog::Archive(d) => match d.as_ref() {
                ArchiveDialog::Extract(f) => self.other(f.from),
                _ => self.active_ref(),
            },
            _ => self.active_ref(),
        }
    }

    fn start_extract(&mut self, plan: ExtractPlan) {
        let local_out = plan.dfs.is_local_disk();
        let (out_dir, temp) = if local_out {
            (PathBuf::from(&plan.dpath), plan.temp.clone())
        } else {
            let t = match plan.temp.clone().map(Ok).unwrap_or_else(new_temp) {
                Ok(t) => t,
                Err(e) => {
                    self.last_error = Some(tr!("Cannot create a temporary folder: {error}", error = e));
                    return;
                }
            };
            let d = t.path.join("out");
            let _ = std::fs::remove_dir_all(&d);
            if let Err(e) = std::fs::create_dir_all(&d) {
                self.last_error = Some(format!("{}: {e}", d.display()));
                return;
            }
            (d, Some(t))
        };
        let title = match plan.archives.as_slice() {
            [a] => tr!("Extract {archive}", archive = &a.name),
            v => trn!("Extract {n} archive", "Extract {n} archives", v.len()),
        };
        let archives = plan.archives.clone();
        let (password, encoding, limits, exist, subfolder) =
            (plan.password.clone(), plan.encoding, plan.limits, plan.exist, plan.subfolder);
        let out2 = out_dir.clone();
        let multi = archives.len() > 1;
        self.archive_job(
            title.clone(),
            move |ctl| {
                let opts = ReadOpts { password, encoding, max_entries: limits.max_entries };
                // Totals for the progress (from the listings, cheap for zip / 7z).
                let mut items = 0u64;
                let mut bytes = 0u64;
                for a in &archives {
                    if let Ok(l) = rolid_archive::list(&a.path, a.format, &opts) {
                        items += l.items.len() as u64;
                        bytes += l.total_size;
                    }
                }
                ctl.tick(Tick::Total { items, bytes });
                let mut created: Vec<PathBuf> = Vec::new();
                let mut failures: Vec<Failure> = Vec::new();
                let mut left = limits;
                for a in &archives {
                    let dest = if subfolder {
                        let d = out2.join(subfolder_name(&a.name));
                        if !d.exists() {
                            if let Err(e) = std::fs::create_dir(&d) {
                                rolid_archive::cleanup(&created);
                                return Err(e.into());
                            }
                            created.push(d.clone());
                        }
                        d
                    } else {
                        out2.clone()
                    };
                    match rolid_archive::extract(&a.path, a.format, &dest, &opts, left, exist, None, ctl) {
                        Ok(rep) => {
                            left.max_total = left.max_total.saturating_sub(rep.bytes);
                            created.extend(rep.created);
                            failures.extend(rep.failures.into_iter().map(|(p, e)| Failure {
                                path: if multi { format!("{}: {}", a.name, item_name(&p)) } else { item_name(&p) },
                                error: e,
                            }));
                        }
                        Err(e) => {
                            // This archive cleaned up after itself; the earlier ones too.
                            rolid_archive::cleanup(&created);
                            return Err(e);
                        }
                    }
                }
                Ok(failures)
            },
            move |a: &mut App, res| match res {
                Ok(failures) => {
                    if !failures.is_empty() {
                        let id = a.id();
                        a.drop.reports.push(super::drop_ui::Report { id, title, failures });
                    }
                    if local_out {
                        a.refresh_all(&[plan.to]);
                    } else {
                        a.upload_extracted(plan, out_dir, temp);
                    }
                }
                Err(e) if e.is_cancel() => {}
                Err(rolid_archive::Error::BadPassword | rolid_archive::Error::NeedPassword) => {
                    // Ask again (the extraction removed what it created).
                    a.archives_ready(Purpose::Extract, plan.from, plan.archives, plan.temp);
                    if let Dialog::Archive(d) = &mut a.dialog {
                        if let ArchiveDialog::Extract(f) = d.as_mut() {
                            f.pw_error = Some(tr!("Wrong password, try again."));
                            f.focus_pw = true;
                            f.to_other = plan.to != plan.from;
                        }
                    }
                }
                Err(e) => a.last_error = Some(format!("{title}: {}", t(&e.text()))),
            },
        );
    }

    fn upload_extracted(&mut self, plan: ExtractPlan, out_dir: PathBuf, temp: Option<Arc<TempDir>>) {
        let up = Upload {
            dfs: plan.dfs,
            dpath: plan.dpath,
            conflict: plan.conflict,
            dst_label: plan.dst_label,
            to: plan.to,
            production: plan.production,
            links: plan.links,
        };
        self.upload_dir(up, out_dir, temp);
    }

    /// Upload everything in a local (temporary) folder through the queue.
    pub(super) fn upload_dir(&mut self, up: Upload, out_dir: PathBuf, temp: Option<Arc<TempDir>>) {
        let dir = out_dir.to_string_lossy().into_owned();
        let mut items: Vec<Target> = std::fs::read_dir(&out_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                        Target { dir: dir.clone(), label: name.clone(), name, size }
                    })
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        if items.is_empty() {
            return;
        }
        let what = super::archive_ui::items_label(&items);
        self.spawn_copy(CopySpec {
            sfs: Arc::new(LocalFs),
            dfs: up.dfs,
            dpath: up.dpath,
            items,
            conflict: up.conflict,
            title: format!("{what} → {}", up.dst_label),
            refresh: vec![up.to],
            links: up.links,
            auto_retry: !up.production || up.conflict == Conflict::Skip,
            state: None,
            keep_going: true,
            pre_failed: Vec::new(),
            then: None,
            temp,
            moving: false,
            verify: false,
            target_name: None,
        });
    }

    // ------------------------------------------------------------------ test

    fn run_test(&mut self, archives: Vec<ArchFile>, temp: Option<Arc<TempDir>>, password: Option<SecretString>) {
        let title = match archives.as_slice() {
            [a] => tr!("Test {archive}", archive = &a.name),
            v => trn!("Test {n} archive", "Test {n} archives", v.len()),
        };
        let list = archives.clone();
        let pw2 = password.clone();
        self.archive_job(
            title,
            move |ctl| {
                let opts = ReadOpts { password: pw2, ..ReadOpts::new() };
                let mut out = Vec::new();
                for a in &list {
                    match rolid_archive::test(&a.path, a.format, &opts, ctl) {
                        Ok(r) => out.push((a.name.clone(), r.bad, None)),
                        Err(e @ (rolid_archive::Error::NeedPassword | rolid_archive::Error::BadPassword | rolid_archive::Error::Canceled)) => {
                            return Err(e)
                        }
                        Err(e) => out.push((a.name.clone(), Vec::new(), Some(e.text()))),
                    }
                }
                Ok(out)
            },
            move |a: &mut App, res| match res {
                Ok(lines) => {
                    drop(temp);
                    a.dialog = Dialog::Archive(Box::new(ArchiveDialog::TestResult(TestResult { lines })));
                }
                Err(e) if e.is_cancel() => {}
                Err(e) => {
                    let error = matches!(e, rolid_archive::Error::BadPassword).then(|| tr!("Wrong password, try again."));
                    a.dialog = Dialog::Archive(Box::new(ArchiveDialog::TestPassword(TestPassword {
                        archives,
                        temp,
                        password: Zeroizing::default(),
                        error,
                    })));
                }
            },
        );
    }

    pub(super) fn test_dialogs_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Archive(d) = &mut self.dialog else { return };
        let mut close = false;
        let mut run: Option<(Vec<ArchFile>, Option<Arc<TempDir>>, SecretString)> = None;
        match d.as_mut() {
            ArchiveDialog::TestPassword(f) => {
                dialog::window("test-pw", tr!("Test archive")).icon(Role::Unlock).show(ctx, |ui| {
                    dialog::lead(ui, tr!("The archive is password protected."));
                    dialog::form(ui, "test-pw-form", |ui| {
                        ui.label(tr!("Password:"));
                        let r = crate::secret::edit(ui, &mut f.password, true, &tr!("archive password"), 280.0);
                        r.request_focus();
                        ui.end_row();
                    });
                    if let Some(e) = &f.error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    let ok = !f.password.is_empty();
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, tr!("Test")).default(true).enabled(ok), dialog::cancel()]);
                    if b[0] && ok {
                        run = Some((f.archives.clone(), f.temp.clone(), SecretString::from(f.password.to_string())));
                        close = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            ArchiveDialog::TestResult(r) => {
                dialog::window("test-result", tr!("Test archive")).icon(Role::Info).show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (name, bad, err) in &r.lines {
                            if let Some(e) = err {
                                dialog::warn(ui, format!("✖ {}", tr!("“{archive}”: {error}", archive = name, error = e)), true);
                            } else if let [(p, e)] = bad.as_slice() {
                                if p == rolid_archive::WHOLE_ARCHIVE {
                                    dialog::warn(ui, format!("✖ {}", tr!("“{archive}”: the archive is damaged – {error}", archive = name, error = e)), true);
                                } else {
                                    dialog::warn(ui, format!("✖ {}", tr!("“{archive}”: damaged item {item} – {error}", archive = name, item = item_name(p), error = e)), true);
                                }
                            } else if bad.is_empty() {
                                ui.colored_label(crate::look::GREEN, format!("✔ {}", tr!("“{archive}”: the archive is OK.", archive = name)));
                            } else {
                                dialog::warn(
                                    ui,
                                    format!("✖ {}", trn!("“{archive}”: {n} damaged item", "“{archive}”: {n} damaged items", bad.len(), archive = name)),
                                    true,
                                );
                                for (p, e) in bad.iter().take(200) {
                                    ui.label(format!("    {} – {}", item_name(p), t(e)));
                                }
                            }
                        }
                    });
                    let b = dialog::buttons(ui, [dialog::close().default(true)]);
                    if b[0] {
                        close = true;
                    }
                });
            }
            _ => {}
        }
        if close {
            self.dialog = Dialog::None;
        }
        if let Some((a, t, pw)) = run {
            self.run_test(a, t, Some(pw));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subfolder_names() {
        assert_eq!(subfolder_name("photos.zip"), "photos");
        assert_eq!(subfolder_name("backup.tar.gz"), "backup");
        assert_eq!(subfolder_name("x.7z"), "x");
    }
}
