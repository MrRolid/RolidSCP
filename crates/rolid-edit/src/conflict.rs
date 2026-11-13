//! Conflict check: has the remote file changed since it was downloaded (or last
//! uploaded)? Compared by size and modification time (whole seconds – SFTP).

use rolid_core::Entry;
use rolid_text::{txt, Text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteStamp {
    pub size: u64,
    /// Seconds since 1970, if the server reports it.
    pub mtime: Option<u64>,
}

impl From<&Entry> for RemoteStamp {
    fn from(e: &Entry) -> Self {
        RemoteStamp {
            size: e.size,
            mtime: e
                .modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        }
    }
}

impl RemoteStamp {
    /// True if `now` differs from the remembered state (size, or a known mtime).
    pub fn changed(&self, now: &RemoteStamp) -> bool {
        if self.size != now.size {
            return true;
        }
        match (self.mtime, now.mtime) {
            (Some(a), Some(b)) => a != b,
            // An mtime appeared or disappeared: be careful, report a change.
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        }
    }
}

/// Name for "save next to it": `name (my edit 2026-11-07 0731).ext` (the UI
/// translates it into the file name).
pub fn aside_name(name: &str, stamp: &str) -> Text {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    txt!("{stem} (my edit {stamp}){ext}", stem = stem, stamp = stamp, ext = ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(size: u64, mtime: Option<u64>) -> RemoteStamp {
        RemoteStamp { size, mtime }
    }

    #[test]
    fn detects_remote_changes() {
        let base = s(100, Some(1_700_000_000));
        assert!(!base.changed(&s(100, Some(1_700_000_000))), "unchanged");
        assert!(base.changed(&s(101, Some(1_700_000_000))), "size");
        assert!(base.changed(&s(100, Some(1_700_000_001))), "mtime, same size");
        assert!(base.changed(&s(100, None)), "mtime disappeared");
        assert!(!s(5, None).changed(&s(5, None)), "no mtime at all: size only");
        assert!(s(5, None).changed(&s(6, None)));
    }

    #[test]
    fn stamp_from_entry_uses_whole_seconds() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_750);
        let e = Entry {
            name: "a".into(),
            kind: rolid_core::vfs::EntryKind::File,
            link_to_dir: false,
            size: 3,
            modified: Some(t),
            mode: None,
        };
        assert_eq!(RemoteStamp::from(&e), s(3, Some(1_700_000_000)));
    }

    #[test]
    fn aside_names() {
        let a = |n, s| aside_name(n, s).to_string();
        assert_eq!(a("index.php", "2026-11-07 0731"), "index (my edit 2026-11-07 0731).php");
        assert_eq!(a("Makefile", "x"), "Makefile (my edit x)");
        assert_eq!(a(".bashrc", "x"), ".bashrc (my edit x)");
        assert_eq!(a("a.tar.gz", "x"), "a.tar (my edit x).gz");
    }
}
