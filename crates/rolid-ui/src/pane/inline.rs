//! Renaming an item right in the list (F2, Shift+F6).
//!
//! The edit belongs to the item's full path (folder + name), never to a row
//! number: whatever reorders or reloads the list (sorting, refresh, the quick
//! filter, another folder or tab) ends the edit, and the field is only ever
//! drawn on the row of that very item.

use eframe::egui::{self, Key, Rect};

use super::{Pane, PaneEvent};

/// The item being renamed and the text typed so far.
#[derive(Debug, Clone)]
pub struct InlineRename {
    /// Folder of the item (the panel's folder when the edit started).
    pub dir: String,
    /// Its current name.
    pub name: String,
    pub buf: String,
    is_dir: bool,
    /// Focus and the selection were set up (first frame done).
    started: bool,
    /// Why the typed name cannot be used (shown under the field).
    error: Option<String>,
    /// The field was drawn in this frame.
    seen: bool,
}

impl InlineRename {
    pub fn new(dir: String, name: String, is_dir: bool) -> Self {
        Self { dir, buf: name.clone(), name, is_dir, started: false, error: None, seen: false }
    }

    /// Edit again with `buf` typed (after "Cancel" in a question about the new name).
    pub fn again(dir: String, name: String, is_dir: bool, buf: String) -> Self {
        Self { buf, ..Self::new(dir, name, is_dir) }
    }

    /// Characters selected when the edit starts: a file's name without its
    /// extension, a folder's whole name.
    fn selected(&self) -> usize {
        if self.is_dir {
            self.buf.chars().count()
        } else {
            rolid_core::rename::stem_chars(&self.buf)
        }
    }
}

impl Pane {
    /// F2 / Shift+F6: start renaming the item under the cursor.
    pub fn start_inline_rename(&mut self) -> Result<(), String> {
        if self.fs.read_only() {
            return Err(tr!("Cannot rename in an archive: the archive is read-only."));
        }
        if self.search.is_some() {
            return Err(tr!("Cannot rename in search results: open the folder with the file (Enter) and rename it there."));
        }
        if self.offline {
            return Err(tr!("The panel is not connected to the server."));
        }
        let Some(e) = self.current_entry() else {
            return Err(tr!("The cursor is not on a file or folder."));
        };
        self.rename = Some(InlineRename::new(self.path.clone(), e.name.clone(), e.is_dir_like()));
        self.scroll_to_cursor = true;
        Ok(())
    }

    /// The edit of `name` in `dir` continues with `buf` typed (the item stays
    /// where it is).
    pub fn resume_inline_rename(&mut self, dir: String, name: String, buf: String) {
        if self.path != dir || self.search.is_some() {
            return;
        }
        let Some(e) = self.entries.iter().find(|e| e.name == name) else { return };
        let is_dir = e.is_dir_like();
        self.select_name(&name);
        self.rename = Some(InlineRename::again(dir, name, is_dir, buf));
    }

    /// After drawing the list: an edit whose row was not drawn (scrolled away
    /// with the mouse wheel) ends, so it cannot hold the keyboard unseen.
    pub(super) fn end_unseen_rename(&mut self) {
        if let Some(r) = &mut self.rename {
            if r.started && !r.seen {
                self.rename = None;
            } else {
                r.seen = false;
            }
        }
    }

    /// The row of entry `i` is the one being renamed.
    pub(super) fn renaming_entry(&self, i: usize) -> bool {
        match (&self.rename, self.entries.get(i)) {
            (Some(r), Some(e)) => r.dir == self.path && r.name == e.name && self.search.is_none(),
            _ => false,
        }
    }

    /// The text field over the name in `rect`: Enter renames, Esc, a click
    /// elsewhere or any other loss of focus cancels.
    pub(super) fn inline_editor(&mut self, ui: &mut egui::Ui, rect: Rect, events: &mut Vec<PaneEvent>) {
        let Some(r) = &mut self.rename else { return };
        r.seen = true;
        let id = egui::Id::unique(("inline-rename", r.dir.as_str(), r.name.as_str()));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let edit = egui::TextEdit::singleline(&mut r.buf)
            .id(id)
            .font(crate::fonts::prop(13.5))
            .desired_width(rect.width())
            .margin(egui::Margin::symmetric(3, 0));
        let resp = child.add(edit);
        if !r.started {
            r.started = true;
            resp.request_focus();
            let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let n = r.selected();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            egui::TextEdit::store_state(ui.ctx(), id, st);
            return;
        }
        if resp.changed() {
            r.error = None;
        }
        if let Some(e) = &r.error {
            resp.show_tooltip_text(egui::RichText::new(format!("⚠ {e}")).color(egui::Color32::from_rgb(200, 30, 30)));
        }
        if resp.has_focus() {
            return;
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if !enter {
            self.rename = None;
            return;
        }
        match rolid_core::rename::check_new_name(&r.buf) {
            Err(e) => {
                // Stay in the field: the name can be fixed right away.
                r.error = Some(rolid_i18n::t(&e));
                resp.request_focus();
            }
            Ok(()) if r.buf == r.name => self.rename = None,
            Ok(()) => {
                events.push(PaneEvent::Rename { dir: r.dir.clone(), from: r.name.clone(), to: r.buf.clone() });
                self.rename = None;
            }
        }
    }
}
