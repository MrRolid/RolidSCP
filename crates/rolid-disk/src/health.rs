//! The state of the disks under a local path, for the panel: which disks,
//! their SMART readings (from the cache, UDisks2 or smartctl) and evaluation.

use std::path::Path;

use crate::cache::{self, Cache};
use crate::mounts;
use crate::physical::{self, NoSmart, RealSys};
use crate::smart::{evaluate, Evaluation, Kind, Report, Sectors, Source, TEMP_RECHECK};

/// One physical disk and what is known about its health.
#[derive(Clone, Debug)]
pub struct DiskHealth {
    /// Kernel name (`sda`, `nvme0n1`).
    pub name: String,
    pub result: Result<Reading, NoSmart>,
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub report: Report,
    pub eval: Evaluation,
    /// When it was read (seconds since 1970).
    pub loaded_at: u64,
    /// When the temperature was read (every few minutes, see `TEMP_RECHECK`).
    pub temp_at: u64,
    /// Bad sector counts of the earlier reading (to evaluate again).
    pub before: Option<Sectors>,
}

impl Reading {
    /// A new temperature: evaluate again.
    pub fn set_temperature(&mut self, t: Option<f64>, at: u64) {
        if t.is_some() {
            self.report.temp_c = t;
        }
        self.temp_at = at;
        self.eval = evaluate(&self.report, self.before);
    }

    /// The temperature is older than `TEMP_RECHECK`.
    pub fn temp_due(&self, now: u64) -> bool {
        now.saturating_sub(self.temp_at) >= TEMP_RECHECK || self.temp_at > now
    }
}

/// The physical disks of a local path, or why it has none. Blocking (reads `/proc`, `/sys`).
pub fn disks_of_path(path: &str) -> Result<Vec<String>, NoSmart> {
    let m = mounts::mount_of(path).ok_or(NoSmart::NoDisk)?;
    physical::disks_of(&RealSys::default(), &m)
}

/// Sample data instead of the real disks (testing): the path of a `smartctl -j`
/// file. Only in a debug build; a release build ignores it.
pub const SAMPLE_ENV: &str = "ROLIDSCP_SMART_SAMPLE";

/// The sample file, in a debug build only.
fn sample() -> Option<std::ffi::OsString> {
    if cfg!(debug_assertions) {
        std::env::var_os(SAMPLE_ENV)
    } else {
        None
    }
}

/// Read the health of one disk: from the cache when read less than a day
/// ago (unless `force`), else from UDisks2, else smartctl. Blocking; never
/// wakes up a sleeping hard disk through smartctl.
pub fn read_disk(name: &str, cache_path: &Path, force: bool) -> DiskHealth {
    let result = match sample() {
        Some(p) => std::fs::read_to_string(p).ok().and_then(|j| crate::smartctl::parse(&j, Source::Sample)).ok_or(NoSmart::NotOffered).map(|r| reading(r, None, cache::now())),
        None => read_real(name, cache_path, force),
    };
    DiskHealth { name: name.to_string(), result }
}

/// The temperature of a disk now (°C), read every few minutes. Blocking;
/// never wakes up a sleeping disk. `None` when it cannot be read.
pub fn read_temperature(name: &str, kind: Kind) -> Option<f64> {
    if let Some(p) = sample() {
        return std::fs::read_to_string(p).ok().and_then(|j| crate::smartctl::temperature_of(&j));
    }
    // NVMe and SSD: the kernel's sensor (no D-Bus, no waking up); not for
    // hard disks, where reading it might spin up a sleeping disk.
    if kind != Kind::Hdd {
        if let Some(t) = hwmon_temperature(Path::new("/sys/block").join(name).as_path()) {
            return Some(t);
        }
    }
    match crate::udisks::drive(name) {
        Some(d) => d.temperature(),
        None => crate::smartctl::temperature(name),
    }
}

/// `temp1_input` of a hwmon sensor under the device (millidegrees).
pub fn hwmon_temperature(block: &Path) -> Option<f64> {
    let dev = block.join("device");
    for base in [dev.clone(), dev.join("hwmon"), dev.join("device")] {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().starts_with("hwmon") {
                continue;
            }
            let p = e.path().join("temp1_input");
            if let Some(m) = std::fs::read_to_string(&p).ok().and_then(|s| s.trim().parse::<i64>().ok()) {
                // An absurd value is a sensor error.
                if (-40_000..=150_000).contains(&m) {
                    return Some((m as f64 / 1000.0).round());
                }
            }
        }
    }
    None
}

fn reading(report: Report, before: Option<Sectors>, loaded_at: u64) -> Reading {
    let eval = evaluate(&report, before);
    Reading { report, eval, loaded_at, temp_at: loaded_at, before }
}

fn read_real(name: &str, cache_path: &Path, force: bool) -> Result<Reading, NoSmart> {
    let sys = RealSys::default();
    let (vendor, model, hw_path) = sys.facts(name);
    if physical::is_virtual(name, &vendor, &model, &hw_path) {
        return Err(NoSmart::Virtual);
    }
    let drive = crate::udisks::drive(name);
    let serial = drive.as_ref().map(|d| d.serial.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let s = std::fs::read_to_string(format!("/sys/block/{name}/device/serial")).unwrap_or_default();
        if s.trim().is_empty() { format!("{name}:{}", model.trim()) } else { s.trim().to_string() }
    });
    let now = cache::now();
    let mut c = Cache::load(cache_path);
    if let Some(e) = c.fresh(&serial, now, force) {
        let mut r = reading(e.report.clone(), e.before, e.loaded_at);
        // From the cache: the temperature may be hours old.
        if r.temp_due(now) {
            let t = read_temperature(name, r.report.kind);
            r.set_temperature(t, now);
        }
        return Ok(r);
    }
    let report = drive.as_ref().and_then(|d| d.smart()).or_else(|| crate::smartctl::read(name)).ok_or(NoSmart::NotOffered)?;
    let e = c.put(&serial, report, now).clone();
    let _ = c.save(cache_path);
    Ok(reading(e.report, e.before, e.loaded_at))
}

/// The worst state of several disks (a RAID).
pub fn worst(disks: &[DiskHealth]) -> Option<crate::smart::Level> {
    disks.iter().filter_map(|d| d.result.as_ref().ok()).map(|r| r.eval.level).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_from_hwmon() {
        let dir = tempfile::tempdir().unwrap();
        let block = dir.path().join("nvme0n1");
        let hw = block.join("device/hwmon3");
        std::fs::create_dir_all(&hw).unwrap();
        std::fs::write(hw.join("temp1_input"), "41850\n").unwrap();
        assert_eq!(hwmon_temperature(&block), Some(42.0));
        // drivetemp puts it under device/hwmon/hwmonN.
        let block2 = dir.path().join("sda");
        let hw2 = block2.join("device/hwmon/hwmon5");
        std::fs::create_dir_all(&hw2).unwrap();
        std::fs::write(hw2.join("temp1_input"), "-1000000").unwrap();
        assert_eq!(hwmon_temperature(&block2), None, "a sensor error");
        assert_eq!(hwmon_temperature(&dir.path().join("sdz")), None);
    }

    #[test]
    fn new_temperature_is_evaluated_again() {
        let mut rep = Report::new(Kind::Nvme, Source::Sample);
        rep.temp_c = Some(40.0);
        let mut r = reading(rep, None, 1000);
        assert_eq!(r.eval.level, crate::smart::Level::Ok);
        assert!(!r.temp_due(1000 + TEMP_RECHECK - 1));
        assert!(r.temp_due(1000 + TEMP_RECHECK));
        r.set_temperature(Some(76.0), 1300);
        assert_eq!(r.eval.level, crate::smart::Level::Crit);
        assert_eq!(r.temp_at, 1300);
        // Not readable now: the last value stays.
        r.set_temperature(None, 1600);
        assert_eq!(r.report.temp_c, Some(76.0));
    }

    #[test]
    fn sample_only_in_a_debug_build() {
        assert_eq!(sample().is_some(), cfg!(debug_assertions) && std::env::var_os(SAMPLE_ENV).is_some());
    }
}
