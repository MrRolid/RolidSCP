//! Disk health (SMART): reading in the background, the badge state of the
//! panels and the detail dialog for lay people.

use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText};
use rolid_disk::health::{DiskHealth, Reading};
use rolid_disk::physical::NoSmart;
use rolid_disk::smart::{Item, Kind, Level, Report, Source};
use rolid_i18n::{tr, trc};

use super::{App, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::{self, Role};
use crate::pane::human_size;

/// What the panels know about the disks of this computer.
#[derive(Default)]
pub struct Disks {
    /// Disk name → its reading (`None` while being read).
    pub map: HashMap<String, Option<DiskHealth>>,
    /// Temperatures are being read now.
    temps_running: bool,
}

pub struct DiskDialog {
    at: PaneRef,
    /// Opened as "Disk properties" (the panel's menu), not from the badge.
    props: bool,
}

/// Colors of the badge pill: (fill, icon and text).
pub fn badge_colors(level: Option<Level>) -> (Color32, Color32) {
    match level {
        None => (Color32::from_white_alpha(230), Color32::from_gray(135)),
        Some(Level::Ok) => (Color32::from_white_alpha(230), look::GREEN),
        // Dark text on orange, white on red: both above 4.5:1.
        Some(Level::Warn) => (Color32::from_rgb(0xf5, 0x9e, 0x0b), Color32::from_rgb(0x2b, 0x1a, 0x00)),
        Some(Level::Crit) => (Color32::from_rgb(0xc6, 0x28, 0x28), Color32::WHITE),
    }
}

/// The width of the badge pill (to place it before painting).
pub fn badge_width(p: &egui::Painter, b: &Badge, font: &egui::FontId) -> f32 {
    let icon = 16.0;
    match &b.short {
        Some(t) if b.level.is_some_and(|l| l > Level::Ok) => icon + 4.0 + p.layout_no_wrap(t.clone(), font.clone(), Color32::BLACK).size().x + 14.0,
        _ => icon + 12.0,
    }
}

/// Paint the badge pill into `rect` (as wide as `badge_width`).
pub fn paint_badge(p: &egui::Painter, rect: egui::Rect, b: &Badge, font: &egui::FontId) {
    let (fill, fg) = badge_colors(b.level);
    p.rect_filled(rect, rect.height() / 2.0, fill);
    let ir = p.text(egui::pos2(rect.left() + 6.0, rect.center().y), egui::Align2::LEFT_CENTER, crate::icons::HEARTBEAT, crate::icons::font(14.0), fg);
    if let (Some(t), true) = (&b.short, b.level.is_some_and(|l| l > Level::Ok)) {
        p.text(egui::pos2(ir.right() + 4.0, rect.center().y), egui::Align2::LEFT_CENTER, t, font.clone(), fg);
    }
}

/// The color of the free-space bar: orange under 10 % free, red under 5 %.
pub fn space_color(avail: u64, total: u64) -> Option<Color32> {
    if total == 0 {
        return None;
    }
    let free = avail as f64 / total as f64;
    if free < 0.05 {
        Some(Color32::from_rgb(0xd3, 0x2f, 0x2f))
    } else if free < 0.10 {
        Some(Color32::from_rgb(0xe8, 0x89, 0x0c))
    } else {
        None
    }
}

/// Color of a state (text, badge dot).
pub fn level_color(level: Level, dark: bool) -> Color32 {
    match (level, dark) {
        (Level::Ok, false) => look::GREEN,
        (Level::Ok, true) => Color32::from_rgb(0x6c, 0xd6, 0x86),
        (Level::Warn, false) => look::ORANGE,
        (Level::Warn, true) => Color32::from_rgb(0xff, 0xa8, 0x4d),
        (Level::Crit, false) => look::RED,
        (Level::Crit, true) => Color32::from_rgb(0xff, 0x7a, 0x7a),
    }
}

/// The badge text.
pub fn badge_text(level: Level) -> String {
    match level {
        Level::Ok => tr!("Disk OK"),
        Level::Warn => tr!("Disk: warning"),
        Level::Crit => tr!("Disk: critical"),
    }
}

/// The disk badge of a panel (path row, Disk properties).
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    /// `None`: not known (no SMART, not read yet, a server).
    pub level: Option<Level>,
    /// The most serious reason, short ("62 °C", "Wear 71 %"), when not OK.
    pub short: Option<String>,
    /// The full state and reasons, for the tooltip.
    pub tip: String,
}

/// A short reason for the badge.
pub fn short_reason(item: Item, rep: &Report) -> String {
    match item {
        Item::Temperature => match rep.temp_c {
            Some(t) => format!("{} °C", rolid_i18n::decimal(t, 0)), // i18n-ok: a number and a unit
            None => tr!("Temperature"),
        },
        Item::Wear => match rep.wear_used {
            Some(w) => tr!("Wear {used} %", used = rolid_i18n::decimal(w, 0)),
            None => tr!("Wear"),
        },
        Item::Reallocated | Item::Pending | Item::Uncorrectable => tr!("Bad sectors"),
        Item::MediaErrors => tr!("Data errors"),
        Item::Overall => tr!("Disk failing"),
        Item::SelfTest => tr!("Self-test failed"),
        Item::NvmeWarning => tr!("Critical warning"),
        Item::Spare => tr!("Low spare space"),
        Item::Crc => tr!("Transfer errors"),
        _ => tr!("Problem"),
    }
}

/// The badge of a panel from its disks and their readings.
pub fn badge_of(disks: &Option<Result<Vec<String>, NoSmart>>, map: &HashMap<String, Option<DiskHealth>>) -> Option<Badge> {
    let names = match disks.as_ref()? {
        Err(n) => return Some(Badge { level: None, short: None, tip: format!("{}\n{}", tr!("Disk health unknown"), no_smart_text(*n)) }),
        Ok(v) if v.is_empty() => return Some(Badge { level: None, short: None, tip: format!("{}\n{}", tr!("Disk health unknown"), no_smart_text(NoSmart::NoDisk)) }),
        Ok(v) => v,
    };
    let mut lines = Vec::new();
    let mut worst: Option<(Level, &Reading)> = None;
    for name in names {
        let dev = format!("/dev/{name}"); // i18n-ok: a device path
        match map.get(name) {
            None | Some(None) => {
                lines.push(tr!("Reading the state of disk {name}…", name = name.as_str()));
            }
            Some(Some(h)) => match &h.result {
                Err(n) => {
                        lines.push(format!("{dev}: {}", no_smart_text(*n)));
                }
                Ok(r) => {
                    let model = if r.report.model.is_empty() { dev.clone() } else { format!("{dev} ({})", r.report.model) };
                    lines.push(format!("{model}: {}", verdict(r)));
                    for (item, l) in &r.eval.items {
                        if *l > Level::Ok {
                            let what = if *l == Level::Crit { tr!("critical") } else { tr!("warning") };
                            lines.push(format!("  • {} ({what})", short_reason(*item, &r.report)));
                        }
                    }
                    if worst.is_none_or(|(w, _)| r.eval.level > w) {
                        worst = Some((r.eval.level, r));
                    }
                }
            },
        }
    }
    let level = worst.map(|(l, _)| l);
    let short = worst.and_then(|(_, r)| r.eval.reason.map(|i| short_reason(i, &r.report)));
    let head = match level {
        Some(l) => badge_text(l),
        None => tr!("Disk health unknown"),
    };
    Some(Badge { level, short, tip: format!("{head}\n{}", lines.join("\n")) })
}

/// The verdict sentence at the top of the detail.
fn verdict(r: &Reading) -> String {
    let e = &r.eval;
    match e.level {
        Level::Ok => tr!("The disk is healthy. Nothing needs to be done."),
        Level::Warn => match e.reason {
            Some(Item::Wear) => tr!("The disk is wearing out. Back up regularly and plan to replace it."),
            Some(Item::Reallocated | Item::Pending | Item::Uncorrectable) => {
                tr!("The disk has some bad sectors. Back up important data and watch whether their number grows.")
            }
            Some(Item::Temperature) => tr!("The disk is running hot. Check the cooling and the airflow around it."),
            Some(Item::MediaErrors) => tr!("The disk has reported data errors. Back up important data."),
            _ => tr!("The disk reports a problem. Back up important data."),
        },
        Level::Crit => {
            if e.growing {
                tr!("The number of bad sectors is growing. Back up your data now and replace the disk.")
            } else if e.reason == Some(Item::Temperature) {
                tr!("The disk is overheating. Stop heavy work on it and check the cooling now; heat can damage it and the data.")
            } else if e.reason == Some(Item::Wear) {
                tr!("The disk is almost worn out. Back up your data now and replace the disk.")
            } else {
                tr!("The disk may fail soon. Back up your data now and replace the disk.")
            }
        }
    }
}

fn no_smart_text(n: NoSmart) -> String {
    match n {
        NoSmart::Server => tr!("The health of the server's disks is not read: only the disks of this computer are checked."),
        NoSmart::Network => {
            tr!("This folder is on a network or remote file system, so the health of its disk cannot be read from here.")
        }
        NoSmart::Virtual => {
            tr!("This is a virtual disk (a virtual machine, a disk image or a RAM disk). It has no SMART data.")
        }
        NoSmart::NoDisk => tr!("This folder is not on a physical disk (for example a file system in memory)."),
        NoSmart::NotOffered => tr!(
            "The disk did not give any SMART data. USB adapters and card readers often do not pass it on, and smartctl may need administrator rights."
        ),
    }
}

impl App {
    /// The panel's disks are known: read those not read yet (in the background).
    pub(super) fn request_health(&mut self, names: &[String], force: bool) {
        for n in names {
            if !force && self.disks.map.contains_key(n) {
                continue;
            }
            self.disks.map.insert(n.clone(), None);
            let name = n.clone();
            let path = rolid_disk::cache::default_path();
            self.spawn(async move {
                let h = tokio::task::spawn_blocking({
                    let name = name.clone();
                    move || rolid_disk::health::read_disk(&name, &path, force)
                })
                .await
                .unwrap_or(DiskHealth { name, result: Err(NoSmart::NotOffered) });
                Msg::DiskHealth(Box::new(h))
            });
        }
        self.update_pane_health();
    }

    /// Every few minutes: read the temperature of the disks shown in the panels again.
    pub(super) fn disk_temps_frame(&mut self, ctx: &egui::Context) {
        let now = rolid_disk::cache::now();
        let mut due: Vec<(String, Kind)> = Vec::new();
        let mut next: Option<u64> = None;
        for h in self.disks.map.values().flatten() {
            if let Ok(r) = &h.result {
                if r.temp_due(now) {
                    due.push((h.name.clone(), r.report.kind));
                } else {
                    let left = (r.temp_at + rolid_disk::smart::TEMP_RECHECK).saturating_sub(now);
                    next = Some(next.map_or(left, |n| n.min(left)));
                }
            }
        }
        if let Some(left) = next {
            ctx.request_repaint_after(std::time::Duration::from_secs(left.max(1)));
        }
        if due.is_empty() || self.disks.temps_running {
            return;
        }
        self.disks.temps_running = true;
        self.spawn(async move {
            let temps = tokio::task::spawn_blocking(move || due.into_iter().map(|(n, k)| {
                let t = rolid_disk::health::read_temperature(&n, k);
                (n, t)
            }).collect())
            .await
            .unwrap_or_default();
            Msg::DiskTemps(temps, rolid_disk::cache::now())
        });
    }

    pub(super) fn on_disk_temps(&mut self, temps: Vec<(String, Option<f64>)>, at: u64) {
        self.disks.temps_running = false;
        for (name, t) in temps {
            if let Some(Some(DiskHealth { result: Ok(r), .. })) = self.disks.map.get_mut(&name) {
                let before = r.eval.level;
                r.set_temperature(t, at);
                if r.eval.level != before {
                    tracing::info!("disk {name}: temperature {t:?} °C, state {:?} → {:?}", before, r.eval.level);
                }
            }
        }
        self.update_pane_health();
    }

    pub(super) fn on_disk_health(&mut self, h: DiskHealth) {
        self.disks.map.insert(h.name.clone(), Some(h));
        self.update_pane_health();
    }

    /// The worst state of each panel's disks (for the badge and the row color).
    pub(super) fn update_pane_health(&mut self) {
        let map = &self.disks.map;
        for side in self.sides.iter_mut() {
            for t in side.tabs.iter_mut() {
                let p = &mut t.pane;
                p.health = badge_of(&p.disks, map);
            }
        }
    }

    pub(super) fn open_disk_detail(&mut self, at: PaneRef) {
        self.dialog = Dialog::Disk(Box::new(DiskDialog { at, props: false }));
    }

    /// "Disk properties…" in the menu of the panel's empty space.
    pub(super) fn open_disk_props(&mut self, at: PaneRef) {
        self.dialog = Dialog::Disk(Box::new(DiskDialog { at, props: true }));
    }

    pub(super) fn disk_dialog_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Disk(d) = &self.dialog else { return };
        let at = d.at;
        let title = if d.props { tr!("Disk properties") } else { tr!("Disk health (SMART)") };
        let Some(p) = self.pane(at) else {
            self.dialog = Dialog::None;
            return;
        };
        let disks: Result<Vec<String>, NoSmart> = p.disks.clone().unwrap_or(Err(NoSmart::NoDisk));
        let mut reload = false;
        let mut close = false;
        let dark = ctx.global_style().visuals.dark_mode;
        let space = p.space.filter(|s| s.total > 0);
        let fstype = p.fstype.clone();
        let folder = p.path.clone();
        let badge = p.health.clone();
        dialog::window("disk-health", title).icon(Role::Info).min_width(520.0).max_width(680.0).show(ctx, |ui| {
            space_section(ui, &folder, space, fstype.as_deref(), badge.as_ref());
            match &disks {
                Err(n) => dialog::lead(ui, no_smart_text(*n)),
                Ok(names) => {
                    egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.7).show(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            if i > 0 {
                                ui.separator();
                            }
                            match self.disks.map.get(name) {
                                None | Some(None) => {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(tr!("Reading the state of disk {name}…", name = name.as_str()));
                                    });
                                }
                                Some(Some(h)) => match &h.result {
                                    Err(n) => {
                                        ui.label(RichText::new(format!("/dev/{name}")).strong()); // i18n-ok: a device path
                                        dialog::lead(ui, no_smart_text(*n));
                                    }
                                    Ok(r) => disk_detail(ui, name, r, dark),
                                },
                            }
                        }
                    });
                }
            }
            let can_reload = disks.as_ref().is_ok_and(|v| !v.is_empty());
            let b = dialog::buttons(
                ui,
                [
                    dialog::btn(Role::Search, tr!("Reload")).enabled(can_reload).tooltip(tr!("Read the disk again now (otherwise once a day).")),
                    dialog::close().default(true),
                ],
            );
            reload = b[0];
            close = b[1];
        });
        if reload {
            if let Ok(names) = disks {
                self.request_health(&names, true);
            }
        } else if close {
            self.dialog = Dialog::None;
        }
    }
}

/// Free space of the panel's file system, above the SMART detail.
fn space_section(ui: &mut egui::Ui, folder: &str, space: Option<rolid_core::vfs::Space>, fstype: Option<&str>, badge: Option<&Badge>) {
    egui::Grid::new("disk-space").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label(tr!("Folder:"));
        ui.label(folder);
        ui.end_row();
        if let Some(s) = space {
            let used = s.total.saturating_sub(s.avail);
            let frac = used as f32 / s.total as f32;
            ui.label(tr!("Free space:"));
            ui.label(tr!("{free} of {total}", free = crate::pane::human_size(s.avail), total = crate::pane::human_size(s.total)));
            ui.end_row();
            ui.label(tr!("Used:"));
            let mut bar = egui::ProgressBar::new(frac).desired_width(240.0).text(format!("{} %", (frac * 100.0).round() as u32));
            if let Some(c) = space_color(s.avail, s.total) {
                bar = bar.fill(c);
            }
            ui.add(bar);
            ui.end_row();
        } else {
            ui.label(tr!("Free space:"));
            ui.label(tr!("unknown"));
            ui.end_row();
        }
        if let Some(t) = fstype {
            ui.label(tr!("File system:"));
            ui.label(t);
            ui.end_row();
        }
        if let Some(b) = badge {
            ui.label(tr!("Disk health:"));
            let font = crate::fonts::prop(12.0);
            let h = 20.0;
            let w = badge_width(ui.painter(), b, &font);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            paint_badge(ui.painter(), rect, b, &font);
            resp.on_hover_text(b.tip.clone());
            ui.end_row();
        }
    });
    ui.separator();
}

/// Hours as "1234 h (about 2.1 years)".
fn hours_text(h: u64) -> String {
    let years = h as f64 / (24.0 * 365.0);
    if years >= 0.1 {
        tr!("{hours} h (about {years} years)", hours = rolid_i18n::num(h), years = rolid_i18n::decimal(years, 1))
    } else {
        tr!("{hours} h", hours = rolid_i18n::num(h))
    }
}

fn kind_text(k: Kind) -> String {
    match k {
        Kind::Hdd => tr!("Hard disk (HDD)"),
        Kind::Ssd => tr!("SSD (SATA)"),
        Kind::Nvme => tr!("SSD (NVMe)"),
    }
}

/// Width of the value column of the detail.
const VALUE_WIDTH: f32 = 340.0;
/// Width of the label column of the detail.
const LABEL_WIDTH: f32 = 170.0;

/// One row of the detail: label, value in the state's color, explanation.
fn row(ui: &mut egui::Ui, label: String, value: String, level: Option<Level>, help: String, dark: bool) {
    ui.allocate_ui_with_layout(egui::vec2(LABEL_WIDTH, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(LABEL_WIDTH);
        ui.add(egui::Label::new(label).wrap());
    });
    let text = match level {
        Some(l) if l != Level::Ok => RichText::new(value).color(level_color(l, dark)).strong(),
        Some(_) => RichText::new(value).color(level_color(Level::Ok, dark)),
        None => RichText::new(value),
    };
    ui.allocate_ui_with_layout(egui::vec2(VALUE_WIDTH, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(VALUE_WIDTH);
        ui.add(egui::Label::new(text).wrap());
    });
    dialog::help(ui, help);
    ui.end_row();
}

fn disk_detail(ui: &mut egui::Ui, name: &str, r: &Reading, dark: bool) {
    let rep: &Report = &r.report;
    let e = &r.eval;
    let lvl = |i: Item| Some(e.level_of(i));
    ui.label(RichText::new(verdict(r)).color(level_color(e.level, dark)).strong().size(15.0));
    ui.add_space(4.0);
    egui::Grid::new(("disk-items", name)).num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
        let model = if rep.model.is_empty() { format!("/dev/{name}") } else { format!("{} (/dev/{name})", rep.model) }; // i18n-ok: model and device path
        row(ui, tr!("Model"), model, None, tr!("The name of the disk as the manufacturer gives it."), dark);
        if !rep.serial.is_empty() {
            row(ui, tr!("Serial number"), rep.serial.clone(), None, tr!("Identifies this particular disk (for example for a warranty claim)."), dark);
        }
        row(ui, tr!("Type"), kind_text(rep.kind), None, tr!("A hard disk has spinning platters; an SSD stores data in flash memory and wears out by writing."), dark);
        if let Some(w) = rep.wear_used {
            let left = (100.0 - w).max(0.0);
            let mut v = tr!("{used} % used, {left} % of life left", used = rolid_i18n::decimal(w, 0), left = rolid_i18n::decimal(left, 0));
            if let (Some(h), true) = (rep.power_on_hours, w >= 1.0) {
                let rest_years = h as f64 * left / w / (24.0 * 365.0);
                v.push_str(&tr!(" · at the current use about {years} years left", years = rolid_i18n::decimal(rest_years.min(99.0), 1)));
            }
            row(
                ui,
                tr!("Wear"),
                v,
                lvl(Item::Wear),
                tr!(
                    "How much of the rated write endurance is used up. From {warn} % plan a replacement, from {crit} % replace it.",
                    warn = rolid_i18n::decimal(rolid_disk::smart::WEAR_WARN, 0),
                    crit = rolid_i18n::decimal(rolid_disk::smart::WEAR_CRIT, 0)
                ),
                dark,
            );
        }
        if let Some(t) = rep.temp_c {
            let (warn, crit) = rolid_disk::smart::temp_limits(rep.kind);
            let checked = std::time::UNIX_EPOCH + std::time::Duration::from_secs(r.temp_at);
            row(
                ui,
                tr!("Temperature"),
                tr!("{t} °C (checked {time})", t = rolid_i18n::decimal(t, 0), time = rolid_i18n::datetime(checked, false)),
                lvl(Item::Temperature),
                tr!(
                    "The disk's temperature, checked every 5 minutes. From {warn} °C it ages faster, from {crit} °C it is dangerously hot; check the cooling.",
                    warn = rolid_i18n::decimal(warn, 0),
                    crit = rolid_i18n::decimal(crit, 0)
                ),
                dark,
            );
        }
        if let Some(h) = rep.power_on_hours {
            row(ui, tr!("Power-on time"), hours_text(h), None, tr!("How long the disk has been running in total."), dark);
        }
        if let Some(n) = rep.power_cycles {
            row(ui, tr!("Power cycles"), rolid_i18n::num(n), None, tr!("How many times the disk was switched on."), dark);
        }
        if let Some(n) = rep.unsafe_shutdowns {
            row(ui, tr!("Unexpected shutdowns"), rolid_i18n::num(n), None, tr!("Power lost without a proper shutdown (power cut, pulled cable). Many of them can damage data."), dark);
        }
        if let Some(n) = rep.reallocated {
            row(ui, tr!("Bad sectors (replaced)"), rolid_i18n::num(n), lvl(Item::Reallocated), tr!("Damaged places the disk has replaced with spare ones. A few are not a disaster; a growing number is a warning sign."), dark);
        }
        if let Some(n) = rep.pending {
            row(ui, tr!("Unstable sectors (waiting)"), rolid_i18n::num(n), lvl(Item::Pending), tr!("Places that could not be read reliably; the disk will try them again. Data in them may be lost."), dark);
        }
        if let Some(n) = rep.uncorrectable {
            row(ui, tr!("Unreadable sectors"), rolid_i18n::num(n), lvl(Item::Uncorrectable), tr!("Places the disk could not read even with error correction."), dark);
        }
        if let Some(n) = rep.media_errors {
            row(ui, tr!("Data errors"), rolid_i18n::num(n), lvl(Item::MediaErrors), tr!("Errors the NVMe disk could not correct when reading or writing."), dark);
        }
        if let Some(c) = rep.nvme_critical {
            let v = if c == 0 { tr!("none") } else { tr!("yes (code {code})", code = format!("0x{c:02x}")) }; // i18n-ok: a hex code
            row(ui, tr!("Critical warning"), v, lvl(Item::NvmeWarning), tr!("The NVMe disk itself reports a serious problem (spare space, temperature, reliability or read-only mode)."), dark);
        }
        if let Some(s) = rep.spare {
            let v = match rep.spare_thresh {
                Some(t) => tr!("{spare} % (limit {limit} %)", spare = rolid_i18n::decimal(s, 0), limit = rolid_i18n::decimal(t, 0)),
                None => tr!("{spare} %", spare = rolid_i18n::decimal(s, 0)),
            };
            row(ui, tr!("Spare space"), v, lvl(Item::Spare), tr!("Reserve memory for replacing worn-out cells. Under the limit the disk is at the end of its life."), dark);
        }
        if let Some(n) = rep.crc_errors {
            row(
                ui,
                tr!("Transfer errors (CRC)"),
                if n > 0 { tr!("{n} – usually a cable or a connector", n = rolid_i18n::num(n)) } else { rolid_i18n::num(n) },
                lvl(Item::Crc),
                tr!("Errors on the way between the disk and the computer, usually a cable or a connector, not the disk itself."),
                dark,
            );
        }
        if let Some(b) = rep.written_bytes {
            row(ui, tr!("Data written"), human_size(b), None, tr!("How much data has been written to the disk in its whole life."), dark);
        }
        if let Some(f) = rep.self_test_failed {
            let v = if f { tr!("failed") } else { tr!("passed") };
            row(ui, tr!("Last self-test"), v, lvl(Item::SelfTest), tr!("The result of the disk's own test, the last time one was run."), dark);
        }
        if rep.failing {
            row(ui, tr!("The disk's own verdict"), tr!("failing"), Some(Level::Crit), tr!("The disk itself reports that it is about to fail."), dark);
        }
    });
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(r.loaded_at);
    let source = match rep.source {
        Source::Udisks => "UDisks2".to_string(),    // i18n-ok: a program name
        Source::Smartctl => "smartctl".to_string(), // i18n-ok: a program name
        Source::Sample => trc!("smart", "sample data"),
    };
    dialog::note(ui, tr!("Read {time} from {source}.", time = rolid_i18n::datetime(when, false), source = source.as_str()));
    egui::CollapsingHeader::new(tr!("Advanced: all attributes")).id_salt(("disk-attrs", name)).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt(("disk-attrs-scroll", name)).show(ui, |ui| {
            egui::Grid::new(("disk-attr-grid", name)).striped(true).num_columns(6).show(ui, |ui| {
                for h in [tr!("ID"), tr!("Attribute"), tr!("Value"), tr!("Worst"), tr!("Threshold"), tr!("Raw value")] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                let opt = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
                for a in &rep.attrs {
                    let c = if a.failing { Some(level_color(Level::Crit, dark)) } else { None };
                    let t = |s: String| match c {
                        Some(c) => RichText::new(s).color(c),
                        None => RichText::new(s),
                    };
                    ui.label(t(if a.id > 0 { a.id.to_string() } else { String::new() }));
                    ui.label(t(a.name.clone()));
                    ui.label(t(opt(a.value)));
                    ui.label(t(opt(a.worst)));
                    ui.label(t(opt(a.thresh)));
                    ui.label(t(a.raw.clone()));
                    ui.end_row();
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolid_disk::smart::evaluate;

    fn disk(name: &str, f: impl Fn(&mut Report)) -> (String, Option<DiskHealth>) {
        let mut report = Report::new(Kind::Ssd, Source::Smartctl);
        report.model = "Test SSD".into();
        f(&mut report);
        let eval = evaluate(&report, None);
        let r = Reading { report, eval, loaded_at: 0, temp_at: 0, before: None };
        (name.to_string(), Some(DiskHealth { name: name.into(), result: Ok(r) }))
    }

    fn badge(names: &[&str], map: Vec<(String, Option<DiskHealth>)>) -> Badge {
        let disks = Some(Ok(names.iter().map(|s| s.to_string()).collect()));
        badge_of(&disks, &map.into_iter().collect()).unwrap()
    }

    #[test]
    fn badge_shows_the_state_and_the_most_serious_reason() {
        let ok = badge(&["sda"], vec![disk("sda", |r| r.temp_c = Some(35.0))]);
        assert_eq!((ok.level, ok.short.as_deref()), (Some(Level::Ok), None));
        assert!(ok.tip.contains("Test SSD"), "{}", ok.tip);

        let worn = badge(&["sda"], vec![disk("sda", |r| r.wear_used = Some(71.0))]);
        assert_eq!((worn.level, worn.short.as_deref()), (Some(Level::Warn), Some("Wear 71 %")));

        let hot = badge(&["sda"], vec![disk("sda", |r| {
            r.temp_c = Some(80.0);
            r.wear_used = Some(71.0);
        })]);
        assert_eq!((hot.level, hot.short.as_deref()), (Some(Level::Crit), Some("80 °C")), "the critical reason wins");
        assert!(hot.tip.contains("Wear 71 %") && hot.tip.contains("80 °C"), "all reasons in the tooltip: {}", hot.tip);

        // Two disks: the worse one decides.
        let two = badge(&["sda", "sdb"], vec![disk("sda", |_| {}), disk("sdb", |r| r.pending = Some(8))]);
        assert_eq!((two.level, two.short.as_deref()), (Some(Level::Warn), Some("Bad sectors")));
    }

    #[test]
    fn unknown_state_without_smart_or_before_reading() {
        let none = badge_of(&Some(Err(NoSmart::Virtual)), &HashMap::new()).unwrap();
        assert_eq!(none.level, None);
        let pending = badge(&["sda"], vec![]);
        assert_eq!(pending.level, None);
        assert!(pending.tip.contains("sda"));
        assert_eq!(badge_of(&None, &HashMap::new()), None, "disks not known yet: no badge");
    }

    #[test]
    fn free_space_bar_color() {
        assert_eq!(space_color(50, 100), None);
        assert_eq!(space_color(10, 100), None);
        assert!(space_color(9, 100).is_some_and(|c| c != space_color(4, 100).unwrap()));
        assert!(space_color(4, 100).is_some());
        assert_eq!(space_color(0, 0), None);
    }
}
