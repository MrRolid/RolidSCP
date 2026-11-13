//! Which mount a local path is on (`/proc/self/mountinfo`): the file system
//! type and the device behind it.

/// One line of `/proc/self/mountinfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Where it is mounted (`/`, `/home`, `/media/usb`).
    pub point: String,
    /// File system type (`ext4`, `vfat`, `fuse.sshfs`…).
    pub fstype: String,
    /// What is mounted (`/dev/nvme0n1p2`, `/dev/mapper/vg-root`, `server:/export`).
    pub source: String,
    /// Device number (major, minor) the kernel reports for the mount.
    pub dev: (u32, u32),
}

/// `\040` and other octal escapes in mountinfo paths.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the text of `/proc/self/mountinfo` (lines that do not fit are skipped).
pub fn parse(text: &str) -> Vec<Mount> {
    let mut out = Vec::new();
    for line in text.lines() {
        // id parent major:minor root point options [optional…] - fstype source superoptions
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 5 || r.len() < 2 {
            continue;
        }
        let Some((ma, mi)) = l[2].split_once(':') else { continue };
        let (Ok(ma), Ok(mi)) = (ma.parse(), mi.parse()) else { continue };
        out.push(Mount { point: unescape(l[4]), fstype: unescape(r[0]), source: unescape(r[1]), dev: (ma, mi) });
    }
    out
}

/// The mount `path` (absolute, without `..`) is on: the longest mount point
/// that contains it; of equal ones the last (a later mount hides an earlier one).
pub fn find<'a>(mounts: &'a [Mount], path: &str) -> Option<&'a Mount> {
    let inside = |p: &str| p == "/" || path == p || path.strip_prefix(p).is_some_and(|rest| rest.starts_with('/'));
    let mut best: Option<&Mount> = None;
    for m in mounts {
        if inside(&m.point) && best.is_none_or(|b| m.point.len() >= b.point.len()) {
            best = Some(m);
        }
    }
    best
}

/// The mount of a local path (symbolic links resolved). Blocking: reads `/proc`.
pub fn mount_of(path: &str) -> Option<Mount> {
    let real = std::fs::canonicalize(path).ok()?;
    let text = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mounts = parse(&text);
    find(&mounts, &real.to_string_lossy()).cloned()
}

/// Is this a network or virtual file system (no physical disk of this computer)?
pub fn is_network(fstype: &str) -> bool {
    let t = fstype.to_ascii_lowercase();
    matches!(t.as_str(), "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "afs" | "9p" | "ceph" | "glusterfs" | "davfs" | "virtiofs" | "sshfs")
        || t.starts_with("fuse.sshfs")
        || t.starts_with("fuse.rclone")
        || t.starts_with("fuse.gvfs")
        || t == "fuse.davfs"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
23 22 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
24 22 259:1 / /boot/efi rw,relatime shared:2 - vfat /dev/nvme0n1p1 rw,fmask=0077
30 22 253:1 / /home rw,relatime shared:3 - ext4 /dev/mapper/vg-home rw
31 30 0:45 / /home/m/Ext\\040disk rw,nosuid,nodev,relatime shared:40 - exfat /dev/sdb1 rw,uid=1000
32 22 0:50 /export /mnt/nas rw,relatime shared:41 - nfs4 nas.local:/export rw,vers=4.2
33 30 0:60 / /home/m/server rw,nosuid,nodev,relatime shared:42 - fuse.sshfs m@server:/ rw,user_id=1000
34 22 0:33 /@data /data rw,relatime shared:43 - btrfs /dev/sda2 rw,space_cache=v2,subvol=/@data
35 22 8:2 / /data rw,relatime shared:44 - xfs /dev/sdc1 rw
garbage line
";

    #[test]
    fn file_system_type_from_mountinfo() {
        let m = parse(SAMPLE);
        assert_eq!(m.len(), 9);
        let t = |p: &str| find(&m, p).map(|x| x.fstype.clone());
        assert_eq!(t("/etc/fstab").as_deref(), Some("ext4"));
        assert_eq!(t("/").as_deref(), Some("ext4"));
        assert_eq!(t("/boot/efi/EFI").as_deref(), Some("vfat"));
        assert_eq!(t("/boot").as_deref(), Some("ext4"));
        assert_eq!(t("/home/m/Ext disk/photos").as_deref(), Some("exfat"));
        assert_eq!(t("/home/m/Ext").as_deref(), Some("ext4"));
        assert_eq!(t("/mnt/nas/a").as_deref(), Some("nfs4"));
        assert_eq!(t("/home/m/server/etc").as_deref(), Some("fuse.sshfs"));
        // The later mount on the same point hides the earlier one.
        assert_eq!(t("/data/x").as_deref(), Some("xfs"));
        // "/homework" is not inside "/home".
        assert_eq!(t("/homework").as_deref(), Some("ext4"));
        let h = find(&m, "/home/m").unwrap();
        assert_eq!(h.source, "/dev/mapper/vg-home");
        assert_eq!(h.dev, (253, 1));
        assert!(is_network("nfs4") && is_network("fuse.sshfs") && is_network("cifs"));
        assert!(!is_network("ext4") && !is_network("exfat"));
    }

    #[test]
    fn this_computer_has_a_root_mount() {
        if std::path::Path::new("/proc/self/mountinfo").exists() {
            assert!(mount_of("/").is_some_and(|m| m.point == "/"));
        }
    }
}
