//! What Enter / double-click / "Open" does with an entry (pure rules).

use rolid_core::{Entry, EntryKind};

/// Where the entry lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// On this computer's disk.
    Local,
    /// On a server.
    Remote,
    /// Inside an archive opened as a folder.
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenAction {
    /// A folder (or a link to one): go in.
    EnterDir,
    /// An archive: open it as a read-only folder.
    OpenArchive,
    /// Open with the remembered / system default application (a server file
    /// through a temporary copy with its own checks, an archive item through a
    /// read-only copy).
    OpenDefault,
    /// A local program or script: ask Run / Open in editor / Cancel.
    /// `reason` is English marked with `n!` (translate with `t_str`).
    AskRun { reason: &'static str, executable: bool },
    /// A program or script inside an archive: only the text editor (read-only copy).
    EditorOnly,
    /// Device, socket…: nothing to open.
    Nothing,
}

/// Why a local file could run something when opened (`None` = harmless).
pub fn local_risk(name: &str, mode: Option<u32>, head: &[u8]) -> Option<&'static str> {
    rolid_edit::apps::risky(name, mode, head).map(|r| {
        if r.contains("on the server") {
            rolid_i18n::rolid_text::n!("has the execute permission")
        } else {
            r
        }
    })
}

/// The action for `e`; `head` = first bytes of a local file (scripts start with `#!`).
pub fn decide(e: &Entry, at: Where, head: &[u8]) -> OpenAction {
    if e.is_dir_like() {
        return OpenAction::EnterDir;
    }
    if !matches!(e.kind, EntryKind::File | EntryKind::Symlink) {
        return OpenAction::Nothing;
    }
    if rolid_archive::Format::from_name(&e.name).is_some() {
        return OpenAction::OpenArchive;
    }
    match at {
        // The server path has its own warning (Risky) after the download.
        Where::Remote => OpenAction::OpenDefault,
        Where::Local => match local_risk(&e.name, e.mode, head) {
            Some(reason) => OpenAction::AskRun { reason, executable: e.mode.is_some_and(|m| m & 0o111 != 0) },
            None => OpenAction::OpenDefault,
        },
        Where::Archive => match rolid_edit::apps::risky(&e.name, e.mode, head) {
            Some(_) => OpenAction::EditorOnly,
            None => OpenAction::OpenDefault,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, kind: EntryKind, mode: u32) -> Entry {
        Entry { name: name.into(), kind, link_to_dir: false, size: 10, modified: None, mode: Some(mode) }
    }

    #[test]
    fn enter_by_file_type() {
        let f = |n: &str, m: u32| e(n, EntryKind::File, m);
        // Ordinary file.
        assert_eq!(decide(&f("foto.jpg", 0o644), Where::Local, b"\xff\xd8"), OpenAction::OpenDefault);
        assert_eq!(decide(&f("text.txt", 0o644), Where::Remote, b""), OpenAction::OpenDefault);
        assert_eq!(decide(&f("text.txt", 0o644), Where::Archive, b""), OpenAction::OpenDefault);
        // Archive anywhere opens as a folder.
        for w in [Where::Local, Where::Remote, Where::Archive] {
            assert_eq!(decide(&f("data.tar.zst", 0o644), w, b""), OpenAction::OpenArchive);
            assert_eq!(decide(&f("x.ZIP", 0o644), w, b""), OpenAction::OpenArchive);
        }
        // Local executable or script asks.
        assert_eq!(
            decide(&f("prog", 0o755), Where::Local, b"\x7fELF"),
            OpenAction::AskRun { reason: "has the execute permission", executable: true }
        );
        assert!(matches!(decide(&f("s.txt", 0o644), Where::Local, b"#!/bin/sh"), OpenAction::AskRun { executable: false, .. }));
        assert!(matches!(decide(&f("run.sh", 0o644), Where::Local, b"echo"), OpenAction::AskRun { .. }));
        assert!(matches!(decide(&f("a.desktop", 0o644), Where::Local, b""), OpenAction::AskRun { .. }));
        // From a server: the download flow warns by itself.
        assert_eq!(decide(&f("run.sh", 0o755), Where::Remote, b""), OpenAction::OpenDefault);
        // Inside an archive: never run, only the editor.
        assert_eq!(decide(&f("run.sh", 0o755), Where::Archive, b""), OpenAction::EditorOnly);
        // Folders, links, specials.
        assert_eq!(decide(&e("dir", EntryKind::Dir, 0o755), Where::Local, b""), OpenAction::EnterDir);
        let mut l = e("link", EntryKind::Symlink, 0o777);
        l.link_to_dir = true;
        assert_eq!(decide(&l, Where::Remote, b""), OpenAction::EnterDir);
        assert_eq!(decide(&e("fifo", EntryKind::Other, 0o644), Where::Local, b""), OpenAction::Nothing);
    }
}
