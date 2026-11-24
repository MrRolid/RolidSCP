//! Fuzzing of what the disk code reads: `/proc/self/mountinfo`, the JSON of
//! `smartctl -j -a` (with the evaluation after it) and the output of `df -PT`
//! / `stat -f` from a server. Damaged input must give nothing or a partial
//! result, never a panic (also no arithmetic overflow: the long run uses
//! overflow checks).
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

use rolid_disk::smart::{evaluate, raw_number, Source};
use rolid_disk::{mounts, remote, smartctl};

const MOUNTINFO: &str = "22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw,errors=remount-ro
23 22 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
41 22 0:38 / /mnt/with\\040space rw,relatime shared:20 - nfs4 server:/export\\011tab rw,vers=4.2
50 22 253:0 /sub /home rw,relatime - btrfs /dev/mapper/vg-home rw,subvol=/sub
";

const NVME: &str = r#"{
  "device": {"name": "/dev/nvme0", "type": "nvme"},
  "model_name": "Example NVMe 1TB", "serial_number": "X1", "firmware_version": "1",
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
  "model_name": "Example HDD 4TB", "serial_number": "X2", "firmware_version": "2",
  "rotation_rate": 5400, "smart_status": {"passed": true},
  "ata_smart_attributes": {"table": [
    {"id": 5, "name": "Reallocated_Sector_Ct", "value": 199, "worst": 199, "thresh": 140, "when_failed": "", "raw": {"value": 12, "string": "12"}},
    {"id": 9, "name": "Power_On_Hours", "value": 52, "worst": 52, "thresh": 0, "raw": {"value": 35210, "string": "35210 (12 34 0)"}},
    {"id": 194, "name": "Temperature_Celsius", "value": 114, "worst": 100, "thresh": 0, "raw": {"value": 38, "string": "38 (Min/Max 20/45)"}},
    {"id": 197, "name": "Current_Pending_Sector", "value": 200, "worst": 200, "thresh": 0, "raw": {"value": 0, "string": "0"}},
    {"id": 241, "name": "Total_LBAs_Written", "value": 100, "worst": 100, "thresh": 0, "raw": {"value": 123456789, "string": "123456789"}}
  ]},
  "ata_smart_self_test_log": {"standard": {"table": [{"status": {"value": 0, "string": "Completed without error", "passed": true}}]}},
  "temperature": {"current": 38}, "power_cycle_count": 140, "power_on_time": {"hours": 35210}
}"#;

const DF: &[&str] = &[
    "Filesystem     Type 1024-blocks      Used Available Capacity Mounted on\n/dev/sda1      ext4   153250588 140000000   5000000      97% /\n",
    "Filesystem           Type       1K-blocks      Used Available Use% Mounted on\nserver:/a b    nfs4  1000 10 990 1% /mnt/a b\n",
    "stat ext2/ext3\n",
    "stat UNKNOWN (0x794c7630)\n",
];

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn fuzz_mountinfo() {
    let seeds = vec![MOUNTINFO.as_bytes().to_vec()];
    let check = |b: &[u8]| {
        let m = mounts::parse(&text(b));
        let _ = mounts::find(&m, "/mnt/with space/x");
    };
    mutate::run("mountinfo", &seeds, &[b" - ", b"\\040", b"\\", b":", b"\n"], check);
}

#[test]
fn fuzz_smartctl_json() {
    let seeds = vec![NVME.as_bytes().to_vec(), HDD.as_bytes().to_vec()];
    let check = |b: &[u8]| {
        if let Some(r) = smartctl::parse(&text(b), Source::Smartctl) {
            let e = evaluate(&r, None);
            let _ = evaluate(&r, Some(r.sectors()));
            let _ = e.level;
        }
    };
    let dict: &[&[u8]] = &[b"18446744073709551615", b"-1", b"1e308", b"null", b"[]", b"{}", b"\"raw\"", b"\"value\"", b"\"string\""];
    mutate::run("smartctl", &seeds, dict, check);
}

#[test]
fn fuzz_smart_raw_numbers() {
    let seeds: Vec<Vec<u8>> = ["35210 (12 34 0)", "38 (Min/Max 20/45)", "0x0000001a2b3c", "123456789"].iter().map(|s| s.as_bytes().to_vec()).collect();
    mutate::run("smart-raw", &seeds, &[b"0x", b"(", b")", b"/"], |b| {
        let _ = raw_number(&text(b));
    });
}

#[test]
fn fuzz_df_output() {
    let seeds: Vec<Vec<u8>> = DF.iter().map(|s| s.as_bytes().to_vec()).collect();
    mutate::run("df", &seeds, &[b"Filesystem", b"stat ", b" ", b"\n"], |b| {
        let _ = remote::parse_fs_type(&text(b));
    });
}
