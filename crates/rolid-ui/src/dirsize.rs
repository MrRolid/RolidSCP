//! Size of folders ("Count folder sizes").
//!
//! Locally and on a server with `du` the size is counted like GNU `du -sb -l`:
//! apparent sizes of the files and of the links themselves (folders count 0),
//! links are not followed, hard links count every time. Over SFTP without
//! `du` and inside an archive it is the sum of the file sizes. Unreadable
//! folders are left out and counted.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rolid_core::{EntryKind, Vfs};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    /// Folders (or items) that could not be read: the size is "at least".
    pub unreadable: u64,
}

impl DirSize {
    /// "12,4 MB" or "≥ 12,4 MB".
    pub fn text(&self) -> String {
        let s = crate::pane::human_size(self.bytes);
        if self.unreadable > 0 {
            format!("≥ {s}")
        } else {
            s
        }
    }

    /// Tooltip for an incomplete result.
    pub fn note(&self) -> Option<String> {
        (self.unreadable > 0).then(|| {
            trn!(
                "At least this much: {n} item could not be read (no permission), its content is not counted.",
                "At least this much: {n} items could not be read (no permission), their content is not counted.",
                self.unreadable
            )
        })
    }
}

/// Local folder, like `du -sb -l` (no links followed). `on_bytes` gets the
/// running total now and then.
pub fn local(path: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Option<DirSize> {
    let mut r = DirSize::default();
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => {
            r.unreadable = 1;
            return Some(r);
        }
    };
    if !meta.is_dir() {
        r.bytes = meta.len();
        return Some(r);
    }
    let mut stack = vec![path.to_path_buf()];
    let mut n = 0u32;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                r.unreadable += 1;
                continue;
            }
        };
        for e in rd {
            let Ok(e) = e else {
                r.unreadable += 1;
                continue;
            };
            let Ok(m) = std::fs::symlink_metadata(e.path()) else {
                r.unreadable += 1;
                continue;
            };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                r.bytes += m.len();
            }
            n += 1;
            if n.is_multiple_of(4096) {
                on_bytes(r.bytes);
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
            }
        }
    }
    Some(r)
}

/// Any file system (SFTP, archive): sum of the file sizes; links not followed.
pub async fn walk(fs: &dyn Vfs, path: &str, cancel: &AtomicBool, on_bytes: &mut (dyn FnMut(u64) + Send)) -> Option<DirSize> {
    let mut r = DirSize::default();
    let mut stack = vec![path.to_string()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let list = match fs.list(&dir).await {
            Ok(l) => l,
            Err(e) if e.is_connection_lost() => return None,
            Err(_) => {
                r.unreadable += 1;
                continue;
            }
        };
        for e in list {
            match e.kind {
                EntryKind::Dir => stack.push(fs.join(&dir, &e.name)),
                EntryKind::File => r.bytes += e.size,
                _ => {}
            }
        }
        on_bytes(r.bytes);
    }
    Some(r)
}

/// Shell command for the size of `path` on a server: `du` in bytes, links
/// not followed, hard links counted every time; the path safely quoted.
pub fn du_command(path: &str) -> String {
    format!("LC_ALL=C du -s -b -l -P -- {}", rolid_search::remote::sh_quote(path))
}

/// Parse `du` output ("12345\t/path") and its stderr; `None` = not GNU du's answer.
pub fn parse_du(stdout: &[u8], stderr: &str) -> Option<DirSize> {
    let s = String::from_utf8_lossy(stdout);
    let first = s.split(['\t', ' ']).next()?.trim();
    let bytes = first.parse::<u64>().ok()?;
    let unreadable = stderr.lines().filter(|l| l.starts_with("du:")).count() as u64;
    Some(DirSize { bytes, unreadable })
}

/// Size over SSH with `du` (killed when `cancel` is set); `Err` = no usable `du`.
pub async fn remote_du(conn: &rolid_ssh::SshConnection, path: &str, cancel: &AtomicBool) -> Result<Option<DirSize>, String> {
    let cmd = rolid_search::remote::guarded(&du_command(path));
    let mut out = Vec::new();
    let (_status, err) = conn
        .exec_stream(&cmd, cancel, &mut |d| {
            out.extend_from_slice(d);
            out.len() < 1 << 16
        })
        .await
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    parse_du(&out, &err).map(Some).ok_or_else(|| format!("du does not work: {}", err.lines().next().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn local_size_with_a_link_and_an_unreadable_folder() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.bin"), vec![1u8; 1000]).unwrap();
        std::fs::write(d.join("sub/b.bin"), vec![1u8; 500]).unwrap();
        // A link to a big file outside: only the link itself counts.
        let big = t.path().join("big");
        std::fs::write(&big, vec![0u8; 1_000_000]).unwrap();
        symlink(&big, d.join("link")).unwrap();
        let no = AtomicBool::new(false);
        let r = local(&d, &no, &mut |_| {}).unwrap();
        // Compare with du itself.
        let du = std::process::Command::new("sh").arg("-c").arg(du_command(d.to_str().unwrap())).output().unwrap();
        assert_eq!(Some(r), parse_du(&du.stdout, &String::from_utf8_lossy(&du.stderr)));
        assert!(r.bytes >= 1500 && r.bytes < 1_000_000, "{r:?}");
        assert_eq!(r.unreadable, 0);
        assert_eq!(r.text(), crate::pane::human_size(r.bytes));
        // An unreadable folder: left out, counted, "≥".
        std::fs::create_dir(d.join("secret")).unwrap();
        std::fs::write(d.join("secret/x"), vec![1u8; 5000]).unwrap();
        std::fs::set_permissions(d.join("secret"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let r2 = local(&d, &no, &mut |_| {}).unwrap();
        let du = std::process::Command::new("sh").arg("-c").arg(du_command(d.to_str().unwrap())).output().unwrap();
        let by_du = parse_du(&du.stdout, &String::from_utf8_lossy(&du.stderr)).unwrap();
        std::fs::set_permissions(d.join("secret"), std::fs::Permissions::from_mode(0o755)).unwrap();
        if r2.unreadable == 0 {
            eprintln!("running as root: an unreadable folder cannot be tested");
            return;
        }
        assert_eq!(r2.unreadable, 1);
        assert_eq!(r2, by_du);
        assert!(r2.text().starts_with("≥ "));
        assert!(r2.note().unwrap().contains("1 item could not"));
        // Cancel.
        let yes = AtomicBool::new(true);
        assert_eq!(local(&d, &yes, &mut |_| {}), None);
    }

    #[test]
    fn du_quoting() {
        let t = tempfile::tempdir().unwrap();
        for name in ["with space", "quo'tes\"a`$(echo x)", "new\nline", "žluťoučký kůň", "-starts-with-dash", "*"] {
            let d = t.path().join(name);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("f"), vec![1u8; 777]).unwrap();
            let cmd = du_command(d.to_str().unwrap());
            let out = std::process::Command::new("sh").arg("-c").arg(&cmd).output().unwrap();
            assert!(out.status.success(), "{name:?}: {}", String::from_utf8_lossy(&out.stderr));
            let r = parse_du(&out.stdout, &String::from_utf8_lossy(&out.stderr)).unwrap();
            let no = AtomicBool::new(false);
            assert_eq!(Some(r), local(&d, &no, &mut |_| {}), "{name:?}");
        }
        assert_eq!(parse_du(b"", "du: invalid option -- 'b'"), None);
        assert_eq!(parse_du(b"4096\t/x\n", "du: cannot read directory '/x/a': Permission denied\n"), Some(DirSize { bytes: 4096, unreadable: 1 }));
    }
}
