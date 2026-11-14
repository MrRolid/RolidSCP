//! Putting a finished file in place of an existing one (copy queue, F4, rename
//! with overwrite) and repairing a replacement cut off in the middle.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::replace::{repair_in, safe_replace_in};
use rolid_core::vfs::{Entry, Reader, Vfs, VfsError, VfsResult, Writer};
use rolid_local::LocalFs;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-replace-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// A "server" on the local disk: records every change, can do without
/// posix-rename, and can lose the connection at a chosen rename.
#[derive(Default)]
struct Server {
    posix_rename: bool,
    /// The connection breaks at the rename onto a path ending with this, and
    /// stays broken (`down`) from then on.
    break_at_rename_to: Option<&'static str>,
    down: AtomicBool,
    log: Mutex<Vec<String>>,
}

impl Server {
    fn up(&self) -> VfsResult<()> {
        if self.down.load(Ordering::SeqCst) {
            Err(VfsError::ConnectionLost(rolid_core::vfs::raw_text("connection lost")))
        } else {
            Ok(())
        }
    }
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn short(p: &str) -> &str {
    p.rsplit('/').next().unwrap()
}

#[async_trait]
impl Vfs for Server {
    fn label(&self) -> String {
        "user@test".into()
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn journal_key(&self) -> Option<String> {
        Some("user@test:22".into())
    }
    fn join(&self, d: &str, n: &str) -> String {
        LocalFs.join(d, n)
    }
    fn parent(&self, p: &str) -> Option<String> {
        LocalFs.parent(p)
    }
    async fn home(&self) -> VfsResult<String> {
        LocalFs.home().await
    }
    async fn canonicalize(&self, p: &str) -> VfsResult<String> {
        LocalFs.canonicalize(p).await
    }
    async fn list(&self, p: &str) -> VfsResult<Vec<Entry>> {
        self.up()?;
        LocalFs.list(p).await
    }
    async fn stat(&self, p: &str) -> VfsResult<Entry> {
        self.up()?;
        LocalFs.stat(p).await
    }
    async fn mkdir(&self, p: &str) -> VfsResult<()> {
        self.up()?;
        LocalFs.mkdir(p).await
    }
    async fn remove_file(&self, p: &str) -> VfsResult<()> {
        self.up()?;
        self.note(format!("remove {}", short(p)));
        LocalFs.remove_file(p).await
    }
    async fn remove_dir(&self, p: &str) -> VfsResult<()> {
        self.up()?;
        LocalFs.remove_dir(p).await
    }
    async fn rename(&self, f: &str, t: &str) -> VfsResult<()> {
        self.up()?;
        if self.break_at_rename_to.is_some_and(|b| t.ends_with(b)) {
            self.down.store(true, Ordering::SeqCst);
            return self.up();
        }
        // SFTPv3: rename does not overwrite.
        if Path::new(t).exists() {
            return Err(VfsError::raw("Failure (the target exists)"));
        }
        self.note(format!("rename {} -> {}", short(f), short(t)));
        LocalFs.rename(f, t).await
    }
    async fn rename_replace(&self, f: &str, t: &str) -> VfsResult<bool> {
        self.up()?;
        if !self.posix_rename {
            return Ok(false);
        }
        self.note(format!("posix-rename {} -> {}", short(f), short(t)));
        LocalFs.rename_replace(f, t).await
    }
    async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
        self.up()?;
        LocalFs.open_read_at(p, o).await
    }
    async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
        self.up()?;
        LocalFs.open_write_at(p, o).await
    }
    async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
        self.up()?;
        LocalFs.set_attrs(p, m, t).await
    }
}

/// Copy `src/f.txt` over the existing `dst/f.txt` on `server` (Overwrite).
async fn copy_over(d: &Path, server: &Server) -> VfsResult<()> {
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&LocalFs, server, Arc::new(AtomicBool::new(false)), &cb).with_conflict(Conflict::Overwrite);
    job.copy_entry(&s(&d.join("src")), "f.txt", &s(&d.join("dst"))).await
}

fn setup(name: &str) -> PathBuf {
    let d = scratch(name);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("dst")).unwrap();
    std::fs::write(d.join("src/f.txt"), "new version").unwrap();
    std::fs::write(d.join("dst/f.txt"), "original").unwrap();
    d
}

#[tokio::test]
async fn the_queue_replaces_atomically_and_never_deletes_the_target_first() {
    // posix-rename: one atomic step.
    let d = setup("atomic");
    let server = Server { posix_rename: true, ..Default::default() };
    copy_over(&d, &server).await.unwrap();
    assert_eq!(std::fs::read_to_string(d.join("dst/f.txt")).unwrap(), "new version");
    assert_eq!(server.log(), ["posix-rename f.txt.filepart -> f.txt"]);
    assert_eq!(names(&d.join("dst")), ["f.txt"]);

    // Without it: the original is moved aside first and deleted only after the
    // new file is in place (never "delete, then rename").
    let d = setup("aside");
    let server = Server::default();
    copy_over(&d, &server).await.unwrap();
    assert_eq!(std::fs::read_to_string(d.join("dst/f.txt")).unwrap(), "new version");
    assert_eq!(
        server.log(),
        ["rename f.txt -> f.txt.rolidscp-old", "rename f.txt.filepart -> f.txt", "remove f.txt.rolidscp-old"]
    );
    assert_eq!(names(&d.join("dst")), ["f.txt"]);
}

#[tokio::test]
async fn a_crash_in_the_middle_of_the_replacement_is_repaired_at_the_next_connection() {
    let d = setup("crash");
    let journal = d.join("journal");
    let dst = d.join("dst");
    std::fs::write(dst.join("f.txt.filepart"), "new version").unwrap();
    // The connection breaks just when the new file should take the name; the
    // original cannot be put back either (the connection is gone).
    let server = Server { break_at_rename_to: Some("/f.txt"), ..Default::default() };
    let r = safe_replace_in(&server, &s(&dst.join("f.txt.filepart")), &s(&dst.join("f.txt")), Some(&journal)).await;
    assert!(r.is_err());
    assert_eq!(names(&dst), ["f.txt.filepart", "f.txt.rolidscp-old"], "the name is missing now");
    assert_eq!(names(&journal).len(), 1, "the step is in the journal");

    // The next connection: the original goes back under its name.
    let again = Server::default();
    let rep = repair_in(&again, &journal).await;
    assert_eq!(rep.restored, [s(&dst.join("f.txt"))]);
    assert_eq!(std::fs::read_to_string(dst.join("f.txt")).unwrap(), "original");
    assert!(names(&journal).is_empty(), "done");
    // Another server's journal entries are not touched.
    let d2 = setup("crash-other");
    let dst2 = d2.join("dst");
    std::fs::write(dst2.join("f.txt.filepart"), "x").unwrap();
    let broken = Server { break_at_rename_to: Some("/f.txt"), ..Default::default() };
    let _ = safe_replace_in(&broken, &s(&dst2.join("f.txt.filepart")), &s(&dst2.join("f.txt")), Some(&journal)).await;
    struct Elsewhere(Server);
    #[async_trait]
    impl Vfs for Elsewhere {
        fn label(&self) -> String {
            "other".into()
        }
        fn is_remote(&self) -> bool {
            true
        }
        fn journal_key(&self) -> Option<String> {
            Some("someone@else:22".into())
        }
        fn join(&self, d: &str, n: &str) -> String {
            self.0.join(d, n)
        }
        fn parent(&self, p: &str) -> Option<String> {
            self.0.parent(p)
        }
        async fn home(&self) -> VfsResult<String> {
            self.0.home().await
        }
        async fn canonicalize(&self, p: &str) -> VfsResult<String> {
            self.0.canonicalize(p).await
        }
        async fn list(&self, p: &str) -> VfsResult<Vec<Entry>> {
            self.0.list(p).await
        }
        async fn stat(&self, p: &str) -> VfsResult<Entry> {
            self.0.stat(p).await
        }
        async fn mkdir(&self, p: &str) -> VfsResult<()> {
            self.0.mkdir(p).await
        }
        async fn remove_file(&self, p: &str) -> VfsResult<()> {
            self.0.remove_file(p).await
        }
        async fn remove_dir(&self, p: &str) -> VfsResult<()> {
            self.0.remove_dir(p).await
        }
        async fn rename(&self, f: &str, t: &str) -> VfsResult<()> {
            self.0.rename(f, t).await
        }
        async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
            self.0.open_read_at(p, o).await
        }
        async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
            self.0.open_write_at(p, o).await
        }
        async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
            self.0.set_attrs(p, m, t).await
        }
    }
    assert!(repair_in(&Elsewhere(Server::default()), &journal).await.is_empty());
    assert_eq!(names(&journal).len(), 1);
    // While the connection is still down the entry stays for later.
    let still_down = Server::default();
    still_down.down.store(true, Ordering::SeqCst);
    let rep = repair_in(&still_down, &journal).await;
    assert_eq!(rep.pending, 1);
    assert_eq!(names(&journal).len(), 1);
    assert_eq!(repair_in(&Server::default(), &journal).await.restored.len(), 1);
    assert!(names(&journal).is_empty());
}

#[tokio::test]
async fn repair_finishes_a_replacement_that_got_further() {
    let d = scratch("further");
    let journal = d.join("journal");
    std::fs::create_dir_all(d.join("dst")).unwrap();
    let dst = d.join("dst");
    // The connection breaks at the rename of the new file; the rename did go
    // through on the server, only the answer was lost.
    std::fs::write(dst.join("g.txt"), "original g").unwrap();
    std::fs::write(dst.join("g.txt.filepart"), "new g").unwrap();
    let server = Server { break_at_rename_to: Some("/g.txt"), ..Default::default() };
    let _ = safe_replace_in(&server, &s(&dst.join("g.txt.filepart")), &s(&dst.join("g.txt")), Some(&journal)).await;
    std::fs::rename(dst.join("g.txt.filepart"), dst.join("g.txt")).unwrap();
    let rep = repair_in(&Server::default(), &journal).await;
    assert!(rep.restored.is_empty() && rep.completed.is_empty(), "{rep:?}");
    assert_eq!(std::fs::read_to_string(dst.join("g.txt")).unwrap(), "new g");
    assert!(!dst.join("g.txt.rolidscp-old").exists(), "the old copy is gone");
}

#[tokio::test]
async fn leftovers_with_the_old_czech_names_are_recognised() {
    let d = scratch("legacy");
    let dst = d.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    // A rename with overwrite from 0.8.13 was cut off: the original is under
    // the old name, the real name is missing.
    std::fs::write(dst.join("f.txt.rolid-zaloha"), "original").unwrap();
    std::fs::write(dst.join("f.txt.filepart"), "new").unwrap();
    let server = Server::default();
    safe_replace_in(&server, &s(&dst.join("f.txt.filepart")), &s(&dst.join("f.txt")), None).await.unwrap();
    assert_eq!(std::fs::read_to_string(dst.join("f.txt")).unwrap(), "new");
    assert_eq!(names(&dst), ["f.txt"], "the old leftover is cleaned up");
    // The real name exists and an old leftover is next to it: it goes.
    std::fs::write(dst.join("f.txt.rolid-zaloha"), "stale").unwrap();
    std::fs::write(dst.join("f.txt.filepart"), "newer").unwrap();
    safe_replace_in(&server, &s(&dst.join("f.txt.filepart")), &s(&dst.join("f.txt")), None).await.unwrap();
    assert_eq!(names(&dst), ["f.txt"]);
    assert_eq!(std::fs::read_to_string(dst.join("f.txt")).unwrap(), "newer");
}
