//! SMART data of one disk (from UDisks2 or smartctl) and its evaluation:
//! OK, warning or critical, item by item.

use serde::{Deserialize, Serialize};

/// SSD wear (percent of the rated life used): warning from this.
pub const WEAR_WARN: f64 = 60.0;
/// SSD wear: critical from this.
pub const WEAR_CRIT: f64 = 95.0;
/// Reallocated, pending or uncorrectable sectors: warning above this.
pub const SECTORS_WARN: u64 = 0;
/// … critical above this (or when the count grew since the last reading).
pub const SECTORS_CRIT: u64 = 50;
/// Temperature of a hard disk: warning from this (°C).
pub const TEMP_WARN_HDD: f64 = 45.0;
/// Temperature of a hard disk: critical from this (°C).
pub const TEMP_CRIT_HDD: f64 = 55.0;
/// Temperature of an SSD or NVMe: warning from this (°C).
pub const TEMP_WARN_SSD: f64 = 60.0;
/// Temperature of an SSD or NVMe: critical from this (°C).
pub const TEMP_CRIT_SSD: f64 = 75.0;
/// NVMe media and data integrity errors: warning above this.
pub const MEDIA_ERRORS_WARN: u64 = 0;
/// Interface (CRC) errors: the item is marked above this, the disk state is
/// not changed by it (usually a cable or a connector).
pub const CRC_MARK: u64 = 0;
/// How often a disk is read again (seconds): once a day.
pub const RELOAD_AFTER: u64 = 24 * 3600;
/// How often the temperature is checked again (seconds): every 5 minutes.
pub const TEMP_RECHECK: u64 = 5 * 60;

/// The temperature limits (warning, critical) of a kind of disk, in °C.
pub fn temp_limits(kind: Kind) -> (f64, f64) {
    match kind {
        Kind::Hdd => (TEMP_WARN_HDD, TEMP_CRIT_HDD),
        Kind::Ssd | Kind::Nvme => (TEMP_WARN_SSD, TEMP_CRIT_SSD),
    }
}

/// The state of a temperature reading.
pub fn temp_level(kind: Kind, t: f64) -> Level {
    let (warn, crit) = temp_limits(kind);
    if t >= crit {
        Level::Crit
    } else if t >= warn {
        Level::Warn
    } else {
        Level::Ok
    }
}

/// The state of SSD wear (percent used).
pub fn wear_level(w: f64) -> Level {
    if w >= WEAR_CRIT {
        Level::Crit
    } else if w >= WEAR_WARN {
        Level::Warn
    } else {
        Level::Ok
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Hdd,
    Ssd,
    Nvme,
}

/// Where the data came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Udisks,
    Smartctl,
    /// Sample data for testing (`ROLIDSCP_SMART_SAMPLE`).
    Sample,
}

/// One row of the full attribute table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Attr {
    /// ATA attribute number (0 for NVMe log fields).
    pub id: u32,
    pub name: String,
    /// Normalized value, worst and threshold (ATA).
    pub value: Option<i64>,
    pub worst: Option<i64>,
    pub thresh: Option<i64>,
    /// Raw value as the source shows it.
    pub raw: String,
    /// The normalized value is at or under the threshold.
    pub failing: bool,
}

/// What a disk says about itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub source: Source,
    pub temp_c: Option<f64>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    /// Percent of the rated life used (SSD, NVMe).
    pub wear_used: Option<f64>,
    pub reallocated: Option<u64>,
    pub pending: Option<u64>,
    pub uncorrectable: Option<u64>,
    pub crc_errors: Option<u64>,
    pub written_bytes: Option<u64>,
    /// NVMe critical warning bits (0 = none).
    pub nvme_critical: Option<u32>,
    pub spare: Option<f64>,
    pub spare_thresh: Option<f64>,
    pub media_errors: Option<u64>,
    /// The last self-test failed (`None`: none known).
    pub self_test_failed: Option<bool>,
    /// The disk itself says it is failing (overall SMART status).
    pub failing: bool,
    pub attrs: Vec<Attr>,
}

impl Report {
    pub fn new(kind: Kind, source: Source) -> Self {
        Report {
            model: String::new(),
            serial: String::new(),
            firmware: String::new(),
            kind,
            size: None,
            source,
            temp_c: None,
            power_on_hours: None,
            power_cycles: None,
            unsafe_shutdowns: None,
            wear_used: None,
            reallocated: None,
            pending: None,
            uncorrectable: None,
            crc_errors: None,
            written_bytes: None,
            nvme_critical: None,
            spare: None,
            spare_thresh: None,
            media_errors: None,
            self_test_failed: None,
            failing: false,
            attrs: Vec::new(),
        }
    }

    /// Bad sector counts, to compare with a later reading.
    pub fn sectors(&self) -> Sectors {
        Sectors { reallocated: self.reallocated, pending: self.pending, uncorrectable: self.uncorrectable }
    }

    /// Fill the summary fields from ATA attributes (both sources use the same numbers).
    pub fn fill_from_ata_attrs(&mut self) {
        let raw = |id: u32| self.attrs.iter().find(|a| a.id == id).and_then(|a| raw_number(&a.raw));
        let norm = |id: u32| self.attrs.iter().find(|a| a.id == id).and_then(|a| a.value);
        self.reallocated = self.reallocated.or(raw(5));
        self.pending = self.pending.or(raw(197));
        self.uncorrectable = self.uncorrectable.or(raw(198));
        self.crc_errors = self.crc_errors.or(raw(199));
        self.power_on_hours = self.power_on_hours.or(raw(9));
        self.power_cycles = self.power_cycles.or(raw(12));
        // 174: unexpected power loss (SSD); 192: emergency retracts after a power loss (HDD).
        self.unsafe_shutdowns = self.unsafe_shutdowns.or(raw(174)).or(raw(192));
        if self.temp_c.is_none() {
            self.temp_c = raw(194).or(raw(190)).map(|t| (t & 0xff) as f64);
        }
        if self.kind == Kind::Ssd && self.wear_used.is_none() {
            // Remaining life as the normalized value (Samsung 177, Intel 233, others 231, 202, 169).
            self.wear_used = [233, 177, 231, 202, 169].iter().find_map(|&id| norm(id)).filter(|v| (0..=100).contains(v)).map(|v| (100 - v) as f64);
        }
        if self.written_bytes.is_none() {
            if let Some(a) = self.attrs.iter().find(|a| a.id == 241) {
                let n = a.name.to_ascii_lowercase();
                let unit: u64 = if n.contains("32mib") {
                    32 << 20
                } else if n.contains("gib") || n.contains("gb") {
                    1 << 30
                } else {
                    512
                };
                self.written_bytes = raw_number(&a.raw).map(|v| v.saturating_mul(unit));
            }
        }
        if self.attrs.iter().any(|a| a.failing) {
            self.failing = true;
        }
    }
}

/// The leading number of a raw value ("35 (Min/Max 20/41)" → 35, "1234h+05m" → 1234).
pub fn raw_number(s: &str) -> Option<u64> {
    let d: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sectors {
    pub reallocated: Option<u64>,
    pub pending: Option<u64>,
    pub uncorrectable: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Ok,
    Warn,
    Crit,
}

/// The items of the lay-person detail, each with its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Overall,
    SelfTest,
    Wear,
    Temperature,
    PowerOn,
    PowerCycles,
    UnsafeShutdowns,
    Reallocated,
    Pending,
    Uncorrectable,
    Crc,
    Written,
    NvmeWarning,
    Spare,
    MediaErrors,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub level: Level,
    /// The main reason (for the verdict sentence), when not OK.
    pub reason: Option<Item>,
    pub items: Vec<(Item, Level)>,
    /// Bad sectors grew since the previous reading.
    pub growing: bool,
}

impl Evaluation {
    pub fn level_of(&self, item: Item) -> Level {
        self.items.iter().find(|(i, _)| *i == item).map(|(_, l)| *l).unwrap_or(Level::Ok)
    }
}

fn sector_level(n: Option<u64>, before: Option<u64>) -> Option<(Level, bool)> {
    let n = n?;
    let grew = before.is_some_and(|b| n > b);
    Some(if n > SECTORS_CRIT || (grew && n > SECTORS_WARN) {
        (Level::Crit, grew)
    } else if n > SECTORS_WARN {
        (Level::Warn, false)
    } else {
        (Level::Ok, false)
    })
}

/// Evaluate a reading; `before` is the previous reading's bad sector counts.
pub fn evaluate(r: &Report, before: Option<Sectors>) -> Evaluation {
    let mut items: Vec<(Item, Level)> = Vec::new();
    let mut growing = false;
    // The order is the order of importance for the verdict sentence.
    items.push((Item::Overall, if r.failing { Level::Crit } else { Level::Ok }));
    if let Some(f) = r.self_test_failed {
        items.push((Item::SelfTest, if f { Level::Crit } else { Level::Ok }));
    }
    if let Some(c) = r.nvme_critical {
        items.push((Item::NvmeWarning, if c != 0 { Level::Crit } else { Level::Ok }));
    }
    if let Some(s) = r.spare {
        let low = r.spare_thresh.is_some_and(|t| s < t) || s <= 10.0;
        items.push((Item::Spare, if low { Level::Crit } else { Level::Ok }));
    }
    let b = before.unwrap_or_default();
    for (item, n, prev) in [
        (Item::Reallocated, r.reallocated, b.reallocated),
        (Item::Pending, r.pending, b.pending),
        (Item::Uncorrectable, r.uncorrectable, b.uncorrectable),
    ] {
        if let Some((l, g)) = sector_level(n, prev) {
            growing |= g;
            items.push((item, l));
        }
    }
    if let Some(w) = r.wear_used {
        items.push((Item::Wear, wear_level(w)));
    }
    if let Some(m) = r.media_errors {
        items.push((Item::MediaErrors, if m > MEDIA_ERRORS_WARN { Level::Warn } else { Level::Ok }));
    }
    if let Some(t) = r.temp_c {
        items.push((Item::Temperature, temp_level(r.kind, t)));
    }
    let crc = r.crc_errors.map(|c| (Item::Crc, if c > CRC_MARK { Level::Warn } else { Level::Ok }));
    for (item, v) in [(Item::PowerOn, r.power_on_hours), (Item::PowerCycles, r.power_cycles), (Item::UnsafeShutdowns, r.unsafe_shutdowns)] {
        if v.is_some() {
            items.push((item, Level::Ok));
        }
    }
    if r.written_bytes.is_some() {
        items.push((Item::Written, Level::Ok));
    }
    // CRC errors are shown, but do not decide the state of the disk.
    let level = items.iter().map(|(_, l)| *l).max().unwrap_or(Level::Ok);
    let reason = (level > Level::Ok).then(|| items.iter().find(|(_, l)| *l == level).map(|(i, _)| *i)).flatten();
    if let Some(c) = crc {
        items.push(c);
    }
    Evaluation { level, reason, items, growing }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn ata(id: u32, name: &str, value: i64, raw: &str) -> Attr {
        Attr { id, name: name.into(), value: Some(value), worst: Some(value), thresh: Some(10), raw: raw.into(), failing: false }
    }

    pub fn healthy_ssd() -> Report {
        let mut r = Report::new(Kind::Ssd, Source::Smartctl);
        r.model = "Samsung SSD 870 EVO 1TB".into();
        r.serial = "S6PTNX0R123456".into();
        r.attrs = vec![
            ata(5, "Reallocated_Sector_Ct", 100, "0"),
            ata(9, "Power_On_Hours", 98, "8123"),
            ata(12, "Power_Cycle_Count", 99, "1520"),
            ata(177, "Wear_Leveling_Count", 93, "61"),
            ata(190, "Airflow_Temperature_Cel", 63, "37"),
            ata(199, "UDMA_CRC_Error_Count", 100, "0"),
            ata(241, "Total_LBAs_Written", 99, "39062500000"),
        ];
        r.self_test_failed = Some(false);
        r.fill_from_ata_attrs();
        r
    }

    #[test]
    fn healthy_ssd_is_ok() {
        let r = healthy_ssd();
        assert_eq!(r.wear_used, Some(7.0));
        assert_eq!(r.temp_c, Some(37.0));
        assert_eq!(r.power_on_hours, Some(8123));
        assert_eq!(r.written_bytes, Some(39062500000 * 512));
        let e = evaluate(&r, None);
        assert_eq!(e.level, Level::Ok);
        assert_eq!(e.reason, None);
        assert_eq!(e.level_of(Item::Wear), Level::Ok);
    }

    #[test]
    fn worn_ssd() {
        let mut r = healthy_ssd();
        r.attrs.iter_mut().find(|a| a.id == 177).unwrap().value = Some(15);
        r.wear_used = None;
        r.fill_from_ata_attrs();
        assert_eq!(r.wear_used, Some(85.0));
        let e = evaluate(&r, None);
        assert_eq!((e.level, e.reason), (Level::Warn, Some(Item::Wear)));
        r.wear_used = Some(97.0);
        assert_eq!(evaluate(&r, None).level, Level::Crit);
        // Just under and at each limit.
        for (w, l) in [(59.0, Level::Ok), (59.9, Level::Ok), (60.0, Level::Warn), (61.0, Level::Warn), (94.0, Level::Warn), (94.9, Level::Warn), (95.0, Level::Crit), (96.0, Level::Crit)] {
            r.wear_used = Some(w);
            let e = evaluate(&r, None);
            assert_eq!(e.level, l, "wear {w} %");
            assert_eq!(e.level_of(Item::Wear), l, "wear {w} %");
        }
    }

    #[test]
    fn hdd_with_reallocated_sectors() {
        let mut r = Report::new(Kind::Hdd, Source::Udisks);
        r.attrs = vec![ata(5, "reallocated-sector-count", 98, "8"), ata(194, "temperature-celsius-2", 40, "41"), ata(197, "current-pending-sector", 100, "0")];
        r.fill_from_ata_attrs();
        let e = evaluate(&r, None);
        assert_eq!((e.level, e.reason), (Level::Warn, Some(Item::Reallocated)));
        assert!(!e.growing);
        // The same count as last time: still a warning.
        assert_eq!(evaluate(&r, Some(r.sectors())).level, Level::Warn);
        // Grown since the last reading: critical.
        let before = Sectors { reallocated: Some(3), ..Default::default() };
        let e = evaluate(&r, Some(before));
        assert_eq!((e.level, e.growing), (Level::Crit, true));
        // Many: critical.
        r.reallocated = Some(51);
        assert_eq!(evaluate(&r, None).level, Level::Crit);
        // Hot hard disk.
        r.reallocated = Some(0);
        r.temp_c = Some(46.0);
        assert_eq!((evaluate(&r, None).level, evaluate(&r, None).reason), (Level::Warn, Some(Item::Temperature)));
        r.temp_c = Some(56.0);
        assert_eq!((evaluate(&r, None).level, evaluate(&r, None).reason), (Level::Crit, Some(Item::Temperature)));
    }

    #[test]
    fn nvme_with_critical_warning() {
        let mut r = Report::new(Kind::Nvme, Source::Udisks);
        r.nvme_critical = Some(0);
        r.spare = Some(100.0);
        r.spare_thresh = Some(10.0);
        r.wear_used = Some(3.0);
        r.temp_c = Some(55.0);
        assert_eq!(evaluate(&r, None).level, Level::Ok, "55 °C is fine for an NVMe");
        r.temp_c = Some(65.0);
        assert_eq!(evaluate(&r, None).level, Level::Warn, "65 °C is warm for an NVMe");
        r.temp_c = Some(40.0);
        r.nvme_critical = Some(0x01);
        let e = evaluate(&r, None);
        assert_eq!((e.level, e.reason), (Level::Crit, Some(Item::NvmeWarning)));
        r.nvme_critical = Some(0);
        r.spare = Some(5.0);
        assert_eq!((evaluate(&r, None).level, evaluate(&r, None).reason), (Level::Crit, Some(Item::Spare)));
    }

    #[test]
    fn failed_self_test_and_crc() {
        let mut r = healthy_ssd();
        r.crc_errors = Some(12);
        let e = evaluate(&r, None);
        assert_eq!(e.level, Level::Ok, "CRC errors alone do not change the state");
        assert_eq!(e.level_of(Item::Crc), Level::Warn);
        r.self_test_failed = Some(true);
        assert_eq!((evaluate(&r, None).level, evaluate(&r, None).reason), (Level::Crit, Some(Item::SelfTest)));
    }

    #[test]
    fn temperature_limits() {
        // Hard disk: warning from 45 °C, critical from 55 °C.
        let hdd = [(44.0, Level::Ok), (44.9, Level::Ok), (45.0, Level::Warn), (46.0, Level::Warn), (54.0, Level::Warn), (54.9, Level::Warn), (55.0, Level::Crit), (56.0, Level::Crit)];
        // SSD and NVMe: warning from 60 °C, critical from 75 °C.
        let ssd = [(59.0, Level::Ok), (59.9, Level::Ok), (60.0, Level::Warn), (61.0, Level::Warn), (74.0, Level::Warn), (74.9, Level::Warn), (75.0, Level::Crit), (76.0, Level::Crit)];
        for (kind, cases) in [(Kind::Hdd, hdd), (Kind::Ssd, ssd), (Kind::Nvme, ssd)] {
            for (t, l) in cases {
                let mut r = Report::new(kind, Source::Sample);
                r.temp_c = Some(t);
                let e = evaluate(&r, None);
                assert_eq!(e.level, l, "{kind:?} at {t} °C");
                assert_eq!(e.level_of(Item::Temperature), l, "{kind:?} at {t} °C");
                if l != Level::Ok {
                    assert_eq!(e.reason, Some(Item::Temperature));
                }
            }
        }
    }

    #[test]
    fn raw_numbers() {
        assert_eq!(raw_number("35 (Min/Max 20/41)"), Some(35));
        assert_eq!(raw_number("1234h+05m+10.123s"), Some(1234));
        assert_eq!(raw_number("-"), None);
    }
}
