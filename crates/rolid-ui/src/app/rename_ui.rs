//! Rename (F2, Shift+F6) and move (F6).
//!
//! Renaming is typed right in the list (see `pane::inline`); here it is
//! carried out: a production server asks first, an existing target asks
//! again (a folder is never overwritten), and afterwards the cursor stays on
//! the item, the other marks stay, and tabs or edited files (F4) inside a
//! renamed folder follow it.
//!
//! A move on the same disk or server is a rename of each item. Between disks
//! or servers it is a copy through the transfer queue; the sources are deleted
//! only after the whole copy succeeded, and only files found in the target
//! with the same size (see `CopyJob::moving`, `CopySpec::moving`).

use std::sync::Arc;

use eframe::egui;
use rolid_core::copy::Conflict;
use rolid_core::rename::{self, MoveStep, RenameOutcome};
use rolid_core::Vfs;

use super::{conflict_choice, preset_conflict, App, CopySpec, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::{btn, Role};
use crate::pane::{Pane, Target};

/// Why a typed rename needs an answer first.
pub(super) enum AskKind {
    Production,
    /// Something with the new name exists.
    Exists { target_dir: bool, src_dir: bool },
}

/// A question about a rename typed in the list.
pub(super) struct RenameAsk {
    at: PaneRef,
    dir: String,
    from: String,
    to: String,
    kind: AskKind,
    production: bool,
}

/// The F6 dialog.
pub(super) struct MoveForm {
    pub(super) from: PaneRef,
    pub(super) to: PaneRef,
    pub(super) items: Vec<Target>,
    /// Target path (for one item including its name).
    pub(super) target: String,
    pub(super) conflict: Option<Conflict>,
    pub(super) src_production: bool,
    pub(super) dst_production: bool,
    pub(super) focus: bool,
    pub(super) error: Option<String>,
}

/// Backend of a panel for "the same place" checks: `"local"` or the server's
/// `user@host:port`; `None` inside an archive.
pub(super) fn backend_key(p: &Pane) -> Option<String> {
    if p.archive.is_some() || p.fs.read_only() {
        return None;
    }
    if p.fs.is_local_disk() {
        return Some("local".into());
    }
    p.remote.as_ref().map(|r| r.target.key())
}

impl App {
    // ------------------------------------------------------------- rename

    /// F2, Shift+F6, "Rename" in the menus: edit the name in the list.
    pub(super) fn start_rename_active(&mut self) {
        let at = self.active_ref();
        let Some(p) = self.pane_mut(at) else { return };
        if let Err(e) = p.start_inline_rename() {
            self.last_error = Some(e);
        }
    }

    /// Enter in the list: the new name was typed.
    pub(super) fn on_rename_typed(&mut self, at: PaneRef, dir: String, from: String, to: String) {
        let Some(p) = self.pane(at) else { return };
        let production = p.remote.as_ref().is_some_and(|r| r.production);
        if production {
            self.dialog = Dialog::Rename(Box::new(RenameAsk { at, dir, from, to, kind: AskKind::Production, production }));
        } else {
            self.run_rename(at, dir, from, to, false);
        }
    }

    fn run_rename(&mut self, at: PaneRef, dir: String, from: String, to: String, overwrite: bool) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let production = p.remote.as_ref().is_some_and(|r| r.production);
        let src_dir = p.entries.iter().find(|e| e.name == from).is_some_and(|e| e.is_dir_like());
        self.spawn(async move {
            let r = rename::rename_entry(fs.as_ref(), &dir, &from, &to, overwrite).await;
            Msg::Then {
                job: None,
                f: Box::new(move |a: &mut App| match r {
                    Ok(RenameOutcome::Done) => a.after_rename(at, &dir, &from, &to),
                    Ok(RenameOutcome::Unchanged) => {}
                    Ok(RenameOutcome::Exists { dir: target_dir }) => {
                        a.dialog = Dialog::Rename(Box::new(RenameAsk {
                            at,
                            dir,
                            from,
                            to,
                            kind: AskKind::Exists { target_dir, src_dir },
                            production,
                        }));
                    }
                    Err(e) => {
                        a.last_error = Some(tr!("Renaming “{from}” to “{to}” failed: {error}", from = &from, to = &to, error = e.text()));
                        a.refresh_all(&[at]);
                    }
                }),
            }
        });
    }

    /// The rename is done: keep the cursor and marks, and let tabs and edited
    /// files inside a renamed folder follow it.
    fn after_rename(&mut self, at: PaneRef, dir: &str, from: &str, to: &str) {
        let Some(p) = self.pane_mut(at) else { return };
        if p.marked.remove(from) {
            p.marked.insert(to.to_string());
        }
        let (old, new) = (p.fs.join(dir, from), p.fs.join(dir, to));
        let key = backend_key(p);
        let path = p.path.clone();
        let same: Vec<PaneRef> = self.same_location(at).into_iter().filter(|r| *r != at).collect();
        self.load(at, path, Some(to.to_string()));
        for r in same {
            if let Some(q) = self.pane_mut(r) {
                if q.marked.remove(from) {
                    q.marked.insert(to.to_string());
                }
            }
            self.refresh(r);
        }
        if let Some(key) = key {
            self.relocate(&key, &old, &new, Some(at));
        }
    }

    /// `old` became `new` on backend `key`: panels showing a folder inside it
    /// go to the new place, edited files in it are uploaded there.
    fn relocate(&mut self, key: &str, old: &str, new: &str, except: Option<PaneRef>) {
        let mut moved = Vec::new();
        for side in 0..2 {
            for t in &mut self.sides[side].tabs {
                let r = PaneRef { tab: t.id, side };
                if Some(r) == except || t.pane.search.is_some() || backend_key(&t.pane).as_deref() != Some(key) {
                    continue;
                }
                if let Some(np) = rename::moved_path(&t.pane.path, old, new) {
                    moved.push((r, np));
                }
            }
        }
        for (r, np) in moved {
            if let Some(p) = self.pane_mut(r) {
                p.path = np.clone();
            }
            self.load(r, np, None);
        }
        let warnings = self.edits_relocate(key, old, new);
        if !warnings.is_empty() {
            self.last_error = Some(warnings.join(" "));
        }
    }

    pub(super) fn rename_dialog_ui(&mut self, ctx: &egui::Context) {
        match &mut self.dialog {
            Dialog::Rename(_) => self.rename_ask_ui(ctx),
            Dialog::Move(_) => self.move_dialog_ui(ctx),
            _ => {}
        }
    }

    fn rename_ask_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Rename(q) = &self.dialog else { return };
        let mut go: Option<bool> = None; // Some(overwrite)
        let mut cancel = false;
        dialog::window("rename-ask", tr!("Rename")).icon(Role::Edit).danger(q.production).show(ctx, |ui| {
            dialog::lead(ui, tr!("“{from}”  →  “{to}”", from = &q.from, to = &q.to));
            if q.production {
                dialog::warn(ui, tr!("⚠ This is a PRODUCTION server."), true);
            }
            match q.kind {
                AskKind::Production => {
                    dialog::note(ui, tr!("Really rename on the production server? (Enter does not confirm here.)"));
                    let b = dialog::buttons(ui, [dialog::btn(Role::Edit, tr!("Rename")), dialog::cancel()]);
                    if b[0] {
                        go = Some(false);
                    }
                    if b[1] {
                        cancel = true;
                    }
                }
                AskKind::Exists { target_dir, src_dir } if target_dir || src_dir => {
                    let exists = if target_dir {
                        tr!("“{name}” already exists in this folder (a folder).", name = &q.to)
                    } else {
                        tr!("“{name}” already exists in this folder (a file).", name = &q.to)
                    };
                    dialog::warn(ui, exists, false);
                    dialog::note(
                        ui,
                        if src_dir {
                            tr!("A folder cannot be renamed over an existing item. Choose a different name.")
                        } else {
                            tr!("A file cannot overwrite a folder. Choose a different name.")
                        },
                    );
                    let b = dialog::buttons(ui, [dialog::cancel().default(true)]);
                    if b[0] {
                        cancel = true;
                    }
                }
                AskKind::Exists { .. } => {
                    dialog::warn(ui, tr!("File “{name}” already exists in this folder. Overwrite it?", name = &q.to), false);
                    dialog::note(ui, tr!("An overwritten file cannot be restored. Enter does not confirm here."));
                    let b = dialog::buttons(ui, [dialog::btn(Role::Overwrite, tr!("Overwrite")), dialog::cancel()]);
                    if b[0] {
                        go = Some(true);
                    }
                    if b[1] {
                        cancel = true;
                    }
                }
            }
        });
        if go.is_none() && !cancel {
            return;
        }
        let Dialog::Rename(q) = std::mem::replace(&mut self.dialog, Dialog::None) else { return };
        let RenameAsk { at, dir, from, to, kind, .. } = *q;
        match (go, kind) {
            // Production confirmed: now the rename itself (it may still ask about a conflict).
            (Some(false), AskKind::Production) => self.run_rename(at, dir, from, to, false),
            (Some(overwrite), _) => self.run_rename(at, dir, from, to, overwrite),
            // Cancel: back in the field with the typed name, to change it.
            (None, _) => {
                if let Some(p) = self.pane_mut(at) {
                    p.resume_inline_rename(dir, from, to);
                }
            }
        }
    }

    // --------------------------------------------------------------- move

    /// F6: move the marked items (or the one under the cursor) to the other panel.
    pub(super) fn ask_move(&mut self) {
        let from = self.active_ref();
        let to = self.other(from);
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        let items = src.target_items();
        let dest = dst.path.clone();
        self.open_move(from, to, dest, items);
    }

    /// The Move dialog for `items` of panel `from` into folder `dest` of panel `to`.
    pub(super) fn open_move(&mut self, from: PaneRef, to: PaneRef, dest: String, items: Vec<Target>) {
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        if src.archive.is_some() || src.fs.read_only() {
            self.last_error = Some(tr!("Cannot move out of an archive (it is read-only), only copy out of it: opening Copy (F5)."));
            self.open_copy(from, to, dest, items);
            return;
        }
        if dst.search.is_some() {
            self.last_error = Some(tr!("The target panel shows search results: open a folder in it first."));
            return;
        }
        if self.refuse_read_only(to, &tr!("Cannot move into an archive")) {
            return;
        }
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        if items.is_empty() {
            return;
        }
        let target = if items.len() == 1 { dst.fs.join(&dest, &items[0].name) } else { dest };
        let form = MoveForm {
            from,
            to,
            items,
            target,
            conflict: preset_conflict(self.settings.copy_preset()),
            src_production: src.remote.as_ref().is_some_and(|r| r.production),
            dst_production: dst.remote.as_ref().is_some_and(|r| r.production),
            focus: true,
            error: None,
        };
        self.dialog = Dialog::Move(Box::new(form));
    }

    fn move_dialog_ui(&mut self, ctx: &egui::Context) {
        let (dst_entries, dst_path, dst_label, dst_fs) = match &self.dialog {
            Dialog::Move(f) => match self.pane(f.to) {
                Some(d) => (d.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), d.path.clone(), d.display_label(), d.fs.clone()),
                None => return,
            },
            _ => return,
        };
        let history = match &self.dialog {
            Dialog::Move(f) => self.targets_on(f.to),
            _ => return,
        };
        let Dialog::Move(f) = &mut self.dialog else { return };
        let mut ok = false;
        let mut cancel = false;
        let mut browse: Option<(PaneRef, String)> = None;
        let production = f.src_production || f.dst_production;
        let one = f.items.len() == 1;
        dialog::window("move", tr!("Move")).icon(Role::Move).min_width(560.0).danger(production).show(ctx, |ui| {
            let lead = if one {
                tr!("Move “{name}” to {disk}:", name = f.items[0].label.as_str(), disk = dst_label.as_str())
            } else {
                trn!("Move {n} item to {disk}:", "Move {n} items to {disk}:", f.items.len(), disk = dst_label.as_str())
            };
            ui.horizontal(|ui| {
                dialog::lead(ui, lead);
                dialog::help(
                    ui,
                    tr!("On the same disk or server the items are only renamed. Otherwise they are copied through the transfer queue \
                         and the source is deleted only after a successful finish (a file that does not have the same size in the target stays)."),
                );
            });
            ui.horizontal(|ui| {
                let w = dialog::width() - if one { 165.0 } else { 140.0 };
                let r = dialog::history_field(ui, "move-dest", &mut f.target, &history, w);
                if f.focus {
                    r.request_focus();
                    f.focus = false;
                }
                if r.changed() {
                    f.error = None;
                }
                if btn(Role::Folder, tr!("Browse…")).ui(ui).clicked() {
                    let start = if one { dst_fs.parent(&f.target).unwrap_or_else(|| f.target.clone()) } else { f.target.clone() };
                    browse = Some((f.to, start));
                }
                if one {
                    dialog::help(ui, tr!("The end of the path is the name the item will have in the target (you can change it). A slash at the end = into that folder under the same name."));
                }
            });
            if let Some(e) = &f.error {
                dialog::warn(ui, format!("⚠ {e}"), true);
            }
            let warn = match (f.src_production, f.dst_production) {
                (true, true) => Some(tr!("⚠ Both the source and the target are PRODUCTION servers.")),
                (true, false) => Some(tr!("⚠ The source is a PRODUCTION server: the files will disappear from it.")),
                (false, true) => Some(tr!("⚠ The target is a PRODUCTION server.")),
                _ => None,
            };
            if let Some(w) = warn {
                dialog::warn(ui, w, true);
            }
            if production {
                dialog::note(ui, tr!("On a production server Enter does not confirm: click Move."));
            }
            // Existing items (only known when the target is the panel's folder).
            let single = one.then(|| f.items[0].name.as_str());
            let overwrite = match rename::split_target(&dst_path, &f.target, single) {
                Ok((dir, name)) if dir == dst_path => f
                    .items
                    .iter()
                    .filter(|t| {
                        let n = name.as_deref().unwrap_or(&t.name);
                        dst_entries.iter().any(|e| e == n)
                    })
                    .count(),
                _ => 0,
            };
            let heading = tr!("Existing files");
            dialog::group(ui, &heading, |ui| {
                if overwrite > 0 {
                    dialog::warn(
                        ui,
                        trn!(
                            "⚠ {n} item already exists in the target. What to do with existing files?",
                            "⚠ {n} items already exist in the target. What to do with existing files?",
                            overwrite
                        ),
                        false,
                    );
                }
                conflict_choice(ui, &mut f.conflict);
            });
            let b = dialog::buttons(ui, [
                dialog::btn(Role::Move, tr!("Move")).default(!production).enabled(f.conflict.is_some()),
                dialog::cancel(),
            ]);
            ok = b[0];
            cancel = b[1];
        });
        if cancel {
            self.dialog = Dialog::None;
            return;
        }
        if let Some((t, start)) = browse {
            self.open_folder_pick(t, start);
            return;
        }
        if !ok {
            return;
        }
        let single = (f.items.len() == 1).then(|| f.items[0].name.clone());
        match rename::split_target(&dst_path, &f.target, single.as_deref()) {
            Err(e) => f.error = Some(rolid_i18n::t(&e)),
            Ok((dir, name)) => {
                let Dialog::Move(f) = std::mem::replace(&mut self.dialog, Dialog::None) else { return };
                let conflict = f.conflict.unwrap_or_default();
                self.start_move(f.from, f.to, f.items, dir, name, conflict);
            }
        }
    }

    /// Move `items` into folder `dpath` of panel `to` (a single item may get `name`).
    fn start_move(&mut self, from: PaneRef, to: PaneRef, items: Vec<Target>, dpath: String, name: Option<String>, conflict: Conflict) {
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        let (sfs, dfs) = (src.fs.clone(), dst.fs.clone());
        // The same disk or server: a rename on the server side.
        let same = match (backend_key(src), backend_key(dst)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let src_key = backend_key(src);
        if let Some(l) = self.location_at(to, &dpath) {
            self.places.remember_target(l);
        }
        if let Some(p) = self.pane_mut(from) {
            p.marked.clear();
        }
        if !same {
            self.move_by_copy(from, to, items, dpath, name, conflict);
            return;
        }
        self.spawn(async move {
            let mut needs_copy = Vec::new();
            let mut moved: Vec<(String, String)> = Vec::new();
            let mut error = None;
            for t in &items {
                let dn = name.clone().unwrap_or_else(|| t.name.clone());
                match rename::move_by_rename(sfs.as_ref(), &t.dir, &t.name, &dpath, &dn).await {
                    Ok(MoveStep::Moved) => moved.push((sfs.join(&t.dir, &t.name), dfs.join(&dpath, &dn))),
                    Ok(MoveStep::NeedsCopy) => needs_copy.push(t.clone()),
                    Err(e) => {
                        error = Some(tr!("Moving “{name}”: {error}", name = &t.name, error = e.text()));
                        break;
                    }
                }
            }
            Msg::Then {
                job: None,
                f: Box::new(move |a: &mut App| {
                    if let Some(e) = error {
                        a.last_error = Some(e);
                    }
                    if let Some(key) = &src_key {
                        for (old, new) in &moved {
                            a.relocate(key, old, new, None);
                        }
                    }
                    for r in [from, to] {
                        for s in a.same_location(r) {
                            a.refresh(s);
                        }
                    }
                    if !needs_copy.is_empty() {
                        a.move_by_copy(from, to, needs_copy, dpath, name, conflict);
                    }
                }),
            }
        });
    }

    /// Copy through the transfer queue, then delete the sources (see `MoveOpts`).
    fn move_by_copy(&mut self, from: PaneRef, to: PaneRef, items: Vec<Target>, dpath: String, name: Option<String>, conflict: Conflict) {
        let (Some(src), Some(dst)) = (self.pane(from), self.pane(to)) else { return };
        let (sfs, dfs): (Arc<dyn Vfs>, Arc<dyn Vfs>) = (src.fs.clone(), dst.fs.clone());
        let dst_label = dst.display_label();
        let production = dst.remote.as_ref().is_some_and(|r| r.production);
        let title = if items.len() == 1 {
            tr!("Moving {name} → {target}", name = name.as_deref().unwrap_or(&items[0].name), target = &dst_label)
        } else {
            trn!("Moving {n} item → {target}", "Moving {n} items → {target}", items.len(), target = &dst_label)
        };
        let links = [self.pane_link(from), self.pane_link(to)].into_iter().flatten().collect();
        self.spawn_copy(CopySpec {
            sfs,
            dfs,
            dpath,
            target_name: if items.len() == 1 { name } else { None },
            items,
            conflict,
            title,
            refresh: vec![to, from],
            links,
            auto_retry: !production || conflict == Conflict::Skip,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: None,
            temp: None,
            moving: true,
        });
    }
}
