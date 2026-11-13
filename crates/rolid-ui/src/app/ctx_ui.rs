//! Context-menu actions on files: copying names / paths / sftp:// addresses to
//! the clipboard, the permissions dialog (with a background job), and
//! "Open" / "Open with" (application chooser).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use rolid_config::{OpenWith, RemoteTarget};
use rolid_core::copy::Progress;
use rolid_core::perms::{self, PermEdit, Plan, Recursion, Scope, Tri};
use rolid_core::{Entry, EntryKind, Vfs, VfsResult};
use rolid_edit::apps::AppDb;

use super::{Action, App, Dialog, Job, Msg, PaneRef};
use crate::dialog;
use crate::look::Role;
use crate::pane::{Opener, Target};

// ------------------------------------------------------------------ clipboard

/// Names, one per line.
pub fn names_text(items: &[Target]) -> String {
    items.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join("\n")
}

/// Full paths, one per line.
pub fn paths_text(fs: &dyn Vfs, items: &[Target]) -> String {
    items.iter().map(|t| fs.join(&t.dir, &t.name)).collect::<Vec<_>>().join("\n")
}

/// `sftp://user@host:port/path` – never with a password. The user and the path
/// are percent-encoded (RFC 3986), an IPv6 host goes in brackets, the port is
/// always written (so the address works whatever the default is).
pub fn sftp_url(t: &RemoteTarget, path: &str) -> String {
    let host = if t.host.contains(':') && !t.host.starts_with('[') { format!("[{}]", t.host) } else { t.host.clone() };
    let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    format!("sftp://{}@{host}:{}{}", encode(&t.user, false), t.port, encode(&path, true))
}

fn encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') || (keep_slash && b == b'/');
        if ok {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// sftp:// addresses, one per line.
pub fn urls_text(fs: &dyn Vfs, t: &RemoteTarget, items: &[Target]) -> String {
    items.iter().map(|i| sftp_url(t, &fs.join(&i.dir, &i.name))).collect::<Vec<_>>().join("\n")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipKind {
    Names,
    Paths,
    Urls,
}

// ---------------------------------------------------------------- permissions

pub struct PermsForm {
    at: PaneRef,
    title: String,
    items: Vec<(String, Entry)>,
    edit: PermEdit,
    /// What the items have now (for "unchanged" and the summary line).
    current: String,
    octal: String,
    octal_err: Option<String>,
    has_dirs: bool,
    recursive: bool,
    rec: Recursion,
    file_oct: String,
    dir_oct: String,
    rec_err: Option<String>,
}

pub struct PermsConfirm {
    at: PaneRef,
    fs: Arc<dyn Vfs>,
    title: String,
    plan: Plan,
    production: bool,
}

/// "Open with → Other application…": pick an application or type a command,
/// optionally remember it for the extension.
pub struct OpenWithForm {
    at: PaneRef,
    name: String,
    ext: Option<String>,
    apps: Vec<(String, String)>,
    /// The type's default application (marked "default").
    default: Option<String>,
    /// Index into `apps`, or `None` = own command.
    choice: Option<usize>,
    command: String,
    remember: bool,
    remembered: Option<String>,
    error: Option<String>,
}

pub struct PermsPlanned {
    pub id: u64,
    pub at: PaneRef,
    pub fs: Arc<dyn Vfs>,
    pub title: String,
    pub production: bool,
    pub result: VfsResult<Plan>,
}

fn parse_mode3(s: &str) -> Option<u32> {
    let s = s.trim();
    (s.len() == 3 && s.bytes().all(|b| (b'0'..=b'7').contains(&b))).then(|| u32::from_str_radix(s, 8).ok()).flatten()
}

/// Lowercase extension used as the key for remembered applications.
pub fn ext_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_lowercase())
}

/// "Change permissions of N files and M folders?" counts (only items that really change).
pub fn change_counts(plan: &Plan) -> (u64, u64) {
    let ch = plan.steps.iter().filter(|s| s.old != s.new);
    let (mut f, mut d) = (0, 0);
    for s in ch {
        if s.is_dir {
            d += 1;
        } else {
            f += 1;
        }
    }
    (f, d)
}

pub fn summary_question(files: u64, dirs: u64) -> String {
    match (files, dirs) {
        (_, 0) => trn!("Change permissions of {n} file?", "Change permissions of {n} files?", files),
        (0, _) => trn!("Change permissions of {n} folder?", "Change permissions of {n} folders?", dirs),
        _ => tr!(
            "Change permissions of {files} and {dirs}?",
            files = rolid_i18n::rolid_text::txtnc!("perms", "{n} file", "{n} files", files),
            dirs = rolid_i18n::rolid_text::txtnc!("perms", "{n} folder", "{n} folders", dirs)
        ),
    }
}

impl App {
    /// Targets of the panel with their entries (marked ones or the cursor's).
    fn target_entries(&self, at: PaneRef) -> Vec<(Target, Entry)> {
        let Some(p) = self.pane(at) else { return Vec::new() };
        p.target_items()
            .into_iter()
            .filter_map(|t| {
                let e = match p.hit(&t.label) {
                    Some(h) => h.entry.clone(),
                    None => p.entries.iter().find(|e| e.name == t.label)?.clone(),
                };
                Some((t, e))
            })
            .collect()
    }

    /// Copy names / paths / sftp addresses of the targets to the clipboard
    /// (CLIPBOARD only – egui never writes PRIMARY).
    pub(super) fn copy_to_clipboard(&mut self, at: PaneRef, kind: ClipKind) {
        let Some(p) = self.pane(at) else { return };
        let items = p.target_items();
        if items.is_empty() {
            return;
        }
        let text = match kind {
            ClipKind::Names => names_text(&items),
            ClipKind::Paths => paths_text(p.fs.as_ref(), &items),
            ClipKind::Urls => match &p.remote {
                Some(r) => urls_text(p.fs.as_ref(), &r.target, &items),
                None => {
                    self.last_error = Some(tr!("Only files on a server have an sftp:// address."));
                    return;
                }
            },
        };
        self.ctx.copy_text(text);
        tracing::info!("copied to the clipboard: {} items ({kind:?})", items.len());
    }

    /// "Permissions…" for the targets of `at`.
    pub(super) fn ask_perms(&mut self, at: PaneRef) {
        if self.refuse_read_only(at, &tr!("Cannot change permissions in an archive")) {
            return;
        }
        let items = self.target_entries(at);
        if items.is_empty() {
            return;
        }
        let Some(p) = self.pane(at) else { return };
        if p.offline {
            return;
        }
        let modes: Vec<u32> = items.iter().filter_map(|(_, e)| e.mode.map(|m| m & 0o7777)).collect();
        if modes.is_empty() {
            self.last_error = Some(tr!("Permissions are not available here (the system does not report them)."));
            return;
        }
        let edit = PermEdit::from_modes(&modes);
        let title = if items.len() == 1 { tr!("“{name}”", name = items[0].0.label.as_str()) } else { trn!("{n} item", "{n} items", items.len()) };
        let current = if modes.iter().all(|m| *m == modes[0]) {
            format!("{} ({:04o})", perms::symbolic(modes[0]), modes[0])
        } else {
            tr!("mixed: {octal}", octal = edit.octal())
        };
        let fs = p.fs.clone();
        self.dialog = Dialog::Perms(Box::new(PermsForm {
            at,
            title,
            has_dirs: items.iter().any(|(_, e)| e.kind == EntryKind::Dir),
            items: items.into_iter().map(|(t, e)| (fs.join(&t.dir, &t.name), e)).collect(),
            octal: edit.octal(),
            edit,
            current,
            octal_err: None,
            recursive: false,
            rec: Recursion::default(),
            file_oct: "644".into(),
            dir_oct: "755".into(),
            rec_err: None,
        }));
    }

    /// Plan in the background (folders' contents can take a while on a server).
    fn start_perms_plan(&mut self, f: PermsForm) {
        let Some(p) = self.pane(f.at) else { return };
        let fs = p.fs.clone();
        let production = p.remote.as_ref().is_some_and(|r| r.production);
        let id = self.id();
        let cancel = Arc::new(AtomicBool::new(false));
        let title = tr!("Permissions: {name}", name = f.title.as_str());
        self.jobs.push(Job {
            id,
            title: title.clone(),
            progress: Arc::new(Mutex::new(Progress { started: Some(Instant::now()), ..Default::default() })),
            total_bytes: 0,
            cancel: cancel.clone(),
            note: Arc::new(Mutex::new(Some(tr!("reading folder contents…")))),
            count: None,
            totals: Arc::default(),
        });
        let (at, items, edit, rec) = (f.at, f.items, f.edit, f.recursive.then_some(f.rec));
        self.spawn(async move {
            let result = perms::plan(fs.as_ref(), &items, &edit, rec, &cancel).await;
            Msg::PermsPlanned(Box::new(PermsPlanned { id, at, fs, title, production, result }))
        });
    }

    pub(super) fn on_perms_planned(&mut self, m: PermsPlanned) {
        let canceled = self.jobs.iter().any(|j| j.id == m.id && j.cancel.load(std::sync::atomic::Ordering::Relaxed));
        self.jobs.retain(|j| j.id != m.id);
        match m.result {
            Err(_) if canceled => {}
            Err(e) => self.last_error = Some(format!("{}: {}", m.title, rolid_i18n::t(&e.text()))),
            Ok(plan) if plan.changes() == 0 && plan.failures.is_empty() => {
                self.last_error = None;
                tracing::info!("{}: permissions already match, nothing changes", m.title);
                self.note_info(&tr!("The permissions already match; nothing changes."));
            }
            Ok(plan) => {
                self.dialog = Dialog::PermsConfirm(Box::new(PermsConfirm {
                    at: m.at,
                    fs: m.fs,
                    title: m.title,
                    plan,
                    production: m.production,
                }));
            }
        }
    }

    /// A short note in the status line (reuses the message line, not red-worthy).
    fn note_info(&mut self, text: &str) {
        self.last_error = Some(format!("ℹ {text}")); // i18n-ok: info symbol, not a word
    }

    fn start_perms_run(&mut self, c: PermsConfirm) {
        let id = self.id();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress { started: Some(Instant::now()), ..Default::default() }));
        self.jobs.push(Job {
            id,
            title: c.title.clone(),
            progress: progress.clone(),
            total_bytes: 0,
            cancel: cancel.clone(),
            note: Arc::new(Mutex::new(None)),
            count: Some(c.plan.steps.len() as u64),
            totals: Arc::default(),
        });
        self.job_kinds.insert(id, super::JobKind::Perms);
        if let Some(p) = self.pane_mut(c.at) {
            p.marked.clear();
        }
        let ctx = self.ctx.clone();
        let at = c.at;
        self.spawn(async move {
            let cb = move |done: u64| {
                progress.lock().unwrap().files_done = done;
                ctx.request_repaint_after(Duration::from_millis(100));
            };
            let (result, failures) = perms::run(c.fs.as_ref(), &c.plan, &cancel, &cb).await;
            let mut all = c.plan.failures.clone();
            all.extend(failures);
            Msg::JobDone { id, refresh: vec![at], result, failures: all, retry: None, then: None }
        });
    }

    // -------------------------------------------------------------- opening

    /// "Open" a file: the application remembered for its extension, otherwise
    /// the system default.
    pub(super) fn open_file(&mut self, at: PaneRef) {
        let name = self.pane(at).and_then(|p| p.current_target()).map(|(t, _)| t.name).unwrap_or_default();
        let opener = ext_of(&name)
            .and_then(|e| self.settings.open_with.get(&e).cloned())
            .map(|o| match o {
                OpenWith::Desktop(id) => Opener::Desktop(id),
                OpenWith::Command(c) => Opener::Command(c),
            })
            .unwrap_or(Opener::System);
        self.open_with(at, opener);
    }

    pub(super) fn ask_open_with(&mut self, at: PaneRef) {
        let Some((t, e)) = self.pane(at).and_then(|p| p.current_target()) else { return };
        if e.is_dir_like() {
            return;
        }
        let db = AppDb::system();
        let apps: Vec<(String, String)> =
            db.apps_for(&db.mime_for(&t.name)).into_iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let ext = ext_of(&t.name);
        let default = db.default_for(&db.mime_for(&t.name)).map(|a| a.id.clone());
        let remembered = ext.as_ref().and_then(|x| self.settings.open_with.get(x)).map(|o| match o {
            OpenWith::Desktop(id) => db.app(id).map(|a| a.name.clone()).unwrap_or_else(|| id.clone()),
            OpenWith::Command(c) => c.clone(),
        });
        let choice = if apps.is_empty() { None } else { Some(0) };
        self.dialog = Dialog::OpenWith(Box::new(OpenWithForm {
            at,
            name: t.name,
            ext,
            apps,
            default,
            choice,
            command: String::new(),
            remember: false,
            remembered,
            error: None,
        }));
    }

    // -------------------------------------------------------------- dialogs

    pub(super) fn ctx_dialog_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut take = false;
        let mut act: Option<Action> = None;
        match &mut self.dialog {
            Dialog::Perms(f) => {
                let title = tr!("Permissions: {name}", name = f.title.as_str());
                dialog::window("perms-dialog", title).icon(Role::Edit).show(ctx, |ui| {
                    let r = perms_form_ui(ui, f);
                    let ok_enabled = f.octal_err.is_none() && (!f.recursive || f.rec_err.is_none());
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Ok, tr!("Continue")).default(!r.octal_focused).enabled(ok_enabled),
                        dialog::cancel(),
                    ]);
                    if b[0] {
                        take = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            Dialog::PermsConfirm(c) => {
                dialog::window("perms-confirm", c.title.clone()).icon(Role::Edit).danger(c.production).show(ctx, |ui| {
                    let (files, dirs) = change_counts(&c.plan);
                    if files + dirs > 0 {
                        dialog::lead(ui, RichText::new(summary_question(files, dirs)).strong());
                    } else {
                        dialog::lead(ui, tr!("Nothing changes."));
                    }
                    let unchanged = c.plan.steps.len() as u64 - files - dirs;
                    if unchanged > 0 {
                        dialog::note(
                            ui,
                            trn!(
                                "{n} item already has the requested permissions.",
                                "{n} items already have the requested permissions.",
                                unchanged
                            ),
                        );
                    }
                    if !c.plan.failures.is_empty() {
                        dialog::warn(
                            ui,
                            trn!(
                                "{n} item will be skipped (listed in the summary at the end):",
                                "{n} items will be skipped (listed in the summary at the end):",
                                c.plan.failures.len()
                            ),
                            true,
                        );
                        for fl in c.plan.failures.iter().take(5) {
                            dialog::note(ui, format!("  {} – {}", fl.path, rolid_i18n::t(&fl.error)));
                        }
                    }
                    if c.production {
                        dialog::warn(ui, tr!("⚠ PRODUCTION SERVER: Enter is disabled, confirm with the button."), true);
                    }
                    let go = files + dirs > 0;
                    let label = if c.production { tr!("Change permissions on production") } else { tr!("Change permissions") };
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, label).default(!c.production).enabled(go), dialog::cancel()]);
                    if b[0] {
                        take = true;
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            Dialog::OpenWith(f) => {
                let title = tr!("Open “{name}” with an application", name = f.name.as_str());
                dialog::window("open-with", title).icon(Role::Open).min_width(480.0).show(ctx, |ui| {
                    if let Some(r) = f.remembered.clone() {
                        ui.horizontal(|ui| {
                            dialog::note(ui, tr!("Remembered: {app}", app = r.as_str()));
                            if ui.small_button(tr!("Forget")).clicked() {
                                if let Some(x) = f.ext.clone() {
                                    act = Some(Box::new(move |a: &mut App| {
                                        a.settings.open_with.remove(&x);
                                        a.save_settings();
                                    }));
                                }
                                f.remembered = None;
                            }
                        });
                    }
                    let heading = tr!("Application");
                    dialog::group(ui, &heading, |ui| {
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for (i, (id, name)) in f.apps.iter().enumerate() {
                                let t = if f.default.as_ref() == Some(id) { tr!("{name} (default)", name = name.as_str()) } else { name.clone() };
                                if ui.radio(f.choice == Some(i), t).clicked() {
                                    f.choice = Some(i);
                                }
                            }
                            if f.apps.is_empty() {
                                dialog::note(ui, tr!("The system has no application registered for this file type."));
                            }
                        });
                        if ui.radio(f.choice.is_none(), tr!("Other application (custom command):")).clicked() {
                            f.choice = None;
                        }
                        ui.horizontal(|ui| {
                            let r = ui.add_enabled(
                                f.choice.is_none(),
                                egui::TextEdit::singleline(&mut f.command).hint_text(tr!("e.g. gimp {{file}}")).desired_width(dialog::width() - 60.0),
                            );
                            if r.changed() {
                                f.error = None;
                            }
                            dialog::help(ui, tr!("{{file}} = path of the file (added at the end when missing)."));
                        });
                    });
                    if let Some(x) = &f.ext {
                        ui.checkbox(&mut f.remember, tr!("Always open .{ext} files this way (“Open”, Enter)", ext = x.as_str()));
                    }
                    if let Some(e) = &f.error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    let b = dialog::buttons(ui, [dialog::btn(Role::Open, trc!("verb", "Open")).default(true), dialog::cancel()]);
                    if b[0] {
                        let chosen = match f.choice {
                            Some(i) => f.apps.get(i).map(|(id, _)| OpenWith::Desktop(id.clone())),
                            None if f.command.trim().is_empty() => {
                                f.error = Some(tr!("Enter a command."));
                                None
                            }
                            None => match rolid_edit::editor::split_command(f.command.trim()) {
                                Ok(w) if !w.is_empty() => Some(OpenWith::Command(f.command.trim().to_string())),
                                Ok(_) => None,
                                Err(e) => {
                                    f.error = Some(rolid_i18n::t(&e));
                                    None
                                }
                            },
                        };
                        if let Some(o) = chosen {
                            let (at, remember, ext) = (f.at, f.remember, f.ext.clone());
                            act = Some(Box::new(move |a: &mut App| {
                                if let (true, Some(x)) = (remember, ext) {
                                    a.settings.open_with.insert(x, o.clone());
                                    a.save_settings();
                                }
                                let opener = match o {
                                    OpenWith::Desktop(id) => Opener::Desktop(id),
                                    OpenWith::Command(c) => Opener::Command(c),
                                };
                                a.open_with(at, opener);
                            }));
                            close = true;
                        }
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            _ => {}
        }
        if take {
            match std::mem::replace(&mut self.dialog, Dialog::None) {
                Dialog::Perms(f) => self.start_perms_plan(*f),
                Dialog::PermsConfirm(c) => self.start_perms_run(*c),
                _ => {}
            }
        }
        if close {
            self.dialog = Dialog::None;
        }
        if let Some(a) = act {
            a(self);
        }
    }
}

struct FormResult {
    octal_focused: bool,
}

fn tri_box(ui: &mut egui::Ui, t: Tri, enabled: bool) -> bool {
    let mut v = t == Tri::On;
    let b = egui::Checkbox::without_text(&mut v).indeterminate(t == Tri::Keep);
    ui.add_enabled(enabled, b).clicked()
}

fn perms_form_ui(ui: &mut egui::Ui, f: &mut PermsForm) -> FormResult {
    dialog::lead(ui, tr!("Now: {perms}", perms = f.current.as_str()));
    let mut changed = false;
    let mut octal_focused = false;
    let heading = tr!("Permissions");
    dialog::group(ui, &heading, |ui| {
        egui::Grid::new("perm-grid").num_columns(4).spacing([18.0, 6.0]).show(ui, |ui| {
            ui.label("");
            for w in [trc!("permission", "Read"), trc!("permission", "Write"), trc!("permission", "Execute")] {
                ui.label(RichText::new(w).strong());
            }
            ui.end_row();
            for (who, wname) in [trc!("permission", "Owner"), trc!("permission", "Group"), trc!("permission", "Others")].into_iter().enumerate() {
                ui.label(RichText::new(wname).strong());
                for what in 0..3 {
                    let i = PermEdit::index(who, what);
                    if tri_box(ui, f.edit.bits[i], true) {
                        f.edit.toggle(i);
                        changed = true;
                    }
                }
                ui.end_row();
            }
            ui.label(trc!("permission", "Special"));
            for (i, name) in [(perms::SETUID, "setuid"), (perms::SETGID, "setgid"), (perms::STICKY, "sticky")] {
                ui.horizontal(|ui| {
                    tri_box(ui, f.edit.bits[i], false);
                    ui.weak(name);
                });
            }
            ui.end_row();
        });
        if changed {
            f.octal = f.edit.octal();
            f.octal_err = None;
        }
        ui.horizontal(|ui| {
            ui.label(tr!("Octal:"));
            let r = ui.add(egui::TextEdit::singleline(&mut f.octal).desired_width(60.0).char_limit(4));
            octal_focused = r.has_focus();
            if r.changed() {
                match f.edit.set_octal(&f.octal) {
                    Ok(()) => f.octal_err = None,
                    Err(e) => f.octal_err = Some(rolid_i18n::t(&e)),
                }
            }
            if r.lost_focus() && f.octal_err.is_none() {
                f.octal = f.edit.octal();
            }
            ui.weak(tr!("? = unchanged"));
            dialog::help(ui, tr!("“–” = unchanged (the items differ). setuid/setgid/sticky can only be changed with a 4-digit octal number."));
        });
        if let Some(e) = &f.octal_err {
            dialog::warn(ui, e.as_str(), true);
        }
    });
    if f.has_dirs {
        let heading = tr!("Folder contents");
        dialog::group(ui, &heading, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.recursive, tr!("Apply to folder contents too"));
                dialog::help(
                    ui,
                    tr!(
                        "Inside: files {file_mode} ({file_sym}), folders {dir_mode} ({dir_sym}). The selected items themselves get the permissions above. \
                         Symbolic links are neither followed nor changed; setuid/setgid/sticky inside are kept.",
                        file_mode = f.file_oct.as_str(),
                        file_sym = perms::symbolic(f.rec.file_mode),
                        dir_mode = f.dir_oct.as_str(),
                        dir_sym = perms::symbolic(f.rec.dir_mode)
                    ),
                );
            });
            if f.recursive {
                ui.horizontal(|ui| {
                    for (s, l) in [(Scope::Both, tr!("files and folders")), (Scope::Files, tr!("files only")), (Scope::Dirs, tr!("folders only"))] {
                        ui.radio_value(&mut f.rec.scope, s, l);
                    }
                });
                dialog::form(ui, "perm-rec", |ui| {
                    let files_on = f.rec.scope != Scope::Dirs;
                    ui.add_enabled(files_on, egui::Label::new(tr!("Files inside:")));
                    ui.add_enabled(files_on, egui::TextEdit::singleline(&mut f.file_oct).desired_width(44.0).char_limit(3));
                    ui.end_row();
                    let dirs_on = f.rec.scope != Scope::Files;
                    ui.add_enabled(dirs_on, egui::Label::new(tr!("Folders inside:")));
                    ui.add_enabled(dirs_on, egui::TextEdit::singleline(&mut f.dir_oct).desired_width(44.0).char_limit(3));
                    ui.end_row();
                });
                let mut bad = None;
                match parse_mode3(&f.file_oct) {
                    Some(m) => f.rec.file_mode = m,
                    None if f.rec.scope != Scope::Dirs => bad = Some(tr!("File permissions: three digits 0–7 (e.g. 644).")),
                    None => {}
                }
                match parse_mode3(&f.dir_oct) {
                    Some(m) => f.rec.dir_mode = m,
                    None if f.rec.scope != Scope::Files => bad = Some(tr!("Folder permissions: three digits 0–7 (e.g. 755).")),
                    None => {}
                }
                f.rec_err = bad;
                if let Some(e) = &f.rec_err {
                    dialog::warn(ui, e.as_str(), true);
                }
            }
        });
    }
    FormResult { octal_focused }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(dir: &str, name: &str) -> Target {
        Target { dir: dir.into(), name: name.into(), label: name.into(), size: 0 }
    }

    fn target(host: &str) -> RemoteTarget {
        RemoteTarget { profile: Some("p".into()), host: host.into(), port: 2222, user: "jan novák".into(), key_path: None }
    }

    #[test]
    fn clipboard_text_one_and_many() {
        let fs = rolid_local::LocalFs;
        assert_eq!(names_text(&[t("/srv", "a.txt")]), "a.txt");
        assert_eq!(names_text(&[t("/srv", "a.txt"), t("/srv", "b c")]), "a.txt\nb c");
        assert_eq!(paths_text(&fs, &[t("/srv/www", "a.txt"), t("/srv/www", "b")]), "/srv/www/a.txt\n/srv/www/b");
    }

    #[test]
    fn sftp_url_has_no_password_and_is_encoded() {
        let u = sftp_url(&target("example.org"), "/var/www/zpráva č.1.txt");
        assert_eq!(u, "sftp://jan%20nov%C3%A1k@example.org:2222/var/www/zpr%C3%A1va%20%C4%8D.1.txt");
        assert!(!u.contains("password") && u.matches('@').count() == 1);
        assert_eq!(sftp_url(&target("::1"), "/x"), "sftp://jan%20nov%C3%A1k@[::1]:2222/x");
        let mut tg = target("h");
        tg.user = "root".into();
        tg.port = 22;
        assert_eq!(sftp_url(&tg, "relative"), "sftp://root@h:22/relative");
        let fs = rolid_local::LocalFs;
        assert_eq!(urls_text(&fs, &tg, &[t("/a", "x"), t("/a", "y#z")]), "sftp://root@h:22/a/x\nsftp://root@h:22/a/y%23z");
    }

    #[test]
    fn summary_text_and_counts() {
        assert_eq!(summary_question(1, 0), "Change permissions of 1 file?");
        assert_eq!(summary_question(3, 2), "Change permissions of 3 files and 2 folders?");
        assert_eq!(summary_question(0, 1), "Change permissions of 1 folder?");
        let mut p = Plan::default();
        for (d, o, n) in [(false, 0o600, 0o644), (false, 0o644, 0o644), (true, 0o700, 0o755)] {
            p.steps.push(perms::Step { path: "x".into(), is_dir: d, old: o, new: n });
        }
        assert_eq!(change_counts(&p), (1, 1));
        assert_eq!(ext_of("Archive.TAR.GZ"), Some("gz".into()));
        assert_eq!(ext_of(".bashrc"), None);
        assert_eq!(parse_mode3("644"), Some(0o644));
        assert_eq!(parse_mode3("648"), None);
    }
}
