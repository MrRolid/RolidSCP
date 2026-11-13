//! Shift+F4: a new file, opened in the external editor the same way as F4.
//! Locally the empty file is created at once; on a server it is created by
//! the first save in the editor (see `open_new_remote`).

use eframe::egui;
use rolid_core::rename::{check_new_name, stem_chars};
use rolid_core::Entry;

use super::{App, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::Role;
use crate::pane::Target;

pub struct NewFile {
    at: PaneRef,
    /// The folder the file goes to.
    dir: String,
    name: String,
    /// The first frame: focus the field and select the name without its extension.
    started: bool,
}

/// What a typed name means in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameState {
    /// Not a valid name (the reason).
    Bad(String),
    /// A folder of this name exists.
    Folder,
    /// A file of this name exists: it is opened for editing.
    Exists,
    New,
}

/// Check a typed name against the rules of renaming and the folder's listing.
pub fn name_state(name: &str, listing: &[Entry]) -> NameState {
    if let Err(e) = check_new_name(name) {
        return NameState::Bad(rolid_i18n::t(&e));
    }
    match listing.iter().find(|e| e.name == name) {
        Some(e) if e.is_dir_like() => NameState::Folder,
        Some(_) => NameState::Exists,
        None => NameState::New,
    }
}

impl App {
    /// Shift+F4, File → New file…, the panel's context menu.
    pub(super) fn ask_new_file(&mut self) {
        let at = self.active_ref();
        let Some(p) = self.pane(at) else { return };
        if p.search.is_some() {
            self.last_error = Some(tr!("Cannot create a file in search results: open a folder first."));
            return;
        }
        if self.refuse_read_only(at, &tr!("Cannot create a file in an archive")) {
            return;
        }
        let Some(p) = self.pane(at) else { return };
        if p.offline {
            self.last_error = Some(tr!("The panel is not connected to the server."));
            return;
        }
        // As in Total Commander: the name under the cursor, to change.
        let name = p.current_name().unwrap_or_default();
        self.dialog = Dialog::NewFile(Box::new(NewFile { at, dir: p.path.clone(), name, started: false }));
    }

    pub(super) fn new_file_ui(&mut self, ctx: &egui::Context) {
        let Dialog::NewFile(d) = &mut self.dialog else { return };
        let listing: &[Entry] = super::find_pane(&self.sides, d.at).map(|p| p.entries.as_slice()).unwrap_or(&[]);
        let state = name_state(&d.name, listing);
        let mut go = false;
        let mut cancel = false;
        dialog::window("new-file", tr!("New file")).icon(Role::Edit).show(ctx, |ui| {
            dialog::lead(ui, tr!("The file is opened in the external editor (as with F4)."));
            dialog::form(ui, "new-file-form", |ui| {
                ui.label(tr!("Name:"));
                let id = egui::Id::unique("new-file-name");
                let r = ui.add(egui::TextEdit::singleline(&mut d.name).id(id).desired_width(dialog::width() - 80.0));
                if !d.started {
                    d.started = true;
                    r.request_focus();
                    let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                    let n = stem_chars(&d.name);
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                    egui::TextEdit::store_state(ui.ctx(), id, st);
                }
                ui.end_row();
            });
            match &state {
                NameState::Bad(e) if !d.name.is_empty() => dialog::warn(ui, e.clone(), true),
                NameState::Folder => dialog::warn(ui, tr!("A folder of this name already exists."), true),
                NameState::Exists => dialog::warn(ui, tr!("A file of this name already exists: it will be opened for editing."), false),
                _ => {}
            }
            let ok = matches!(state, NameState::New | NameState::Exists);
            let label = if state == NameState::Exists { tr!("Edit") } else { tr!("Create and edit") };
            let b = dialog::buttons(ui, [dialog::btn(Role::Edit, label).default(true).enabled(ok), dialog::cancel()]);
            go = b[0];
            cancel = b[1];
        });
        if cancel {
            self.dialog = Dialog::None;
        } else if go {
            let Dialog::NewFile(d) = std::mem::replace(&mut self.dialog, Dialog::None) else { return };
            self.create_new_file(d.at, d.dir, d.name);
        }
    }

    /// Check the name on the disk or the server (the listing may be old) and
    /// open the file in the editor.
    fn create_new_file(&mut self, at: PaneRef, dir: String, name: String) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let path = fs.join(&dir, &name);
        if fs.is_local_disk() {
            let lp = std::path::Path::new(&path);
            match std::fs::symlink_metadata(lp) {
                Ok(m) if lp.is_dir() || m.is_dir() => self.last_error = Some(tr!("A folder of this name already exists.")),
                Ok(_) => self.edit_existing(at, dir, name),
                Err(_) => {
                    let mut o = std::fs::OpenOptions::new();
                    o.write(true).create_new(true);
                    match o.open(lp) {
                        Ok(_) => {
                            tracing::info!("new local file created");
                            self.select_after_create(at, &dir, &name);
                            self.launch_editor(lp);
                        }
                        Err(e) => self.last_error = Some(tr!("Cannot create “{name}”: {error}", name = name.as_str(), error = e.to_string())),
                    }
                }
            }
            return;
        }
        self.spawn(async move {
            let r = fs.stat(&path).await;
            Msg::Then {
                job: None,
                f: Box::new(move |a: &mut App| match r {
                    Ok(e) if e.is_dir_like() => a.last_error = Some(tr!("A folder of this name already exists.")),
                    Ok(_) => a.edit_existing(at, dir, name),
                    Err(e) if e.is_not_found() => a.open_new_remote(at, dir, name),
                    Err(e) => a.last_error = Some(tr!("Cannot create “{name}”: {error}", name = name.as_str(), error = rolid_i18n::t(&e.text()))),
                }),
            }
        });
    }

    /// The name exists: edit that file as with F4.
    fn edit_existing(&mut self, at: PaneRef, dir: String, name: String) {
        let Some(p) = self.pane(at) else { return };
        let fs = p.fs.clone();
        let path = fs.join(&dir, &name);
        self.spawn(async move {
            let r = fs.stat(&path).await;
            Msg::Then {
                job: None,
                f: Box::new(move |a: &mut App| match r {
                    Ok(mut entry) => {
                        entry.name = name.clone();
                        let target = Target { dir: dir.clone(), name: name.clone(), label: name.clone(), size: entry.size };
                        a.select_after_create(at, &dir, &name);
                        a.edit_target(at, target, entry);
                    }
                    Err(e) => a.last_error = Some(tr!("Cannot open “{name}”: {error}", name = name.as_str(), error = rolid_i18n::t(&e.text()))),
                }),
            }
        });
    }

    /// Reload the panel with the cursor on the file (if it still shows that folder).
    fn select_after_create(&mut self, at: PaneRef, dir: &str, name: &str) {
        if self.pane(at).is_some_and(|p| p.path == dir && p.search.is_none()) {
            self.load(at, dir.to_string(), Some(name.to_string()));
        }
        for r in self.same_location(at) {
            if r != at {
                self.refresh(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(name: &str, dir: bool) -> Entry {
        let kind = if dir { rolid_core::EntryKind::Dir } else { rolid_core::EntryKind::File };
        Entry { name: name.into(), kind, link_to_dir: false, size: 0, modified: None, mode: None }
    }

    #[test]
    fn name_checks_as_for_renaming() {
        let l = vec![ent("docs", true), ent("a.txt", false)];
        assert_eq!(name_state("b.txt", &l), NameState::New);
        assert_eq!(name_state("notes with space.md", &l), NameState::New);
        assert_eq!(name_state("a.txt", &l), NameState::Exists);
        assert_eq!(name_state("A.txt", &l), NameState::New, "another letter case is another name");
        assert_eq!(name_state("docs", &l), NameState::Folder);
        for bad in ["", "   ", ".", "..", "a/b", "x\0y"] {
            assert!(matches!(name_state(bad, &l), NameState::Bad(_)), "{bad:?}");
        }
        assert!(matches!(name_state(&"é".repeat(200), &l), NameState::Bad(_)), "too long in bytes");
    }
}
