//! Copying many small files: the target folder is listed once instead of
//! checking every file (each check is a round trip to a server), and a file
//! that appears under the same name meanwhile is still never overwritten
//! against the conflict choice.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::vfs::{Entry, Reader, Vfs, VfsResult, Writer};
use rolid_local::LocalFs;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-copylist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The local disk as a "server" that counts the questions about single
/// paths, and can put a file under the target name just before the copy is
/// put in place (another program writing at the same moment).
#[derive(Default)]
struct Counting {
    stats: AtomicUsize,
    lists: AtomicUsize,
    /// Part files opened and not yet put in place: now and the most at once.
    writing: AtomicUsize,
    writing_max: AtomicUsize,
    /// Before the rename onto a path ending with this, write "theirs" there.
    sneak_in: Option<&'static str>,
}

#[async_trait]
impl Vfs for Counting {
    fn label(&self) -> String {
        "counting".into()
    }
    fn is_remote(&self) -> bool {
        true
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
        self.lists.fetch_add(1, Ordering::SeqCst);
        LocalFs.list(p).await
    }
    async fn stat(&self, p: &str) -> VfsResult<Entry> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        LocalFs.stat(p).await
    }
    async fn mkdir(&self, p: &str) -> VfsResult<()> {
        LocalFs.mkdir(p).await
    }
    async fn remove_file(&self, p: &str) -> VfsResult<()> {
        LocalFs.remove_file(p).await
    }
    async fn remove_dir(&self, p: &str) -> VfsResult<()> {
        LocalFs.remove_dir(p).await
    }
    async fn rename(&self, f: &str, t: &str) -> VfsResult<()> {
        LocalFs.rename(f, t).await
    }
    async fn rename_replace(&self, f: &str, t: &str) -> VfsResult<bool> {
        LocalFs.rename_replace(f, t).await
    }
    async fn rename_noreplace(&self, f: &str, t: &str) -> VfsResult<bool> {
        self.writing.fetch_sub(1, Ordering::SeqCst);
        if self.sneak_in.is_some_and(|n| t.ends_with(n)) {
            std::fs::write(t, "theirs").unwrap();
        }
        LocalFs.rename_noreplace(f, t).await
    }
    async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
        LocalFs.open_read_at(p, o).await
    }
    async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
        let now = self.writing.fetch_add(1, Ordering::SeqCst) + 1;
        self.writing_max.fetch_max(now, Ordering::SeqCst);
        // Give the others a chance to start (a server answers after a while).
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        LocalFs.open_write_at(p, o).await
    }
    async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
        LocalFs.set_attrs(p, m, t).await
    }
}

fn source(base: &Path, n: usize) -> PathBuf {
    let src = base.join("src/tree");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..n {
        std::fs::write(src.join(format!("f{i:03}")), format!("ours {i}")).unwrap();
    }
    src
}

async fn copy(dst: &dyn Vfs, src: &Path, into: &Path, conflict: Conflict) -> (u64, u64) {
    let progress = std::sync::Mutex::new((0, 0));
    let cb = |p: &Progress| *progress.lock().unwrap() = (p.files_done, p.files_skipped);
    let mut job = CopyJob::new(&LocalFs, dst, Arc::new(AtomicBool::new(false)), &cb).with_conflict(conflict);
    let items = vec![(s(src.parent().unwrap()), "tree".to_string())];
    job.copy_items_retrying(&items, &s(into), 0, |_| async { false }).await.unwrap();
    let r = *progress.lock().unwrap();
    r
}

#[tokio::test]
async fn files_in_a_new_folder_are_not_checked_one_by_one() {
    let base = scratch("new");
    let src = source(&base, 50);
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let fs = Counting::default();
    let (done, skipped) = copy(&fs, &src, &dst, Conflict::Skip).await;
    assert_eq!((done, skipped), (50, 0));
    for i in 0..50 {
        assert_eq!(std::fs::read_to_string(dst.join(format!("tree/f{i:03}"))).unwrap(), format!("ours {i}"));
    }
    assert!(!std::fs::read_dir(dst.join("tree")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains(".filepart")));
    // The folder itself is checked; its 50 files are not (before: 5 checks per file).
    let stats = fs.stats.load(Ordering::SeqCst);
    assert!(stats <= 3, "{stats} checks of single paths for 50 files");
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn an_existing_folder_is_listed_once_and_its_files_skipped() {
    let base = scratch("existing");
    let src = source(&base, 20);
    let dst = base.join("dst");
    std::fs::create_dir_all(dst.join("tree")).unwrap();
    std::fs::write(dst.join("tree/f003"), "theirs").unwrap();
    // A part file of f007 from an interrupted transfer: that file takes the careful path.
    std::fs::write(dst.join("tree/f007.filepart"), "half").unwrap();
    let fs = Counting::default();
    let (done, skipped) = copy(&fs, &src, &dst, Conflict::Skip).await;
    assert_eq!((done, skipped), (19, 1));
    assert_eq!(std::fs::read_to_string(dst.join("tree/f003")).unwrap(), "theirs");
    assert_eq!(std::fs::read_to_string(dst.join("tree/f007")).unwrap(), "ours 7");
    assert_eq!(fs.lists.load(Ordering::SeqCst), 1, "the target folder, once");
    // Only f007 (its part file) takes the careful path, with its checks by name.
    let stats = fs.stats.load(Ordering::SeqCst);
    assert!(stats <= 6, "{stats} checks of single paths");
    // The same copy again: everything is there and is skipped from the listing.
    let again = Counting::default();
    let (done, skipped) = copy(&again, &src, &dst, Conflict::Skip).await;
    assert_eq!((done, skipped), (0, 20));
    let stats = again.stats.load(Ordering::SeqCst);
    assert!(stats <= 2, "{stats} checks of single paths to skip 20 files");
    let _ = std::fs::remove_dir_all(&base);
}

/// Another program writes the same name between the listing and the moment the
/// copy is put in place: "Skip existing" keeps that file, "Overwrite" replaces it.
#[tokio::test]
async fn a_file_that_appears_meanwhile_follows_the_conflict_choice() {
    for (conflict, expect) in [(Conflict::Skip, "theirs"), (Conflict::Overwrite, "ours 5")] {
        let base = scratch(&format!("race-{conflict:?}"));
        let src = source(&base, 10);
        let dst = base.join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        let fs = Counting { sneak_in: Some("tree/f005"), ..Default::default() };
        let (done, skipped) = copy(&fs, &src, &dst, conflict).await;
        assert_eq!(std::fs::read_to_string(dst.join("tree/f005")).unwrap(), expect, "{conflict:?}");
        assert_eq!(done + skipped, 10);
        assert_eq!(skipped, (conflict == Conflict::Skip) as u64);
        let left: Vec<String> = std::fs::read_dir(dst.join("tree")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!left.iter().any(|n| n.contains(".filepart") || n.contains(".rolidscp-old")), "{left:?}");
        assert_eq!(left.len(), 10);
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// Small files go several at a time (each costs round trips to a server, not
/// bandwidth): before, one after another took ~5 round trips per file.
#[tokio::test(flavor = "multi_thread")]
async fn small_files_are_copied_several_at_a_time() {
    let base = scratch("parallel");
    let src = source(&base, 64);
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let fs = Counting::default();
    let (done, _) = copy(&fs, &src, &dst, Conflict::Skip).await;
    assert_eq!(done, 64);
    let most = fs.writing_max.load(Ordering::SeqCst);
    assert!(most > 1, "at most {most} file at a time");
    for i in 0..64 {
        assert_eq!(std::fs::read_to_string(dst.join(format!("tree/f{i:03}"))).unwrap(), format!("ours {i}"));
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A move of small files where one cannot be read: the others are copied and
/// only their sources are deleted; the unreadable one stays and is reported.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_move_of_small_files_deletes_only_what_was_copied() {
    use std::os::unix::fs::PermissionsExt;
    let base = scratch("move");
    let src = source(&base, 30);
    let locked = src.join("f013");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        eprintln!("SKIP: running as root, the file stays readable");
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let fs = Counting::default();
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&LocalFs, &fs, Arc::new(AtomicBool::new(false)), &cb).moving().keep_going();
    let items = vec![(s(src.parent().unwrap()), "tree".to_string())];
    job.copy_items_retrying(&items, &s(&dst), 0, |_| async { false }).await.unwrap();
    assert_eq!(job.failures().len(), 1, "{:?}", job.failures().iter().map(|f| &f.path).collect::<Vec<_>>());
    assert!(job.failures()[0].path.ends_with("f013"));
    let cleanup = job.delete_moved_sources().await;
    assert_eq!(cleanup.deleted, 29);
    let left: Vec<String> = std::fs::read_dir(&src).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["f013"]);
    assert_eq!(std::fs::read_dir(dst.join("tree")).unwrap().count(), 29);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    let _ = std::fs::remove_dir_all(&base);
}

/// A name that is not valid UTF-8 cannot be sent to a server (SFTP names are
/// text): it is reported clearly and nothing with a made-up name is created.
#[cfg(unix)]
#[tokio::test]
async fn a_name_that_is_not_utf8_is_not_sent_to_a_server() {
    use std::os::unix::ffi::OsStrExt;
    let base = scratch("bytes");
    let src = source(&base, 2);
    std::fs::write(src.join(std::ffi::OsStr::from_bytes(b"bad\xff")), "x").unwrap();
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let fs = Counting::default();
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&LocalFs, &fs, Arc::new(AtomicBool::new(false)), &cb).keep_going();
    let items = vec![(s(src.parent().unwrap()), "tree".to_string())];
    job.copy_items_retrying(&items, &s(&dst), 0, |_| async { false }).await.unwrap();
    assert_eq!(job.failures().len(), 1);
    let error = job.failures()[0].error.to_string();
    assert!(error.contains("bad\\xFF") && error.contains("not valid UTF-8"), "{error}");
    let mut left: Vec<String> = std::fs::read_dir(dst.join("tree")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    left.sort();
    assert_eq!(left, ["f000", "f001"]);
    let _ = std::fs::remove_dir_all(&base);
}
