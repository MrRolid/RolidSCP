//! File system type on a server, from a shell command over SSH.

/// Shell command printing the file system type of `quoted_path` (already
/// quoted for `sh`): `df -PT` (GNU, BusyBox), else `stat -f` (only the magic
/// name, e.g. `ext2/ext3` for ext4).
pub fn fs_type_command(quoted_path: &str) -> String {
    format!("LC_ALL=C df -PT -- {quoted_path} 2>/dev/null || LC_ALL=C stat -f -c 'stat %T' -- {quoted_path}")
}

/// The type from the output of [`fs_type_command`].
pub fn parse_fs_type(out: &str) -> Option<String> {
    let mut lines = out.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next()?;
    if let Some(t) = first.strip_prefix("stat ") {
        let t = t.trim();
        return (!t.is_empty() && t != "UNKNOWN" && !t.starts_with("UNKNOWN")).then(|| t.to_string());
    }
    // df -PT: a header, then "source type blocks used available capacity mount".
    if !first.starts_with("Filesystem") {
        return None;
    }
    let data = lines.next()?;
    let cols: Vec<&str> = data.split_whitespace().collect();
    // The source may contain spaces: the type is the 6th column from the end.
    (cols.len() >= 7).then(|| cols[cols.len() - 6].to_string()).filter(|t| t.chars().all(|c| c.is_ascii_graphic()) && t.parse::<u64>().is_err())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_and_stat_output() {
        let gnu = "Filesystem     Type 1024-blocks      Used Available Capacity Mounted on\n/dev/sda1      ext4    10218772   5418340   4259860      56% /\n";
        assert_eq!(parse_fs_type(gnu).as_deref(), Some("ext4"));
        let space = "Filesystem Type 1024-blocks Used Available Capacity Mounted on\nmy share xfs 100 50 50 50% /mnt/x\n";
        assert_eq!(parse_fs_type(space).as_deref(), Some("xfs"));
        let nfs = "Filesystem Type 1024-blocks Used Available Capacity Mounted on\nnas:/export nfs4 100 50 50 50% /mnt/nas\n";
        assert_eq!(parse_fs_type(nfs).as_deref(), Some("nfs4"));
        // df without -T support printed nothing; stat answered.
        assert_eq!(parse_fs_type("stat btrfs\n").as_deref(), Some("btrfs"));
        assert_eq!(parse_fs_type("stat UNKNOWN (0x1234)\n"), None);
        assert_eq!(parse_fs_type(""), None);
        // Plain df without the type column.
        let plain = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100 50 50 50% /\n";
        assert_eq!(parse_fs_type(plain), None);
        assert!(fs_type_command("'/a b'").contains("df -PT -- '/a b'"));
    }
}
