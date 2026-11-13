//! SMART readings kept on disk (`~/.cache/rolidscp/smart.json`) by serial
//! number, so a disk is read at most once a day.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::smart::{Report, Sectors, RELOAD_AFTER};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// When it was read (seconds since 1970).
    pub loaded_at: u64,
    pub report: Report,
    /// Bad sector counts of the reading before (to see them grow).
    pub before: Option<Sectors>,
}

impl Entry {
    /// Read less than a day ago.
    pub fn fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.loaded_at) < RELOAD_AFTER && self.loaded_at <= now
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    pub disks: HashMap<String, Entry>,
}

pub fn default_path() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("rolidscp").join("smart.json")
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Cache {
    pub fn load(path: &Path) -> Cache {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Write through a temporary file (a crash never leaves half a file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    /// A reading of `serial` less than a day old (unless `force`).
    pub fn fresh(&self, serial: &str, now: u64, force: bool) -> Option<&Entry> {
        self.disks.get(serial).filter(|e| !force && e.fresh(now))
    }

    /// Store a new reading; the counts it is compared with come from the
    /// previous day's reading (a reload on the same day keeps them).
    pub fn put(&mut self, serial: &str, report: Report, now: u64) -> &Entry {
        let before = self.disks.get(serial).and_then(|old| if old.fresh(now) { old.before } else { Some(old.report.sectors()) });
        self.disks.insert(serial.to_string(), Entry { loaded_at: now, report, before });
        &self.disks[serial]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart::{Kind, Source};

    fn report(realloc: u64) -> Report {
        let mut r = Report::new(Kind::Hdd, Source::Udisks);
        r.serial = "X1".into();
        r.reallocated = Some(realloc);
        r
    }

    #[test]
    fn once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smart.json");
        let t0 = 1_700_000_000;
        let mut c = Cache::load(&path);
        assert!(c.fresh("X1", t0, false).is_none());
        c.put("X1", report(2), t0);
        c.save(&path).unwrap();
        let c2 = Cache::load(&path);
        // The same day: from the cache; "Reload" reads again.
        assert!(c2.fresh("X1", t0 + 3600, false).is_some());
        assert!(c2.fresh("X1", t0 + 3600, true).is_none());
        assert!(c2.fresh("X2", t0 + 3600, false).is_none());
        // A day later: read again.
        assert!(c2.fresh("X1", t0 + RELOAD_AFTER, false).is_none());
        // A clock set back is not "fresh" for ever.
        assert!(c2.fresh("X1", t0 - 10, false).is_none());
        let mut c3 = c2.clone();
        let e = c3.put("X1", report(5), t0 + RELOAD_AFTER + 1);
        assert_eq!(e.before.unwrap().reallocated, Some(2));
        // Reloaded the same day: compared with the same earlier reading.
        let e = c3.put("X1", report(6), t0 + RELOAD_AFTER + 100);
        assert_eq!(e.before.unwrap().reallocated, Some(2));
        // A damaged file is an empty cache.
        std::fs::write(&path, b"{oops").unwrap();
        assert!(Cache::load(&path).disks.is_empty());
    }
}
