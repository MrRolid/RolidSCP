//! Odd names and special files on the local disk (roadmap 0.8.15): names with
//! a backslash, a new line, a tab, a leading dash, 255 bytes, NFC/NFD twins
//! are copied with their exact bytes; a pipe, a socket, a device and links to
//! them are skipped and reported, and nothing waits for a pipe that nobody
//! writes into.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rolid_core::copy::{CopyJob, Progress};
use rolid_core::vfs::{EntryKind, Vfs};
use rolid_local::LocalFs;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-odd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn fifo(p: &Path) {
    rustix::fs::mknodat(rustix::fs::CWD, p, rustix::fs::FileType::Fifo, rustix::fs::Mode::from(0o644), 0).unwrap();
}

/// Names that are valid UTF-8 but unusual; each must survive a copy byte for byte.
fn odd_names() -> Vec<String> {
    vec![
        "back\\slash.txt".into(),
        "new\nline".into(),
        "tab\there".into(),
        "-leading-dash".into(),
        "--help".into(),
        "trailing space ".into(),
        "x".repeat(255),
        "日".repeat(85),
        "caf\u{e9}".into(),   // NFC
        "cafe\u{301}".into(), // NFD: a different name with the same look
        "CASE".into(),
        "case".into(),
    ]
}

/// A folder with odd names, a pipe, a socket and links to a pipe and a device.
fn tree(base: &Path) -> (PathBuf, std::os::unix::net::UnixListener) {
    let t = base.join("src/tree");
    std::fs::create_dir_all(&t).unwrap();
    for (i, n) in odd_names().iter().enumerate() {
        std::fs::write(t.join(n), format!("content {i}")).unwrap();
    }
    fifo(&t.join("pipe"));
    let sock = std::os::unix::net::UnixListener::bind(t.join("sock")).unwrap();
    std::os::unix::fs::symlink("pipe", t.join("link-to-pipe")).unwrap();
    std::os::unix::fs::symlink("/dev/null", t.join("link-to-null")).unwrap();
    std::os::unix::fs::symlink("missing", t.join("dangling")).unwrap();
    std::os::unix::fs::symlink("self", t.join("self")).unwrap();
    (t, sock)
}

/// Copy `name` from `dir` into `into`, as F5 does (stops at the first real error).
async fn copy(dir: &Path, name: &str, into: &Path) -> (Result<(), String>, Vec<(String, String)>, u64) {
    let skipped = Mutex::new(0);
    let cb = |p: &Progress| *skipped.lock().unwrap() = p.files_skipped;
    let mut job = CopyJob::new(&LocalFs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb);
    let items = vec![(s(dir), name.to_string())];
    let into = s(into);
    let run = job.copy_items_retrying(&items, &into, 0, |_| async { false });
    let r = tokio::time::timeout(Duration::from_secs(20), run).await.expect("the copy waits (for a pipe?)");
    let failures = job.failures().iter().map(|f| (f.path.clone(), f.error.to_string())).collect();
    let n = *skipped.lock().unwrap();
    (r.map_err(|e| e.text().to_string()), failures, n)
}

#[tokio::test(flavor = "multi_thread")]
async fn special_files_are_skipped_with_a_message_and_odd_names_are_kept() {
    let base = scratch("tree");
    let (src, _sock) = tree(&base);
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();

    let (r, failures, skipped) = copy(src.parent().unwrap(), "tree", &dst).await;
    assert_eq!(r, Ok(()));
    let mut failed: Vec<&str> = failures.iter().map(|(p, _)| p.rsplit('/').next().unwrap()).collect();
    failed.sort();
    assert_eq!(failed, ["dangling", "link-to-null", "link-to-pipe", "pipe", "self", "sock"]);
    for (p, e) in &failures {
        let link = p.ends_with("/dangling") || p.ends_with("/self");
        assert!(e.contains(if link { "leads nowhere" } else { "not a regular file" }), "{p}: {e}");
    }
    assert_eq!(skipped, 6);
    // Every odd name is there with its exact bytes and content, and nothing else.
    for (i, n) in odd_names().iter().enumerate() {
        assert_eq!(std::fs::read_to_string(dst.join("tree").join(n)).unwrap(), format!("content {i}"), "{n:?}");
    }
    let mut got: Vec<String> = std::fs::read_dir(dst.join("tree")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    let mut want = odd_names();
    got.sort();
    want.sort();
    assert_eq!(got, want);

    // The listing shows them as they are, the special ones as "other".
    let list = LocalFs.list(&s(&src)).await.unwrap();
    for n in odd_names() {
        assert!(list.iter().any(|e| e.name == n && e.kind == EntryKind::File), "{n:?}");
    }
    for n in ["pipe", "sock"] {
        assert!(list.iter().any(|e| e.name == n && e.kind == EntryKind::Other), "{n}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A pipe picked directly (or through a link) is not opened: no copy waits for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_pipe_picked_directly_does_not_hang_the_copy() {
    let base = scratch("direct");
    let (src, _sock) = tree(&base);
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    for name in ["pipe", "link-to-pipe", "sock", "link-to-null", "dangling", "self"] {
        let (r, failures, _) = copy(&src, name, &dst).await;
        assert_eq!(r, Ok(()), "{name}");
        assert_eq!(failures.len(), 1, "{name}");
        assert!(!dst.join(name).exists(), "{name}");
    }
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 0, "no part files left");
    // Reading it (the viewer, a checksum) is refused at once as well.
    for name in ["pipe", "link-to-pipe", "link-to-null"] {
        let p = s(&src.join(name));
        let read = tokio::time::timeout(Duration::from_secs(5), LocalFs.open_read_at(&p, 0)).await.expect("waits for a pipe");
        assert!(read.is_err_and(|e| e.is_not_regular()), "{name}");
        let random = tokio::time::timeout(Duration::from_secs(5), LocalFs.open_random_native(&p)).await.expect("waits for a pipe");
        assert!(random.is_err_and(|e| e.is_not_regular()), "{name}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A name that is not valid UTF-8 keeps its exact bytes on the local disk
/// (before 0.8.15 it became "�" and the file could not be found to copy).
#[tokio::test(flavor = "multi_thread")]
async fn a_name_that_is_not_utf8_keeps_its_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let base = scratch("bytes");
    let t = base.join("src/tree");
    std::fs::create_dir_all(&t).unwrap();
    let raw: &[&[u8]] = &[b"bad\xff.txt", b"bad\xfe.txt", b"half \xe6\x97"];
    for (i, n) in raw.iter().enumerate() {
        std::fs::write(t.join(std::ffi::OsStr::from_bytes(n)), format!("bytes {i}")).unwrap();
    }
    std::fs::create_dir(t.join(std::ffi::OsStr::from_bytes(b"dir\x80"))).unwrap();
    std::fs::write(t.join(std::ffi::OsStr::from_bytes(b"dir\x80/inner")), "inner").unwrap();

    // Listed as different names, each shown readably.
    let list = LocalFs.list(&s(&t)).await.unwrap();
    let mut shown: Vec<String> = list.iter().map(|e| rolid_core::names::visible(&e.name).into_owned()).collect();
    shown.sort();
    assert_eq!(shown, ["bad\\xFE.txt", "bad\\xFF.txt", "dir\\x80", "half \\xE6\\x97"]);

    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let (r, failures, _) = copy(t.parent().unwrap(), "tree", &dst).await;
    assert_eq!(r, Ok(()));
    assert!(failures.is_empty(), "{failures:?}");
    let mut got: Vec<Vec<u8>> = std::fs::read_dir(dst.join("tree")).unwrap().map(|e| e.unwrap().file_name().into_vec()).collect();
    got.sort();
    assert_eq!(got, [b"bad\xfe.txt".to_vec(), b"bad\xff.txt".to_vec(), b"dir\x80".to_vec(), b"half \xe6\x97".to_vec()]);
    for (i, n) in raw.iter().enumerate() {
        assert_eq!(std::fs::read_to_string(dst.join("tree").join(std::ffi::OsStr::from_bytes(n))).unwrap(), format!("bytes {i}"));
    }
    assert_eq!(std::fs::read_to_string(dst.join(std::ffi::OsStr::from_bytes(b"tree/dir\x80/inner"))).unwrap(), "inner");

    // Renamed and deleted by the name from the listing.
    let e = list.iter().find(|e| rolid_core::names::visible(&e.name) == "bad\\xFF.txt").unwrap();
    let from = LocalFs.join(&s(&t), &e.name);
    let to = LocalFs.join(&s(&t), &format!("{}.renamed", e.name));
    LocalFs.rename(&from, &to).await.unwrap();
    assert!(t.join(std::ffi::OsStr::from_bytes(b"bad\xff.txt.renamed")).exists());
    LocalFs.remove_file(&to).await.unwrap();
    assert!(!t.join(std::ffi::OsStr::from_bytes(b"bad\xff.txt.renamed")).exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// A big folder is listed with the details read on several threads: every
/// file once, with its size; many files are deleted in one call and a file
/// that is gone is reported at its place (roadmap 0.8.15).
#[tokio::test(flavor = "multi_thread")]
async fn a_big_folder_is_listed_and_deleted_in_batches() {
    let base = scratch("big");
    let n = 25_000;
    for i in 0..n {
        std::fs::write(base.join(format!("f{i:05}")), vec![b'x'; i % 7]).unwrap();
    }
    std::fs::create_dir(base.join("sub")).unwrap();
    let list = LocalFs.list(&s(&base)).await.unwrap();
    assert_eq!(list.len(), n + 1);
    let mut names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), n + 1);
    for e in &list {
        match e.name.strip_prefix('f') {
            Some(i) => assert_eq!(e.size as usize, i.parse::<usize>().unwrap() % 7, "{}", e.name),
            None => assert_eq!(e.kind, EntryKind::Dir),
        }
    }
    let batch: Vec<String> = vec!["f00000".into(), "missing".into(), "f00001".into()];
    let r = LocalFs.remove_files(&s(&base), &batch).await;
    assert_eq!(r.len(), 3);
    assert!(r[0].is_ok() && r[2].is_ok());
    assert!(r[1].as_ref().is_err_and(|e| e.is_not_found()), "{:?}", r[1]);
    assert!(!base.join("f00000").exists() && !base.join("f00001").exists() && base.join("f00002").exists());
    let _ = std::fs::remove_dir_all(&base);
}
