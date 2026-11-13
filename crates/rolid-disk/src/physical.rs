//! From a mount to the physical disks behind it: a partition to its disk,
//! LVM, LUKS and RAID (device mapper, md) to the disks they are made of.

use std::path::{Path, PathBuf};

use crate::mounts::Mount;

/// The block device tree (`/sys/class/block`), replaceable in tests.
pub trait Sys {
    /// Kernel name of the block device a mount is on (`nvme0n1p2`, `dm-1`).
    fn device(&self, m: &Mount) -> Option<String>;
    /// The whole disk of a partition (`sda1` → `sda`), `None` for a whole device.
    fn parent(&self, name: &str) -> Option<String>;
    /// Devices this one is made of (device mapper, md RAID).
    fn slaves(&self, name: &str) -> Vec<String>;
}

/// Why a disk has no SMART data to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSmart {
    /// A network or FUSE file system.
    Network,
    /// A virtual disk (virtual machine, loop file, RAM disk).
    Virtual,
    /// A memory-only file system (tmpfs, proc…), no disk at all.
    NoDisk,
    /// The disk did not give SMART data (USB bridge, card reader, no access).
    NotOffered,
    /// A folder on a server: its disks are not read.
    Server,
}

/// The physical disks of a mount (sorted, without repeats), or why there are none.
pub fn disks_of(sys: &dyn Sys, m: &Mount) -> Result<Vec<String>, NoSmart> {
    if crate::mounts::is_network(&m.fstype) || m.fstype.starts_with("fuse") {
        return Err(NoSmart::Network);
    }
    let Some(dev) = sys.device(m) else {
        return Err(NoSmart::NoDisk);
    };
    let mut out = Vec::new();
    walk(sys, &dev, 0, &mut out);
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err(NoSmart::NoDisk);
    }
    Ok(out)
}

fn walk(sys: &dyn Sys, name: &str, depth: u32, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    let slaves = sys.slaves(name);
    if !slaves.is_empty() {
        for s in slaves {
            walk(sys, &s, depth + 1, out);
        }
        return;
    }
    match sys.parent(name) {
        Some(p) => walk(sys, &p, depth + 1, out),
        None => out.push(name.to_string()),
    }
}

/// A disk that cannot have real SMART data: virtual, loop, RAM.
pub fn is_virtual(name: &str, vendor: &str, model: &str, path: &str) -> bool {
    let virtual_name = ["loop", "ram", "zram", "nbd", "vd", "xvd", "sr", "md", "dm-"].iter().any(|p| name.starts_with(p));
    let v = format!("{vendor} {model}").to_ascii_uppercase();
    let virtual_hw = ["QEMU", "VBOX", "VIRTUAL", "VMWARE", "MSFT", "XEN", "KVM", "BOCHS"].iter().any(|k| v.contains(k));
    virtual_name || virtual_hw || (path.contains("/virtio") && !name.starts_with("nvme"))
}

/// The real tree in `/sys`.
pub struct RealSys {
    root: PathBuf,
}

impl Default for RealSys {
    fn default() -> Self {
        RealSys { root: PathBuf::from("/sys") }
    }
}

impl RealSys {
    fn class(&self, name: &str) -> PathBuf {
        self.root.join("class/block").join(name)
    }

    fn read(&self, p: &Path) -> String {
        std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
    }

    /// Vendor, model and the device's place in the hardware tree.
    pub fn facts(&self, disk: &str) -> (String, String, String) {
        let b = self.root.join("block").join(disk);
        let path = std::fs::canonicalize(&b).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        (self.read(&b.join("device/vendor")), self.read(&b.join("device/model")), path)
    }
}

impl Sys for RealSys {
    fn device(&self, m: &Mount) -> Option<String> {
        if m.source.starts_with("/dev/") {
            if let Ok(p) = std::fs::canonicalize(&m.source) {
                if let Some(n) = p.file_name().map(|n| n.to_string_lossy().into_owned()) {
                    if self.class(&n).exists() {
                        return Some(n);
                    }
                }
            }
        }
        let link = self.root.join(format!("dev/block/{}:{}", m.dev.0, m.dev.1));
        let p = std::fs::canonicalize(link).ok()?;
        let n = p.file_name()?.to_string_lossy().into_owned();
        self.class(&n).exists().then_some(n)
    }

    fn parent(&self, name: &str) -> Option<String> {
        let c = self.class(name);
        if !c.join("partition").exists() {
            return None;
        }
        let p = std::fs::canonicalize(&c).ok()?;
        Some(p.parent()?.file_name()?.to_string_lossy().into_owned())
    }

    fn slaves(&self, name: &str) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(self.class(name).join("slaves")) else { return Vec::new() };
        rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeSys {
        dev: HashMap<&'static str, &'static str>,
        parent: HashMap<&'static str, &'static str>,
        slaves: HashMap<&'static str, Vec<&'static str>>,
    }

    impl Sys for FakeSys {
        fn device(&self, m: &Mount) -> Option<String> {
            self.dev.get(m.source.as_str()).map(|s| s.to_string())
        }
        fn parent(&self, name: &str) -> Option<String> {
            self.parent.get(name).map(|s| s.to_string())
        }
        fn slaves(&self, name: &str) -> Vec<String> {
            self.slaves.get(name).map(|v| v.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
        }
    }

    fn mount(source: &str, fstype: &str) -> Mount {
        Mount { point: "/x".into(), fstype: fstype.into(), source: source.into(), dev: (0, 0) }
    }

    #[test]
    fn path_to_physical_disk() {
        let sys = FakeSys {
            dev: HashMap::from([
                ("/dev/nvme0n1p2", "nvme0n1p2"),
                // LVM on LUKS on a partition.
                ("/dev/mapper/vg-home", "dm-2"),
                // ext4 on md RAID 1 of two partitions.
                ("/dev/md0", "md0"),
                // A whole disk without partitions.
                ("/dev/sdc", "sdc"),
            ]),
            parent: HashMap::from([("nvme0n1p2", "nvme0n1"), ("sda2", "sda"), ("sda1", "sda"), ("sdb1", "sdb")]),
            slaves: HashMap::from([("dm-2", vec!["dm-0"]), ("dm-0", vec!["sda2"]), ("md0", vec!["sdb1", "sda1"])]),
        };
        assert_eq!(disks_of(&sys, &mount("/dev/nvme0n1p2", "ext4")), Ok(vec!["nvme0n1".to_string()]));
        assert_eq!(disks_of(&sys, &mount("/dev/mapper/vg-home", "xfs")), Ok(vec!["sda".to_string()]));
        assert_eq!(disks_of(&sys, &mount("/dev/md0", "ext4")), Ok(vec!["sda".to_string(), "sdb".to_string()]));
        assert_eq!(disks_of(&sys, &mount("/dev/sdc", "exfat")), Ok(vec!["sdc".to_string()]));
        assert_eq!(disks_of(&sys, &mount("nas:/export", "nfs4")), Err(NoSmart::Network));
        assert_eq!(disks_of(&sys, &mount("m@server:/", "fuse.sshfs")), Err(NoSmart::Network));
        assert_eq!(disks_of(&sys, &mount("tmpfs", "tmpfs")), Err(NoSmart::NoDisk));
    }

    #[test]
    fn virtual_disks() {
        assert!(is_virtual("sda", "QEMU", "QEMU HARDDISK", "/sys/devices/pci0000:00/virtio2/host0/block/sda"));
        assert!(is_virtual("vda", "", "", ""));
        assert!(is_virtual("loop3", "", "", ""));
        assert!(is_virtual("sdb", "VMware", "Virtual disk", ""));
        assert!(!is_virtual("sda", "ATA", "Samsung SSD 870", "/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/block/sda"));
        assert!(!is_virtual("nvme0n1", "", "WD Blue SN570", "/sys/devices/pci0000:00/0000:00:1d.0/nvme/nvme0/nvme0n1"));
    }
}
