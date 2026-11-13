//! SMART data from `smartctl -j` (when UDisks has none and smartctl runs
//! without root), and the sample data for testing.

use serde_json::Value;

use crate::smart::{Attr, Kind, Report, Source};

/// Run smartctl for `/dev/<disk>` (a sleeping hard disk is not woken up).
/// `None` when smartctl is missing, has no access or the disk has no SMART.
pub fn read(disk: &str) -> Option<Report> {
    let out = std::process::Command::new("smartctl")
        .args(["-j", "-a", "-n", "standby,2"])
        .arg(format!("/dev/{disk}"))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    // Bits 0 and 1 of the exit status: bad command line, device not opened.
    if out.status.code().is_none_or(|c| c & 0b11 != 0) {
        return None;
    }
    parse(&String::from_utf8_lossy(&out.stdout), Source::Smartctl)
}

/// Only the temperature (°C) from `smartctl -j -A`; a sleeping disk is not woken up.
pub fn temperature(disk: &str) -> Option<f64> {
    let out = std::process::Command::new("smartctl")
        .args(["-j", "-A", "-n", "standby,2"])
        .arg(format!("/dev/{disk}"))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if out.status.code().is_none_or(|c| c & 0b11 != 0) {
        return None;
    }
    temperature_of(&String::from_utf8_lossy(&out.stdout))
}

/// The current temperature in the JSON of `smartctl -j`.
pub fn temperature_of(json: &str) -> Option<f64> {
    let j: Value = serde_json::from_str(json).ok()?;
    j.pointer("/temperature/current")
        .and_then(|v| v.as_f64())
        .or_else(|| j.pointer("/nvme_smart_health_information_log/temperature").and_then(|v| v.as_f64()))
}

fn u(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

/// Parse the JSON of `smartctl -j -a`.
pub fn parse(json: &str, source: Source) -> Option<Report> {
    let j: Value = serde_json::from_str(json).ok()?;
    let nvme = j.get("nvme_smart_health_information_log");
    let table = j.pointer("/ata_smart_attributes/table").and_then(|t| t.as_array());
    if nvme.is_none() && table.is_none() && j.pointer("/smart_status/passed").is_none() {
        return None;
    }
    let kind = if nvme.is_some() {
        Kind::Nvme
    } else if j.get("rotation_rate").and_then(u) == Some(0) {
        Kind::Ssd
    } else {
        Kind::Hdd
    };
    let mut r = Report::new(kind, source);
    let s = |p: &str| j.pointer(p).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    r.model = s("/model_name");
    r.serial = s("/serial_number");
    r.firmware = s("/firmware_version");
    r.size = j.pointer("/user_capacity/bytes").and_then(u);
    r.failing = j.pointer("/smart_status/passed").and_then(|v| v.as_bool()) == Some(false);
    r.temp_c = j.pointer("/temperature/current").and_then(|v| v.as_f64());
    r.power_on_hours = j.pointer("/power_on_time/hours").and_then(u);
    r.power_cycles = j.get("power_cycle_count").and_then(u);
    let passed = j
        .pointer("/ata_smart_self_test_log/standard/table/0/status/passed")
        .or_else(|| j.pointer("/nvme_self_test_log/table/0/self_test_result/value").map(|v| if v.as_u64() == Some(0) { &Value::Bool(true) } else { &Value::Bool(false) }))
        .and_then(|v| v.as_bool());
    r.self_test_failed = passed.map(|p| !p);
    if let Some(n) = nvme {
        let g = |k: &str| n.get(k).and_then(u);
        r.nvme_critical = g("critical_warning").map(|v| v as u32);
        r.spare = g("available_spare").map(|v| v as f64);
        r.spare_thresh = g("available_spare_threshold").map(|v| v as f64);
        r.wear_used = g("percentage_used").map(|v| v as f64);
        r.written_bytes = g("data_units_written").map(|v| v.saturating_mul(512_000));
        r.power_cycles = r.power_cycles.or(g("power_cycles"));
        r.power_on_hours = r.power_on_hours.or(g("power_on_hours"));
        r.unsafe_shutdowns = g("unsafe_shutdowns");
        r.media_errors = g("media_errors");
        r.temp_c = r.temp_c.or(g("temperature").map(|v| v as f64));
        if let Some(o) = n.as_object() {
            for (k, v) in o {
                r.attrs.push(Attr { id: 0, name: k.clone(), raw: v.to_string(), ..Default::default() });
            }
        }
    }
    for a in table.into_iter().flatten() {
        let g = |k: &str| a.get(k).and_then(|v| v.as_i64());
        let raw = a.pointer("/raw/string").and_then(|v| v.as_str()).map(str::to_string).or_else(|| a.pointer("/raw/value").map(|v| v.to_string())).unwrap_or_default();
        let failing = a.get("when_failed").and_then(|v| v.as_str()).is_some_and(|w| w == "now");
        r.attrs.push(Attr {
            id: a.get("id").and_then(u).unwrap_or(0) as u32,
            name: a.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            value: g("value"),
            worst: g("worst"),
            thresh: g("thresh"),
            raw,
            failing,
        });
    }
    r.fill_from_ata_attrs();
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart::{evaluate, Item, Level};

    pub const NVME: &str = r#"{
      "device": {"name": "/dev/nvme0", "type": "nvme"},
      "model_name": "WD Blue SN570 1TB", "serial_number": "22101X800123", "firmware_version": "234110WD",
      "user_capacity": {"bytes": 1000204886016},
      "smart_status": {"passed": false, "nvme": {"value": 4}},
      "nvme_smart_health_information_log": {
        "critical_warning": 4, "temperature": 44, "available_spare": 100, "available_spare_threshold": 10,
        "percentage_used": 12, "data_units_written": 41234567, "power_cycles": 812, "power_on_hours": 5321,
        "unsafe_shutdowns": 37, "media_errors": 0
      },
      "temperature": {"current": 44}, "power_cycle_count": 812, "power_on_time": {"hours": 5321}
    }"#;

    const HDD: &str = r#"{
      "model_name": "WDC WD40EFRX-68N32N0", "serial_number": "WD-WCC7K1234567", "firmware_version": "82.00A82",
      "rotation_rate": 5400, "smart_status": {"passed": true},
      "ata_smart_attributes": {"table": [
        {"id": 5, "name": "Reallocated_Sector_Ct", "value": 199, "worst": 199, "thresh": 140, "when_failed": "", "raw": {"value": 12, "string": "12"}},
        {"id": 9, "name": "Power_On_Hours", "value": 52, "worst": 52, "thresh": 0, "raw": {"value": 35210, "string": "35210"}},
        {"id": 194, "name": "Temperature_Celsius", "value": 114, "worst": 100, "thresh": 0, "raw": {"value": 38, "string": "38"}},
        {"id": 197, "name": "Current_Pending_Sector", "value": 200, "worst": 200, "thresh": 0, "raw": {"value": 0, "string": "0"}},
        {"id": 199, "name": "UDMA_CRC_Error_Count", "value": 200, "worst": 200, "thresh": 0, "raw": {"value": 3, "string": "3"}}
      ]},
      "ata_smart_self_test_log": {"standard": {"table": [{"status": {"value": 0, "string": "Completed without error", "passed": true}}]}},
      "temperature": {"current": 38}, "power_cycle_count": 140, "power_on_time": {"hours": 35210}
    }"#;

    #[test]
    fn nvme_from_smartctl() {
        let r = parse(NVME, Source::Smartctl).unwrap();
        assert_eq!(r.kind, Kind::Nvme);
        assert_eq!(r.nvme_critical, Some(4));
        assert_eq!(r.written_bytes, Some(41234567 * 512_000));
        assert_eq!(r.unsafe_shutdowns, Some(37));
        let e = evaluate(&r, None);
        assert_eq!(e.level, Level::Crit);
        assert_eq!(e.level_of(Item::NvmeWarning), Level::Crit);
    }

    #[test]
    fn hdd_from_smartctl() {
        let r = parse(HDD, Source::Smartctl).unwrap();
        assert_eq!(r.kind, Kind::Hdd);
        assert_eq!((r.reallocated, r.pending, r.crc_errors), (Some(12), Some(0), Some(3)));
        assert_eq!(r.self_test_failed, Some(false));
        let e = evaluate(&r, None);
        assert_eq!((e.level, e.reason), (Level::Warn, Some(Item::Reallocated)));
        assert_eq!(e.level_of(Item::Crc), Level::Warn);
        assert!(parse("{}", Source::Smartctl).is_none());
        assert!(parse("not json", Source::Smartctl).is_none());
    }
}
