//! Dragging files inside the application: rows of a panel onto the other
//! panel, a folder row, the ".." row or a tab. The drop opens the ordinary
//! Copy dialog (Move with Shift) with the target filled in; every rule of
//! those dialogs applies. Esc cancels the drag.
//!
//! On X11 the rows can also be dragged out of the window to another
//! application (see `drag_out`); coming back continues the drag here.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::drop_ui::{hit, paint_target, PaneZone, Refusal, Spot};
use super::{App, PaneRef, TabId};
use crate::pane::{Pane, Target};

/// Holding the dragged files over a tab this long switches to it.
const TAB_HOLD: Duration = Duration::from_millis(800);

/// A drag in progress.
pub struct InnerDrag {
    pub from: PaneRef,
    pub items: Vec<Target>,
    /// The tab under the cursor and since when.
    over_tab: Option<(TabId, Instant)>,
}

/// Where the dragged files are over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAt {
    /// A panel: its folder, or a folder row (or "..") in it.
    Pane(PaneRef, Spot),
    /// A tab: the folder it shows.
    Tab(PaneRef),
}

impl DropAt {
    fn place(self) -> (PaneRef, Spot) {
        match self {
            DropAt::Pane(at, spot) => (at, spot),
            DropAt::Tab(at) => (at, Spot::Here),
        }
    }
}

/// The target under `pos`: a tab first, then a panel (and a row in it).
pub fn target_at(zones: &[PaneZone], tabs: &[(PaneRef, Rect)], pos: Pos2) -> Option<DropAt> {
    if let Some((at, _)) = tabs.iter().find(|(_, r)| r.contains(pos)) {
        return Some(DropAt::Tab(*at));
    }
    hit(zones, pos).map(|(at, spot)| DropAt::Pane(at, spot))
}

/// What a drop does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The items are already there: nothing happens.
    Same,
    /// A folder onto itself or into a folder inside it.
    IntoItself,
    Refused(Refusal),
    Copy,
    Move,
    /// Move asked out of an archive: only copying is possible.
    CopyOnly,
}

/// Which disk a panel shows: on the same disk, the same folder is the same place.
pub fn disk_key(p: &Pane) -> String {
    match (&p.archive, &p.remote) {
        (Some(st), _) => format!("archive:{:p}", std::sync::Arc::as_ptr(&st.fs)),
        (None, Some(r)) => format!("sftp:{}", r.target.key()),
        (None, None) => "local".into(),
    }
}

fn norm(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/" } else { t }
}

/// Decide a drop of `items` (from a panel on disk `src_disk`) into folder
/// `dest` on disk `dst_disk`.
pub fn outcome(
    src_disk: &str,
    src_read_only: bool,
    items: &[Target],
    dst_disk: &str,
    dest: &str,
    refusal: Option<Refusal>,
    shift: bool,
) -> Outcome {
    if let Some(r) = refusal {
        return Outcome::Refused(r);
    }
    if src_disk == dst_disk {
        let dest = norm(dest);
        if items.iter().all(|t| norm(&t.dir) == dest) {
            return Outcome::Same;
        }
        let inside = items.iter().any(|t| {
            let path = format!("{}/{}", norm(&t.dir).trim_end_matches('/'), t.name);
            dest == path || dest.starts_with(&format!("{path}/"))
        });
        if inside {
            return Outcome::IntoItself;
        }
    }
    match (shift, src_read_only) {
        (false, _) => Outcome::Copy,
        (true, true) => Outcome::CopyOnly,
        (true, false) => Outcome::Move,
    }
}

impl App {
    /// A row of panel `at` started being dragged.
    pub(super) fn start_inner_drag(&mut self, at: PaneRef, name: &str) {
        let Some(p) = self.pane(at) else { return };
        let items = p.drag_items(name);
        if items.is_empty() {
            return;
        }
        tracing::info!("drag inside the app: {} items", items.len());
        self.inner_drag = Some(InnerDrag { from: at, items, over_tab: None });
    }

    /// Esc during a drag cancels it (before the panels see the key).
    pub(super) fn inner_drag_keys(&mut self, ctx: &egui::Context) {
        if self.inner_drag.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.inner_drag = None;
            self.cancel_drag_out();
            tracing::info!("drag inside the app canceled");
        }
    }

    /// The drop target, its folder and what a drop would do.
    fn inner_outcome(&self, d: &InnerDrag, at: PaneRef, spot: &Spot, shift: bool) -> Option<(String, Outcome)> {
        let (src, dst) = (self.pane(d.from)?, self.pane(at)?);
        let dest = spot.dest(dst.fs.as_ref(), &dst.path);
        let src_ro = src.archive.is_some() || src.fs.read_only();
        let o = outcome(&disk_key(src), src_ro, &d.items, &disk_key(dst), &dest, self.drop_refusal(at), shift);
        Some((dest, o))
    }

    /// Every frame of a drag: highlight the target, switch tabs held over,
    /// and drop when the button is released.
    pub(super) fn inner_drag_ui(&mut self, ctx: &egui::Context) {
        if self.inner_drag.is_none() {
            return;
        }
        let (pos, down, shift) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.modifiers.shift));
        // Out of the window (X11): another application may take the files.
        if pos.is_none_or(|p| !ctx.content_rect().contains(p)) {
            self.start_drag_out(ctx);
        }
        if let Some(st) = self.drag_out.as_ref().filter(|o| !o.dropping).map(|o| o.status()) {
            if st.outside {
                if !down {
                    // The source thread hands the drop over to the target.
                    self.inner_drag = None;
                    if let Some(o) = &mut self.drag_out {
                        o.dropping = true;
                    }
                    return;
                }
                ctx.set_cursor_icon(if st.accepted { egui::CursorIcon::Grabbing } else { egui::CursorIcon::NoDrop });
                ctx.request_repaint();
                return;
            }
        }
        let target = pos.and_then(|p| target_at(&self.drop.zones, &self.drop.tabs, p));
        // Holding over a tab shows it (then its folder rows are targets too).
        let mut switch = None;
        if let Some(d) = &mut self.inner_drag {
            match &target {
                Some(DropAt::Tab(at)) => match d.over_tab {
                    Some((id, since)) if id == at.tab => {
                        if since.elapsed() >= TAB_HOLD {
                            switch = Some(*at);
                        }
                    }
                    _ => d.over_tab = Some((at.tab, Instant::now())),
                },
                _ => d.over_tab = None,
            }
        }
        if let Some(at) = switch {
            if let Some(i) = self.sides[at.side].index(at.tab) {
                if self.sides[at.side].active != i {
                    self.activate_tab(at.side, i);
                }
            }
        }
        if !down {
            let d = self.inner_drag.take().expect("checked above");
            self.drag_out_back_inside();
            if let Some(t) = target {
                self.inner_drop(d, t, shift);
            }
            return;
        }
        let Some(d) = &self.inner_drag else { return };
        let n = d.items.len() as u64;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("inner-drag")));
        let mut refused = false;
        if let Some(t) = &target {
            let (at, spot) = t.clone().place();
            if let Some((_, o)) = self.inner_outcome(d, at, &spot, shift) {
                let text = match o {
                    Outcome::Same => tr!("The items are already here"),
                    Outcome::IntoItself => tr!("A folder cannot go into itself"),
                    Outcome::Refused(r) => r.text(),
                    Outcome::Copy | Outcome::CopyOnly => trn!("Copy here: {n} item", "Copy here: {n} items", n),
                    Outcome::Move => trn!("Move here: {n} item", "Move here: {n} items", n),
                };
                refused = !matches!(o, Outcome::Copy | Outcome::CopyOnly | Outcome::Move);
                match t {
                    DropAt::Tab(_) => {
                        if let Some((_, r)) = self.drop.tabs.iter().find(|(a, _)| *a == at) {
                            let color = if refused { Color32::from_gray(120) } else { Color32::from_rgb(30, 130, 60) };
                            painter.rect_stroke(r.expand(2.0), 4.0, Stroke::new(3.0, color), StrokeKind::Outside);
                            let galley = painter.layout_no_wrap(text, crate::fonts::prop(15.0), Color32::WHITE);
                            let bg = Rect::from_min_size(r.left_bottom() + Vec2::new(0.0, 6.0), galley.size() + Vec2::new(16.0, 8.0));
                            painter.rect_filled(bg, 5.0, color);
                            painter.galley(bg.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
                        }
                    }
                    DropAt::Pane(_, spot) => {
                        if let Some(z) = self.drop.zones.iter().find(|z| z.at == at) {
                            let row = match spot {
                                Spot::Folder(name) => z.dirs.iter().find(|(_, d)| d == name).map(|(r, _)| *r),
                                Spot::Here => None,
                            };
                            paint_target(&painter, z.rect, row, refused, &text);
                        }
                    }
                }
            }
        }
        // "N items" at the cursor.
        if let Some(p) = pos {
            let label = trn!("{n} item", "{n} items", n);
            let galley = painter.layout_no_wrap(label, crate::fonts::prop(14.0), Color32::WHITE);
            let bg = Rect::from_min_size(p + Vec2::new(16.0, 14.0), galley.size() + Vec2::new(14.0, 8.0));
            painter.rect_filled(bg, 5.0, Color32::from_rgb(40, 90, 170));
            painter.galley(bg.min + Vec2::new(7.0, 4.0), galley, Color32::WHITE);
        }
        ctx.set_cursor_icon(if refused || target.is_none() { egui::CursorIcon::NoDrop } else { egui::CursorIcon::Grabbing });
        ctx.request_repaint();
    }

    /// The button was released over `target`.
    fn inner_drop(&mut self, d: InnerDrag, target: DropAt, shift: bool) {
        let (at, spot) = target.place();
        let Some((dest, o)) = self.inner_outcome(&d, at, &spot, shift) else { return };
        tracing::info!("drop inside the app: {o:?}");
        match o {
            Outcome::Same => {}
            Outcome::IntoItself => self.last_error = Some(tr!("A folder cannot be copied or moved into itself.")),
            Outcome::Refused(Refusal::ReadOnly) => {
                self.last_error = Some(tr!("Cannot drop into an archive: it is open read-only."));
            }
            Outcome::Refused(r) => self.last_error = Some(r.text()),
            Outcome::Copy => self.open_copy(d.from, at, dest, d.items),
            Outcome::Move | Outcome::CopyOnly => self.open_move(d.from, at, dest, d.items),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn t(dir: &str, name: &str) -> Target {
        Target { dir: dir.into(), name: name.into(), label: name.into(), size: 1 }
    }

    #[test]
    fn target_under_the_cursor() {
        let left = PaneRef { tab: 1, side: 0 };
        let right = PaneRef { tab: 2, side: 1 };
        let other_tab = PaneRef { tab: 3, side: 1 };
        let zones = vec![
            PaneZone { at: left, rect: Rect::from_min_max(pos2(0.0, 100.0), pos2(500.0, 700.0)), dirs: vec![] },
            PaneZone {
                at: right,
                rect: Rect::from_min_max(pos2(510.0, 100.0), pos2(1000.0, 700.0)),
                dirs: vec![
                    (Rect::from_min_max(pos2(510.0, 120.0), pos2(1000.0, 140.0)), "..".into()),
                    (Rect::from_min_max(pos2(510.0, 140.0), pos2(1000.0, 160.0)), "docs".into()),
                ],
            },
        ];
        let tabs = vec![(other_tab, Rect::from_min_max(pos2(600.0, 70.0), pos2(700.0, 95.0)))];
        assert_eq!(target_at(&zones, &tabs, pos2(100.0, 300.0)), Some(DropAt::Pane(left, Spot::Here)));
        assert_eq!(target_at(&zones, &tabs, pos2(700.0, 150.0)), Some(DropAt::Pane(right, Spot::Folder("docs".into()))));
        assert_eq!(target_at(&zones, &tabs, pos2(700.0, 130.0)), Some(DropAt::Pane(right, Spot::Folder("..".into()))));
        assert_eq!(target_at(&zones, &tabs, pos2(700.0, 400.0)), Some(DropAt::Pane(right, Spot::Here)));
        assert_eq!(target_at(&zones, &tabs, pos2(650.0, 80.0)), Some(DropAt::Tab(other_tab)));
        assert_eq!(target_at(&zones, &tabs, pos2(650.0, 20.0)), None, "the menu bar is no target");
    }

    #[test]
    fn spot_gives_the_folder() {
        let fs = rolid_local::LocalFs;
        assert_eq!(Spot::Here.dest(&fs, "/a/b"), "/a/b");
        assert_eq!(Spot::Folder("c".into()).dest(&fs, "/a/b"), "/a/b/c");
        assert_eq!(Spot::Folder("..".into()).dest(&fs, "/a/b"), "/a");
    }

    #[test]
    fn what_a_drop_does() {
        let items = [t("/home/u", "a.txt"), t("/home/u", "dir")];
        // The other panel, another folder: copy, with Shift move.
        assert_eq!(outcome("local", false, &items, "local", "/tmp", None, false), Outcome::Copy);
        assert_eq!(outcome("local", false, &items, "local", "/tmp", None, true), Outcome::Move);
        // To a server: also copy / move.
        assert_eq!(outcome("local", false, &items, "sftp:u@h:22", "/home/u", None, true), Outcome::Move);
        // Back onto the source folder (also with a trailing slash): nothing.
        assert_eq!(outcome("local", false, &items, "local", "/home/u/", None, false), Outcome::Same);
        // Onto its own row or into a folder inside it: refused.
        assert_eq!(outcome("local", false, &items, "local", "/home/u/dir", None, false), Outcome::IntoItself);
        assert_eq!(outcome("local", false, &items, "local", "/home/u/dir/x", None, true), Outcome::IntoItself);
        // A sibling whose name starts the same is fine.
        assert_eq!(outcome("local", false, &items, "local", "/home/u/dir2", None, false), Outcome::Copy);
        // Out of an archive: only copy, also with Shift.
        assert_eq!(outcome("archive:0x1", true, &items, "local", "/home/u", None, false), Outcome::Copy);
        assert_eq!(outcome("archive:0x1", true, &items, "local", "/home/u", None, true), Outcome::CopyOnly);
        // Into an archive or search results: refused.
        assert_eq!(
            outcome("local", false, &items, "archive:0x1", "/", Some(Refusal::ReadOnly), false),
            Outcome::Refused(Refusal::ReadOnly)
        );
        assert_eq!(outcome("local", false, &items, "local", "/x", Some(Refusal::Search), true), Outcome::Refused(Refusal::Search));
    }
}
