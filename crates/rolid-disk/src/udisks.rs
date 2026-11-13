//! SMART data from UDisks2 over D-Bus (no root needed): `Drive.Ata` for
//! SATA disks, `NVMe.Controller` for NVMe.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::smart::{Attr, Kind, Report, Source};

const DEST: &str = "org.freedesktop.UDisks2";

/// Object path part for a kernel device name (UDisks escapes all but letters and digits).
fn escape(name: &str) -> String {
    let mut s = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            s.push(b as char);
        } else {
            s.push_str(&format!("_{b:02x}"));
        }
    }
    s
}

/// The drive of a whole disk and what UDisks knows about it.
pub struct Drive {
    conn: Connection,
    path: OwnedObjectPath,
    pub model: String,
    pub serial: String,
    pub bus: String,
}

fn proxy<'a>(conn: &Connection, path: &'a OwnedObjectPath, iface: &'a str) -> zbus::Result<Proxy<'a>> {
    Proxy::new(conn, DEST, path.as_str(), iface)
}

/// The UDisks drive of `disk` (`sda`, `nvme0n1`); `None` without UDisks.
pub fn drive(disk: &str) -> Option<Drive> {
    let conn = Connection::system().ok()?;
    let block = OwnedObjectPath::try_from(format!("/org/freedesktop/UDisks2/block_devices/{}", escape(disk))).ok()?;
    let b = proxy(&conn, &block, "org.freedesktop.UDisks2.Block").ok()?;
    let path: OwnedObjectPath = b.get_property("Drive").ok()?;
    if path.as_str() == "/" {
        return None;
    }
    let (model, serial, bus) = {
        let d = proxy(&conn, &path, "org.freedesktop.UDisks2.Drive").ok()?;
        let g = |k: &str| d.get_property::<String>(k).unwrap_or_default();
        (g("Model"), g("Serial"), g("ConnectionBus"))
    };
    Some(Drive { conn, path, model, serial, bus })
}

fn num(v: &OwnedValue) -> Option<u64> {
    if let Ok(x) = u64::try_from(v) {
        return Some(x);
    }
    if let Ok(x) = i64::try_from(v) {
        return u64::try_from(x).ok();
    }
    if let Ok(x) = u32::try_from(v) {
        return Some(x as u64);
    }
    if let Ok(x) = u16::try_from(v) {
        return Some(x as u64);
    }
    if let Ok(x) = u8::try_from(v) {
        return Some(x as u64);
    }
    None
}

/// Kelvin from UDisks to °C (0 = unknown).
fn kelvin(k: f64) -> Option<f64> {
    (k > 1.0).then(|| (k - 273.15).round())
}

impl Drive {
    /// Read SMART; `None` when UDisks has no SMART data for this drive.
    pub fn smart(&self) -> Option<Report> {
        let mut r = self.nvme().or_else(|| self.ata())?;
        r.model = self.model.clone();
        r.serial = self.serial.clone();
        if let Ok(d) = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.Drive") {
            r.firmware = d.get_property::<String>("Revision").unwrap_or_default();
            r.size = d.get_property::<u64>("Size").ok().filter(|s| *s > 0);
        }
        Some(r)
    }

    /// The temperature now (°C). Asks UDisks to read the disk again first, but
    /// never wakes up a sleeping disk; without the right to do that UDisks
    /// gives its own last reading (it refreshes it every few minutes).
    pub fn temperature(&self) -> Option<f64> {
        let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        opts.insert("nowakeup", zbus::zvariant::Value::from(true));
        if let Ok(p) = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.NVMe.Controller") {
            if let Ok(k) = p.get_property::<u16>("SmartTemperature") {
                let _ = p.call_method("SmartUpdate", &(HashMap::<&str, zbus::zvariant::Value>::new(),));
                let k = p.get_property::<u16>("SmartTemperature").unwrap_or(k);
                return kelvin(k as f64);
            }
        }
        let p = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.Drive.Ata").ok()?;
        if !p.get_property::<bool>("SmartSupported").unwrap_or(false) {
            return None;
        }
        let _ = p.call_method("SmartUpdate", &(opts,));
        p.get_property::<f64>("SmartTemperature").ok().and_then(kelvin)
    }

    fn nvme(&self) -> Option<Report> {
        let p = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.NVMe.Controller").ok()?;
        let updated: u64 = p.get_property("SmartUpdated").ok()?;
        if updated == 0 {
            return None;
        }
        let mut r = Report::new(Kind::Nvme, Source::Udisks);
        let warn: Vec<String> = p.get_property("SmartCriticalWarning").unwrap_or_default();
        r.nvme_critical = Some(warn.iter().fold(0u32, |acc, w| {
            acc | match w.as_str() {
                "spare" => 0x01,
                "temperature" => 0x02,
                "degraded" => 0x04,
                "readonly" => 0x08,
                "volatile_mem" => 0x10,
                "pmr_readonly" => 0x20,
                _ => 0x80,
            }
        }));
        r.power_on_hours = p.get_property::<u64>("SmartPowerOnHours").ok();
        r.temp_c = p.get_property::<u16>("SmartTemperature").ok().and_then(|k| kelvin(k as f64));
        let opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        let a: HashMap<String, OwnedValue> = p.call("SmartGetAttributes", &(opts,)).unwrap_or_default();
        let g = |k: &str| a.get(k).and_then(num);
        r.spare = g("avail_spare").map(|v| v as f64);
        r.spare_thresh = g("spare_thresh").map(|v| v as f64);
        r.wear_used = g("percent_used").map(|v| v as f64);
        r.written_bytes = g("total_data_written");
        r.power_cycles = g("power_cycles");
        r.unsafe_shutdowns = g("unsafe_shutdowns");
        r.media_errors = g("media_errors");
        let mut keys: Vec<&String> = a.keys().collect();
        keys.sort();
        for k in keys {
            let raw = a.get(k).and_then(num).map(|n| n.to_string()).unwrap_or_else(|| format!("{:?}", a[k]));
            r.attrs.push(Attr { id: 0, name: k.clone(), raw, ..Default::default() });
        }
        Some(r)
    }

    fn ata(&self) -> Option<Report> {
        let p = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.Drive.Ata").ok()?;
        let supported: bool = p.get_property("SmartSupported").unwrap_or(false);
        let updated: u64 = p.get_property("SmartUpdated").unwrap_or(0);
        if !supported || updated == 0 {
            return None;
        }
        let rotation: i32 = proxy(&self.conn, &self.path, "org.freedesktop.UDisks2.Drive").ok().and_then(|d| d.get_property("RotationRate").ok()).unwrap_or(-1);
        let mut r = Report::new(if rotation == 0 { Kind::Ssd } else { Kind::Hdd }, Source::Udisks);
        r.failing = p.get_property("SmartFailing").unwrap_or(false);
        r.temp_c = p.get_property::<f64>("SmartTemperature").ok().and_then(kelvin);
        r.power_on_hours = p.get_property::<u64>("SmartPowerOnSeconds").ok().filter(|s| *s > 0).map(|s| s / 3600);
        let st: String = p.get_property("SmartSelftestStatus").unwrap_or_default();
        r.self_test_failed = match st.as_str() {
            "success" => Some(false),
            s if s.starts_with("error") || s == "fatal" => Some(true),
            _ => None,
        };
        type Row = (u8, String, u16, i32, i32, i32, i64, i32, HashMap<String, OwnedValue>);
        let opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        let rows: Vec<Row> = p.call("SmartGetAttributes", &(opts,)).unwrap_or_default();
        for (id, name, _flags, value, worst, thresh, pretty, unit, _) in rows {
            // libatasmart units: 2 ms, 3 sectors, 4 mK, 5/6 percent, 7 MB.
            let raw = match unit {
                2 => format!("{} h", pretty / 3_600_000),
                4 => format!("{} °C", (pretty - 273_150) / 1000), // i18n-ok: a unit
                7 => format!("{pretty} MB"), // i18n-ok: a unit
                _ => pretty.to_string(),
            };
            if id == 241 && unit == 7 {
                r.written_bytes = u64::try_from(pretty).ok().map(|v| v << 20);
            }
            let opt = |v: i32| (v >= 0).then_some(v as i64);
            let failing = value >= 0 && thresh > 0 && value <= thresh;
            r.attrs.push(Attr { id: id as u32, name, value: opt(value), worst: opt(worst), thresh: opt(thresh), raw, failing });
        }
        r.fill_from_ata_attrs();
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn object_path_names() {
        assert_eq!(super::escape("nvme0n1"), "nvme0n1");
        assert_eq!(super::escape("dm-0"), "dm_2d0");
    }
}
