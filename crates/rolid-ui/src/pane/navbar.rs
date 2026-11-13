//! The bar above a panel (as in WinSCP): a drop-down with the tree of the
//! path (root, every parent folder, the current one; disks of this computer
//! under it) and the buttons Back, Forward, Up, Root, Home, Refresh and
//! Favorites. The path row under it keeps the path and shows the free space,
//! the file system and the disk health badge on the right.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Vec2};
use rolid_disk::mounts::Mount;

use super::{Pane, PaneEvent};
use crate::icons;

/// Height of the bar.
pub const BAR_H: f32 = 28.0;

/// What a row of the tree opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Go {
    Path(String),
    /// The folder the archive is in (the archive is left).
    LeaveArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeIcon {
    Root,
    Folder,
    Archive,
}

/// One level of the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub go: Go,
    pub depth: usize,
    pub current: bool,
    pub icon: NodeIcon,
}

/// The tree of `path`: the root, every parent folder and the folder itself.
/// Inside an archive (`archive` = its name and the folder it is in) the
/// folder of the archive comes first, then the archive and the path in it.
pub fn path_tree(path: &str, archive: Option<(&str, &str)>) -> Vec<Node> {
    let mut out = Vec::new();
    match archive {
        None => {
            out.push(Node { label: "/".into(), go: Go::Path("/".into()), depth: 0, current: false, icon: NodeIcon::Root });
            let mut acc = String::new();
            for (i, part) in path.split('/').filter(|s| !s.is_empty()).enumerate() {
                acc.push('/');
                acc.push_str(part);
                out.push(Node { label: part.into(), go: Go::Path(acc.clone()), depth: i + 1, current: false, icon: NodeIcon::Folder });
            }
        }
        Some((name, outer)) => {
            out.push(Node { label: folder_name(outer), go: Go::LeaveArchive, depth: 0, current: false, icon: NodeIcon::Folder });
            out.push(Node { label: name.into(), go: Go::Path(name.into()), depth: 1, current: false, icon: NodeIcon::Archive });
            let inner = path.strip_prefix(name).unwrap_or("");
            let mut acc = name.to_string();
            for (i, part) in inner.split('/').filter(|s| !s.is_empty()).enumerate() {
                acc.push('/');
                acc.push_str(part);
                out.push(Node { label: part.into(), go: Go::Path(acc.clone()), depth: i + 2, current: false, icon: NodeIcon::Folder });
            }
        }
    }
    if let Some(last) = out.last_mut() {
        last.current = true;
    }
    out
}

/// The name shown in the drop-down: the current folder ("/" for the root).
pub fn folder_name(path: &str) -> String {
    path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/").to_string()
}

/// A disk or mount point offered under the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountRow {
    pub point: String,
    pub fstype: String,
    /// Total size, when known.
    pub total: Option<u64>,
}

/// Mount points worth offering: disks (`/dev/…`) and network file systems,
/// not the system's own (`/boot`, `/proc`, snaps…). A later mount on the
/// same point hides an earlier one.
pub fn user_mounts(mounts: &[Mount]) -> Vec<Mount> {
    let hidden = |p: &str| {
        ["/boot", "/proc", "/sys", "/dev", "/snap", "/var/snap", "/var/lib", "/run"]
            .iter()
            .any(|h| p == *h || p.starts_with(&format!("{h}/")))
            && !p.starts_with("/run/media/")
    };
    let mut out: Vec<Mount> = Vec::new();
    for m in mounts {
        let disk = m.source.starts_with("/dev/") && !matches!(m.fstype.as_str(), "squashfs" | "iso9660" | "swap");
        if !(disk || rolid_disk::mounts::is_network(&m.fstype)) || hidden(&m.point) {
            continue;
        }
        out.retain(|o| o.point != m.point);
        out.push(m.clone());
    }
    out.sort_by(|a, b| a.point.cmp(&b.point));
    out
}

/// The free-space text that fits `width`: first "Free:" goes, then the
/// total size, then the file system. `measure` gives a text's width.
pub fn space_label(free: &str, total: &str, fstype: Option<&str>, width: f32, measure: impl Fn(&str) -> f32) -> String {
    let fs = |s: String| match fstype {
        Some(t) => format!("{s} · {t}"), // i18n-ok: a separator and a file system name
        None => s,
    };
    let tries = [
        fs(tr!("Free: {free} of {total}", free = free, total = total)),
        fs(tr!("{free} of {total}", free = free, total = total)),
        fs(free.to_string()),
        free.to_string(),
    ];
    for t in &tries[..3] {
        if measure(t) <= width {
            return t.clone();
        }
    }
    tries[3].clone()
}

/// An open drop-down tree.
#[derive(Debug, Clone, Default)]
pub struct TreeOpen {
    /// The row chosen with the keyboard.
    pub sel: usize,
    /// Opened with the keyboard: the selection starts at the current folder.
    pub started: bool,
}

/// The buttons of the bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    Back,
    Forward,
    Up,
    Root,
    Home,
    Refresh,
    Favorites,
}

/// Paint a bar icon; Root, Home and Up are composed like in WinSCP.
fn paint_icon(p: &egui::Painter, c: Pos2, b: Btn, color: Color32) {
    let glyph = |g: &str, size: f32, at: Pos2| {
        p.text(at, Align2::CENTER_CENTER, g, icons::font(size), color);
    };
    match b {
        Btn::Back => glyph(icons::BACK, 17.0, c),
        Btn::Forward => glyph(icons::FORWARD, 17.0, c),
        Btn::Refresh => glyph(icons::REFRESH, 17.0, c),
        Btn::Favorites => glyph(icons::STAR, 17.0, c),
        Btn::Up => {
            glyph(icons::FOLDER_SIMPLE, 19.0, c);
            glyph(icons::ARROW_UP, 9.5, c + Vec2::new(0.0, 1.5));
        }
        Btn::Root => {
            glyph(icons::FOLDER_SIMPLE, 19.0, c);
            p.text(c + Vec2::new(0.0, 1.5), Align2::CENTER_CENTER, "/", egui::FontId::monospace(10.5), color);
        }
        Btn::Home => {
            glyph(icons::HOUSE_SIMPLE, 19.0, c);
            p.text(c + Vec2::new(0.0, 2.5), Align2::CENTER_CENTER, "~", egui::FontId::monospace(9.5), color);
        }
    }
}

fn node_glyph(i: NodeIcon) -> &'static str {
    match i {
        NodeIcon::Root | NodeIcon::Folder => icons::FOLDER_SIMPLE,
        NodeIcon::Archive => icons::ARCHIVE,
    }
}

impl Pane {
    /// The tree of this panel's path.
    pub fn tree(&self) -> Vec<Node> {
        match &self.archive {
            Some(st) => path_tree(&self.path, Some((&st.fs.name, &st.back_path))),
            None => path_tree(&self.path, None),
        }
    }

    /// The root of this panel: "/" or the top of the archive.
    pub fn root_path(&self) -> String {
        match &self.archive {
            Some(st) => st.fs.name.clone(),
            None => "/".into(),
        }
    }

    /// Disks are offered for a local folder (not in an archive).
    pub fn offers_disks(&self) -> bool {
        self.archive.is_none() && !self.fs.is_remote()
    }

    /// Open the drop-down (Alt+↓ or a click); asks for the disk list.
    pub fn open_tree(&mut self, events: &mut Vec<PaneEvent>) {
        if self.search.is_some() {
            return;
        }
        let sel = self.tree().len().saturating_sub(1);
        self.tree_open = Some(TreeOpen { sel, started: false });
        if self.offers_disks() {
            events.push(PaneEvent::WantMounts);
        }
    }

    /// The bar above the panel.
    pub(super) fn nav_bar(&mut self, ui: &mut egui::Ui, width: f32, favorites: &[(String, rolid_config::Location)], events: &mut Vec<PaneEvent>) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR_H), Sense::hover());
        let vis = ui.visuals().clone();
        let fg = vis.text_color();
        let weak = vis.weak_text_color();
        let buttons = [Btn::Back, Btn::Forward, Btn::Up, Btn::Root, Btn::Home, Btn::Refresh, Btn::Favorites];
        // A narrow panel: the drop-down shrinks first, then the buttons.
        let bw = ((width - 70.0) / buttons.len() as f32).clamp(22.0, 28.0);
        let combo_w = (width - buttons.len() as f32 * bw - 8.0).clamp(60.0, 280.0);
        let combo = Rect::from_min_size(rect.min + Vec2::new(0.0, 2.0), Vec2::new(combo_w, BAR_H - 4.0));
        // --- The drop-down ---
        let searching = self.search.is_some();
        let cresp = ui.interact(combo, ui.scope_id().with("nav-combo"), if searching { Sense::hover() } else { Sense::click() });
        let p = ui.painter_at(rect);
        let fill = if cresp.hovered() && !searching { vis.widgets.hovered.weak_bg_fill } else { vis.widgets.inactive.weak_bg_fill };
        p.rect_filled(combo, 3.0, fill);
        p.rect_stroke(combo, 3.0, vis.widgets.inactive.bg_stroke, egui::StrokeKind::Inside);
        let color = if searching { weak } else { fg };
        let (glyph, name) = match &self.archive {
            Some(st) if self.fs.parent(&self.path).is_none() => (icons::ARCHIVE, st.fs.name.clone()),
            _ => (icons::FOLDER_SIMPLE, folder_name(&self.path)),
        };
        p.text(combo.left_center() + Vec2::new(13.0, 0.0), Align2::CENTER_CENTER, glyph, icons::font(16.0), color);
        let caret = combo.right_center() - Vec2::new(11.0, 0.0);
        p.text(caret, Align2::CENTER_CENTER, icons::CARET_DOWN, icons::font(12.0), color);
        let text_clip = Rect::from_min_max(combo.min + Vec2::new(24.0, 0.0), egui::pos2(caret.x - 9.0, combo.max.y));
        ui.painter_at(text_clip).text(text_clip.left_center(), Align2::LEFT_CENTER, &name, crate::fonts::prop(13.5), color);
        let tip = if searching { tr!("Not available in search results") } else { tr!("Folders on the path and disks (Alt+↓)") };
        let cresp = cresp.on_hover_text(tip);
        if cresp.clicked() {
            events.push(PaneEvent::Focus);
            if self.tree_open.is_some() {
                self.tree_open = None;
            } else {
                self.open_tree(events);
            }
        }
        // --- The buttons ---
        let mut x = combo.right() + 6.0;
        for b in buttons {
            let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), Vec2::new(bw - 2.0, BAR_H - 4.0));
            x += bw;
            let enabled = match b {
                Btn::Back => !searching && self.history.can_back(),
                Btn::Forward => !searching && self.history.can_forward(),
                Btn::Up => searching || self.fs.parent(&self.path).is_some() || self.archive.is_some(),
                _ => true,
            };
            let resp = ui.interact(r, ui.scope_id().with(("nav-btn", b as u8)), if enabled { Sense::click() } else { Sense::hover() });
            if resp.hovered() && enabled {
                p.rect_filled(r, 3.0, vis.widgets.hovered.weak_bg_fill);
            }
            paint_icon(&p, r.center(), b, if enabled { fg } else { weak.gamma_multiply(0.6) });
            let tip = match b {
                Btn::Back => tr!("Back (Alt+←)"),
                Btn::Forward => tr!("Forward (Alt+→)"),
                Btn::Up => tr!("Up one level (Backspace)"),
                Btn::Root => tr!("Root folder (Ctrl+\\)"),
                Btn::Home => tr!("Home folder (Ctrl+H)"),
                Btn::Refresh => tr!("Refresh (Ctrl+R)"),
                Btn::Favorites => tr!("Favorite and recent folders (Ctrl+D adds this folder)"),
            };
            let resp = resp.on_hover_text(tip);
            if b == Btn::Favorites {
                self.favorites_menu(&resp, favorites, events);
                continue;
            }
            if resp.clicked() {
                events.push(PaneEvent::Focus);
                events.push(match b {
                    Btn::Back => PaneEvent::History(false),
                    Btn::Forward => PaneEvent::History(true),
                    Btn::Up => PaneEvent::Up,
                    Btn::Root => PaneEvent::GoTo(self.root_path()),
                    Btn::Home => PaneEvent::GoTo("~".into()),
                    _ => PaneEvent::Refresh,
                });
            }
        }
        if self.tree_open.is_some() {
            self.tree_popup(ui, combo, events);
        }
    }

    /// The open drop-down: the tree of the path, then the disks.
    pub(super) fn tree_popup(&mut self, ui: &mut egui::Ui, combo: Rect, events: &mut Vec<PaneEvent>) {
        let nodes = self.tree();
        let disks: Vec<MountRow> = if self.offers_disks() { self.mounts.clone().unwrap_or_default() } else { Vec::new() };
        let n = nodes.len() + disks.len();
        let Some(open) = self.tree_open.as_mut() else { return };
        open.sel = open.sel.min(n.saturating_sub(1));
        // The keyboard belongs to the drop-down while it is open.
        let (up, down, enter, esc) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) || i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if up {
            open.sel = open.sel.saturating_sub(1);
        }
        if down && open.started {
            open.sel = (open.sel + 1).min(n.saturating_sub(1));
        }
        open.started = true;
        let mut chosen: Option<usize> = if enter { Some(open.sel) } else { None };
        let sel = open.sel;
        let row_h = 22.0;
        let width = combo.width().max(320.0);
        let id = ui.scope_id().with("nav-tree");
        let vis = ui.visuals().clone();
        let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(combo.left_bottom() + Vec2::new(0.0, 2.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut row = |ui: &mut egui::Ui, i: usize, depth: usize, glyph: &str, text: &str, right: Option<String>, current: bool| {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());
                    let p = ui.painter_at(r);
                    if i == sel || resp.hovered() {
                        p.rect_filled(r, 2.0, vis.selection.bg_fill.gamma_multiply(if i == sel { 1.0 } else { 0.5 }));
                    }
                    let color = if i == sel { vis.selection.stroke.color } else { vis.text_color() };
                    let x = r.left() + 8.0 + depth as f32 * 14.0;
                    p.text(egui::pos2(x + 8.0, r.center().y), Align2::CENTER_CENTER, glyph, icons::font(15.0), color);
                    let galley = p.layout_no_wrap(text.to_string(), crate::fonts::prop(13.5), color);
                    let tx = egui::pos2(x + 20.0, r.center().y - galley.size().y / 2.0);
                    if current {
                        // The current folder: bold-looking (drawn twice).
                        p.galley(tx, galley.clone(), color);
                        p.galley(tx + Vec2::new(0.6, 0.0), galley.clone(), color);
                    } else {
                        p.galley(tx, galley.clone(), color);
                    }
                    if let Some(rt) = right {
                        p.text(r.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, rt, crate::fonts::prop(12.0), if i == sel { color } else { vis.weak_text_color() });
                    }
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                };
                for (i, nd) in nodes.iter().enumerate() {
                    row(ui, i, nd.depth, node_glyph(nd.icon), &nd.label, None, nd.current);
                }
                if self.offers_disks() {
                    ui.add_space(3.0);
                    ui.separator();
                    ui.weak(tr!("Disks"));
                    if self.mounts.is_none() {
                        ui.weak("…");
                    }
                    for (k, m) in disks.iter().enumerate() {
                        let size = m.total.map(super::human_size);
                        let right = match size {
                            Some(s) => format!("{s} · {}", m.fstype), // i18n-ok: size and file system
                            None => m.fstype.clone(),
                        };
                        row(ui, nodes.len() + k, 0, icons::HARD_DRIVES, &m.point, Some(right), false);
                    }
                }
            });
        });
        let area_rect = area.response.rect;
        let clicked_outside = ui.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area_rect.contains(p) && !combo.contains(p))
        });
        if let Some(i) = chosen {
            self.tree_open = None;
            events.push(PaneEvent::Focus);
            if let Some(nd) = nodes.get(i) {
                if !nd.current {
                    // A parent folder: the cursor on the folder we came from.
                    let from = nodes.get(i + 1).map(|n| n.label.clone());
                    events.push(match &nd.go {
                        Go::Path(p) => PaneEvent::Jump { path: p.clone(), select: from },
                        // Out of the archive even from a folder deep in it.
                        Go::LeaveArchive => PaneEvent::LeaveArchive,
                    });
                }
            } else if let Some(m) = disks.get(i - nodes.len()) {
                events.push(PaneEvent::GoTo(m.point.clone()));
            }
        } else if esc || clicked_outside {
            self.tree_open = None;
        }
        ui.ctx().request_repaint();
    }

    /// The favorite and recent folders (the ★ button, or "*" in the path row).
    pub(super) fn favorites_menu(&self, resp: &egui::Response, favorites: &[(String, rolid_config::Location)], events: &mut Vec<PaneEvent>) {
        let recent = self.history.recent(&self.path, 15);
        let here = self.location();
        let favorite = favorites.iter().any(|(_, l)| l.same_place(&here));
        let mut picked = None;
        egui::Popup::menu(resp).show(|ui| {
            ui.set_min_width(260.0);
            ui.weak(tr!("Favorite folders"));
            if favorites.is_empty() {
                ui.weak(tr!("None yet: Ctrl+D adds the current folder."));
            }
            ui.set_max_width(520.0);
            for (i, (label, _)) in favorites.iter().enumerate() {
                let b = egui::Button::new(format!("★ {label}")).wrap_mode(egui::TextWrapMode::Truncate);
                if ui.add(b).on_hover_text(label).clicked() {
                    events.push(PaneEvent::Focus);
                    events.push(PaneEvent::Favorite(i));
                    ui.close();
                }
            }
            ui.separator();
            ui.weak(tr!("Recent folders of this tab"));
            if recent.is_empty() {
                ui.weak(tr!("No other folders yet."));
            }
            for p in &recent {
                if ui.add(egui::Button::new(p).wrap_mode(egui::TextWrapMode::Truncate)).on_hover_text(p).clicked() {
                    picked = Some(p.clone());
                    ui.close();
                }
            }
            ui.separator();
            let toggle = if favorite { tr!("Remove from favorites") } else { tr!("Add to favorites") };
            if ui.add(egui::Button::new(toggle).shortcut_text("Ctrl+D")).clicked() {
                events.push(PaneEvent::ToggleFavorite);
                ui.close();
            }
            if ui.button(tr!("Organize favorites…")).clicked() {
                events.push(PaneEvent::OrganizeFavorites);
                ui.close();
            }
        });
        if let Some(p) = picked {
            events.push(PaneEvent::Focus);
            events.push(PaneEvent::GoTo(p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(t: &[Node]) -> Vec<(&str, usize)> {
        t.iter().map(|n| (n.label.as_str(), n.depth)).collect()
    }

    #[test]
    fn tree_of_a_local_or_remote_path() {
        let t = path_tree("/home/user/Documents and photos", None);
        assert_eq!(labels(&t), vec![("/", 0), ("home", 1), ("user", 2), ("Documents and photos", 3)]);
        assert_eq!(t[0].go, Go::Path("/".into()));
        assert_eq!(t[2].go, Go::Path("/home/user".into()));
        assert_eq!(t[3].go, Go::Path("/home/user/Documents and photos".into()));
        assert!(t[3].current && !t[2].current);
        assert_eq!(t[0].icon, NodeIcon::Root);
        // A server path looks the same (with a trailing slash too).
        let t = path_tree("/var/www/", None);
        assert_eq!(labels(&t), vec![("/", 0), ("var", 1), ("www", 2)]);
        // The root itself: one row, the current one.
        let t = path_tree("/", None);
        assert_eq!(t.len(), 1);
        assert!(t[0].current);
        assert_eq!(folder_name("/"), "/");
        assert_eq!(folder_name("/home/m/"), "m");
    }

    #[test]
    fn tree_inside_an_archive() {
        let t = path_tree("data.zip/docs/2026", Some(("data.zip", "/home/m/Downloads")));
        assert_eq!(labels(&t), vec![("Downloads", 0), ("data.zip", 1), ("docs", 2), ("2026", 3)]);
        assert_eq!(t[0].go, Go::LeaveArchive);
        assert_eq!(t[1].go, Go::Path("data.zip".into()));
        assert_eq!(t[1].icon, NodeIcon::Archive);
        assert_eq!(t[3].go, Go::Path("data.zip/docs/2026".into()));
        assert!(t[3].current);
        let t = path_tree("data.zip", Some(("data.zip", "/tmp")));
        assert_eq!(labels(&t), vec![("tmp", 0), ("data.zip", 1)]);
        assert!(t[1].current);
    }

    #[test]
    fn disks_from_mountinfo() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw shared:12 - proc proc rw
24 22 259:1 / /boot/efi rw shared:2 - vfat /dev/nvme0n1p1 rw
25 22 0:5 / /dev rw shared:2 - devtmpfs udev rw
26 22 0:25 / /run rw shared:5 - tmpfs tmpfs rw
30 22 253:1 / /home rw shared:3 - ext4 /dev/mapper/vg-home rw
31 30 0:45 / /home/m/Ext\\040disk rw shared:40 - exfat /dev/sdb1 rw
32 22 0:50 /export /mnt/nas rw shared:41 - nfs4 nas.local:/export rw
33 26 8:33 / /run/media/m/USB rw shared:44 - vfat /dev/sdc1 rw
34 22 7:1 / /snap/core/1 ro shared:45 - squashfs /dev/loop1 ro
35 22 7:2 / /media/cd ro shared:46 - iso9660 /dev/sr0 ro
36 22 8:2 / /data rw shared:47 - btrfs /dev/sda2 rw
37 22 8:3 / /data rw shared:48 - xfs /dev/sdc3 rw
";
        let m = user_mounts(&rolid_disk::mounts::parse(text));
        let points: Vec<(&str, &str)> = m.iter().map(|m| (m.point.as_str(), m.fstype.as_str())).collect();
        assert_eq!(
            points,
            vec![
                ("/", "ext4"),
                ("/data", "xfs"),
                ("/home", "ext4"),
                ("/home/m/Ext disk", "exfat"),
                ("/mnt/nas", "nfs4"),
                ("/run/media/m/USB", "vfat"),
            ]
        );
    }

    #[test]
    fn free_space_text_shortens_by_width() {
        let w = |s: &str| s.chars().count() as f32 * 7.0;
        let l = |width: f32| space_label("10.0 GB", "15.3 GB", Some("ext4"), width, w);
        assert_eq!(l(1000.0), "Free: 10.0 GB of 15.3 GB · ext4");
        assert_eq!(l(30.0 * 7.0), "10.0 GB of 15.3 GB · ext4", "“Free:” goes first");
        assert_eq!(l(24.0 * 7.0), "10.0 GB · ext4", "then the total");
        assert_eq!(l(5.0 * 7.0), "10.0 GB", "then the file system; the free space stays");
        assert_eq!(space_label("1 GB", "2 GB", None, 1000.0, w), "Free: 1 GB of 2 GB");
        assert_eq!(space_label("1 GB", "2 GB", None, 50.0, w), "1 GB");
    }
}
