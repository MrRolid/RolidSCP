//! The clickable path in the row above the listing: every part of the path
//! is its own target. A click opens that folder, the middle button opens it
//! in a new tab, and a part under the mouse is highlighted. A path too long
//! for the row is shortened from the left with "…" (which opens the
//! drop-down of the whole path); the last part always stays whole. The path
//! is edited by a double click on the empty part of the row, the pencil at
//! its end or Ctrl+G.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Vec2};

use super::navbar::{path_tree, Go};
use super::{Pane, PaneEvent};
use crate::icons;

/// One part of the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub go: Go,
    /// A "/" is drawn before it (not after the root "/").
    pub sep_before: bool,
}

/// The parts of `path`; inside an archive (`archive` = its name and the
/// folder it is in) the folder of the archive comes first, then the archive
/// and the path in it. A path that is not absolute ("~" before the server
/// told where home is) is one part.
pub fn crumbs(path: &str, archive: Option<(&str, &str)>) -> Vec<Crumb> {
    if archive.is_none() && !path.starts_with('/') {
        return vec![Crumb { label: path.to_string(), go: Go::Path(path.to_string()), sep_before: false }];
    }
    let nodes = path_tree(path, archive);
    let root = archive.is_none();
    nodes
        .into_iter()
        .enumerate()
        .map(|(i, n)| Crumb { label: n.label, go: n.go, sep_before: i > 1 || (i == 1 && !root) })
        .collect()
}

/// The first part shown when the parts (`items` = width and whether a "/"
/// goes before it) must fit `avail`: 0 = the whole path, otherwise "…/" and
/// the parts from the returned index. The last part always stays.
pub fn first_shown(items: &[(f32, bool)], sep: f32, ellipsis: f32, avail: f32) -> usize {
    let n = items.len();
    if n == 0 {
        return 0;
    }
    let width_from = |i: usize| -> f32 {
        items[i..].iter().enumerate().map(|(k, (w, s))| w + if k > 0 && *s { sep } else { 0.0 }).sum()
    };
    if width_from(0) <= avail {
        return 0;
    }
    (1..n).find(|&i| ellipsis + sep + width_from(i) <= avail).unwrap_or(n - 1)
}

/// Width of the pencil button at the end of the row.
pub const PENCIL_W: f32 = 22.0;

impl Pane {
    /// The parts of this panel's path.
    pub fn crumbs(&self) -> Vec<Crumb> {
        match &self.archive {
            Some(st) => crumbs(&self.path, Some((&st.fs.name, &st.back_path))),
            None => crumbs(&self.path, None),
        }
    }

    /// Draw the clickable path into `area` (the pencil at its right end).
    pub(super) fn crumbs_ui(&mut self, ui: &mut egui::Ui, area: Rect, fg: Color32, events: &mut Vec<PaneEvent>) {
        let font = FontId::monospace(crate::fonts::fs(13.0));
        let painter = ui.painter_at(area);
        // --- The pencil ---
        let pencil = Rect::from_min_max(egui::pos2(area.right() - PENCIL_W, area.top() + 2.0), egui::pos2(area.right(), area.bottom() - 2.0));
        let presp = ui.interact(pencil, ui.scope_id().with("path-pencil"), Sense::click());
        if presp.hovered() {
            painter.rect_filled(pencil, 3.0, fg.gamma_multiply(0.15));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        painter.text(pencil.center(), Align2::CENTER_CENTER, icons::PENCIL, icons::font(15.0), fg);
        if presp.on_hover_text(tr!("Edit the path (Ctrl+G, or double-click the empty part of the row)")).clicked() {
            events.push(PaneEvent::Focus);
            self.path_edit = Some(self.path.clone());
            return;
        }
        let text_area = Rect::from_min_max(area.min, egui::pos2(pencil.left() - 4.0, area.max.y));
        // --- Search results: the query, not clickable ---
        if let Some(sv) = &self.search {
            let text = format!("🔍 {}", tr!("{query} in {path}", query = sv.describe(), path = &self.path));
            ui.painter_at(text_area).text(egui::pos2(text_area.left(), area.center().y), Align2::LEFT_CENTER, text, font, fg);
            return;
        }
        // --- The parts ---
        let list = self.crumbs();
        let gal = |t: &str| painter.layout_no_wrap(t.to_string(), font.clone(), fg);
        let sep_w = gal("/").size().x;
        let ell_w = gal("…").size().x;
        let pad = 2.0;
        let galleys: Vec<_> = list.iter().map(|c| gal(&c.label)).collect();
        let items: Vec<(f32, bool)> = galleys.iter().zip(&list).map(|(g, c)| (g.size().x + 2.0 * pad, c.sep_before)).collect();
        let first = first_shown(&items, sep_w, ell_w + 2.0 * pad, text_area.width());
        let clip = ui.painter_at(text_area);
        let cy = area.center().y;
        let h = (area.height() - 4.0).max(10.0);
        let mut x = text_area.left();
        let tree_hint = tr!("Folders on the path and disks (Alt+↓)");
        if first > 0 {
            let r = Rect::from_center_size(egui::pos2(x + (ell_w + 2.0 * pad) / 2.0, cy), Vec2::new(ell_w + 2.0 * pad, h));
            let resp = ui.interact(r, ui.scope_id().with("crumb-more"), Sense::click());
            if resp.hovered() {
                clip.rect_filled(r, 3.0, fg.gamma_multiply(0.15));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            clip.text(r.center(), Align2::CENTER_CENTER, "…", font.clone(), fg);
            if resp.on_hover_text(&tree_hint).clicked() {
                events.push(PaneEvent::Focus);
                self.open_tree(events);
            }
            x = r.right();
            clip.text(egui::pos2(x, cy), Align2::LEFT_CENTER, "/", font.clone(), fg.gamma_multiply(0.7));
            x += sep_w;
        }
        let last = list.len().saturating_sub(1);
        let hint = tr!("Click: open · middle button: open in a new tab");
        for i in first..list.len() {
            let c = &list[i];
            if i > first && c.sep_before {
                clip.text(egui::pos2(x, cy), Align2::LEFT_CENTER, "/", font.clone(), fg.gamma_multiply(0.7));
                x += sep_w;
            }
            let w = items[i].0;
            let r = Rect::from_min_size(egui::pos2(x, cy - h / 2.0), Vec2::new(w, h));
            let resp = ui.interact(r, ui.scope_id().with(("crumb", i)), Sense::click());
            if resp.hovered() {
                clip.rect_filled(r, 3.0, fg.gamma_multiply(0.15));
                clip.hline(r.x_range().shrink(pad), r.bottom() - 2.0, egui::Stroke::new(1.0, fg));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let g = galleys[i].clone();
            clip.galley(egui::pos2(x + pad, cy - g.size().y / 2.0), g, fg);
            x += w;
            let target = match &c.go {
                Go::Path(p) => p.clone(),
                Go::LeaveArchive => self.archive.as_ref().map(|st| st.back_path.clone()).unwrap_or_default(),
            };
            let resp = resp.on_hover_text(format!("{target}\n{hint}"));
            if resp.clicked() {
                events.push(PaneEvent::Focus);
                if i == last {
                    events.push(PaneEvent::Refresh);
                } else {
                    // The cursor lands on the folder we came from.
                    let from = list.get(i + 1).map(|n| n.label.clone());
                    events.push(match &c.go {
                        Go::Path(p) => PaneEvent::Jump { path: p.clone(), select: from },
                        Go::LeaveArchive => PaneEvent::LeaveArchive,
                    });
                }
            } else if resp.middle_clicked() {
                events.push(PaneEvent::NewTab(c.go.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(c: &[Crumb]) -> Vec<(&str, bool)> {
        c.iter().map(|c| (c.label.as_str(), c.sep_before)).collect()
    }

    #[test]
    fn parts_of_a_local_remote_and_archive_path() {
        let c = crumbs("/home/alex/Backups", None);
        assert_eq!(labels(&c), vec![("/", false), ("home", false), ("alex", true), ("Backups", true)]);
        assert_eq!(c[0].go, Go::Path("/".into()));
        assert_eq!(c[2].go, Go::Path("/home/alex".into()));
        assert_eq!(c[3].go, Go::Path("/home/alex/Backups".into()));
        // The root alone.
        assert_eq!(labels(&crumbs("/", None)), vec![("/", false)]);
        // A server path not resolved yet.
        let c = crumbs("~", None);
        assert_eq!(labels(&c), vec![("~", false)]);
        // Inside an archive: its folder (leaves the archive), the archive, the path in it.
        let c = crumbs("build.tar.zst/dist/js", Some(("build.tar.zst", "/srv/releases")));
        assert_eq!(labels(&c), vec![("releases", false), ("build.tar.zst", true), ("dist", true), ("js", true)]);
        assert_eq!(c[0].go, Go::LeaveArchive);
        assert_eq!(c[1].go, Go::Path("build.tar.zst".into()));
        assert_eq!(c[3].go, Go::Path("build.tar.zst/dist/js".into()));
    }

    #[test]
    fn long_path_is_shortened_from_the_left() {
        // "/" home / alex / Backups, each 10 wide, "/" 2, "…" 4.
        let items = [(10.0, false), (10.0, false), (10.0, true), (10.0, true)];
        assert_eq!(first_shown(&items, 2.0, 4.0, 44.0), 0, "the whole path fits");
        assert_eq!(first_shown(&items, 2.0, 4.0, 43.9), 1, "…/home/alex/Backups");
        assert_eq!(first_shown(&items, 2.0, 4.0, 28.0), 2, "…/alex/Backups");
        assert_eq!(first_shown(&items, 2.0, 4.0, 16.0), 3, "…/Backups");
        // Not even the last part fits: it is still the one shown.
        assert_eq!(first_shown(&items, 2.0, 4.0, 3.0), 3);
        assert_eq!(first_shown(&[], 2.0, 4.0, 3.0), 0);
        assert_eq!(first_shown(&[(50.0, false)], 2.0, 4.0, 3.0), 0, "one part: shown, never “…”");
    }
}
