//! Rename (F2) and move (F6) on real local folders: name conflicts, a change of
//! letter case on a case-insensitive file system, move by rename on the same
//! disk and copy + delete between disks (the source goes only after success).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::rename::{move_by_rename, rename_entry, MoveStep, RenameOutcome};
use rolid_core::vfs::{Entry, Reader, Vfs, VfsError, VfsResult, Writer};
use rolid_local::LocalFs;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-renmove-{name}-{}", std::process::id()));
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

#[tokio::test]
async fn rename_with_conflicts() {
    let d = scratch("conflicts");
    std::fs::write(d.join("a.txt"), "A").unwrap();
    std::fs::write(d.join("b.txt"), "B").unwrap();
    std::fs::create_dir(d.join("dir1")).unwrap();
    std::fs::create_dir(d.join("dir2")).unwrap();
    let fs = LocalFs;
    let dir = s(&d);

    assert_eq!(rename_entry(&fs, &dir, "a.txt", "a.txt", false).await.unwrap(), RenameOutcome::Unchanged);
    // An existing target is never replaced without asking.
    assert_eq!(rename_entry(&fs, &dir, "a.txt", "b.txt", false).await.unwrap(), RenameOutcome::Exists { dir: false });
    assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "B");
    assert_eq!(rename_entry(&fs, &dir, "a.txt", "dir1", false).await.unwrap(), RenameOutcome::Exists { dir: true });
    // A folder onto a folder: even "overwrite" is refused (only Cancel is offered).
    assert_eq!(rename_entry(&fs, &dir, "dir2", "dir1", false).await.unwrap(), RenameOutcome::Exists { dir: true });
    assert!(rename_entry(&fs, &dir, "dir2", "dir1", true).await.is_err());
    assert!(d.join("dir2").is_dir());
    // Overwrite after the confirmation: file by file.
    assert_eq!(rename_entry(&fs, &dir, "a.txt", "b.txt", true).await.unwrap(), RenameOutcome::Done);
    assert_eq!(names(&d), ["b.txt", "dir1", "dir2"]);
    assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "A");
    // Bad names never reach the disk.
    for bad in ["", ".", "..", "x/y", "nul\0"] {
        assert!(rename_entry(&fs, &dir, "b.txt", bad, false).await.is_err(), "{bad:?}");
    }
    assert_eq!(rename_entry(&fs, &dir, "b.txt", "new name.txt", false).await.unwrap(), RenameOutcome::Done);
    assert_eq!(names(&d), ["dir1", "dir2", "new name.txt"]);
    std::fs::remove_dir_all(&d).unwrap();
}

/// LocalFs that behaves like a case-insensitive file system (FAT, NTFS, APFS,
/// Samba): a name finds an item in any letter case, and a rename onto the same
/// item under another case does nothing.
struct CaseInsensitive;

impl CaseInsensitive {
    fn resolve(p: &str) -> Option<String> {
        let path = Path::new(p);
        let parent = path.parent()?;
        let want = path.file_name()?.to_string_lossy().to_lowercase();
        std::fs::read_dir(parent).ok()?.flatten().find_map(|e| {
            (e.file_name().to_string_lossy().to_lowercase() == want).then(|| s(&e.path()))
        })
    }
    fn real(p: &str) -> String {
        Self::resolve(p).unwrap_or_else(|| p.to_owned())
    }
}

#[async_trait]
impl Vfs for CaseInsensitive {
    fn label(&self) -> String {
        "case-insensitive".into()
    }
    fn is_remote(&self) -> bool {
        false
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
        LocalFs.canonicalize(&Self::real(p)).await
    }
    async fn list(&self, p: &str) -> VfsResult<Vec<Entry>> {
        LocalFs.list(&Self::real(p)).await
    }
    async fn stat(&self, p: &str) -> VfsResult<Entry> {
        LocalFs.stat(&Self::real(p)).await
    }
    async fn mkdir(&self, p: &str) -> VfsResult<()> {
        LocalFs.mkdir(p).await
    }
    async fn remove_file(&self, p: &str) -> VfsResult<()> {
        LocalFs.remove_file(&Self::real(p)).await
    }
    async fn remove_dir(&self, p: &str) -> VfsResult<()> {
        LocalFs.remove_dir(&Self::real(p)).await
    }
    async fn rename(&self, f: &str, t: &str) -> VfsResult<()> {
        let from = Self::real(f);
        match Self::resolve(t) {
            Some(to) if to == from => Ok(()), // "the same file": nothing happens
            Some(to) => LocalFs.rename(&from, &to).await,
            None => LocalFs.rename(&from, t).await,
        }
    }
    async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
        LocalFs.open_read_at(&Self::real(p), o).await
    }
    async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
        LocalFs.open_write_at(p, o).await
    }
    async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
        LocalFs.set_attrs(&Self::real(p), m, t).await
    }
}

#[tokio::test]
async fn case_only_rename_on_a_case_insensitive_file_system() {
    let d = scratch("case");
    std::fs::write(d.join("readme.md"), "content").unwrap();
    std::fs::write(d.join("other.txt"), "other").unwrap();
    let dir = s(&d);
    // A direct rename does nothing there – that is the trap.
    CaseInsensitive.rename(&s(&d.join("readme.md")), &s(&d.join("README.md"))).await.unwrap();
    assert_eq!(names(&d), ["other.txt", "readme.md"]);
    // Through a temporary name it works, and it is not taken for a conflict.
    assert_eq!(rename_entry(&CaseInsensitive, &dir, "readme.md", "README.md", false).await.unwrap(), RenameOutcome::Done);
    assert_eq!(names(&d), ["README.md", "other.txt"]);
    assert_eq!(std::fs::read_to_string(d.join("README.md")).unwrap(), "content");
    // Another item under a different case is a real conflict there.
    assert_eq!(
        rename_entry(&CaseInsensitive, &dir, "README.md", "OTHER.txt", false).await.unwrap(),
        RenameOutcome::Exists { dir: false }
    );
    assert_eq!(std::fs::read_to_string(d.join("other.txt")).unwrap(), "other");
    // On a case-sensitive file system the same rename works as well.
    assert_eq!(rename_entry(&LocalFs, &dir, "README.md", "Readme.md", false).await.unwrap(), RenameOutcome::Done);
    assert_eq!(names(&d), ["Readme.md", "other.txt"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[tokio::test]
async fn move_on_the_same_disk_is_a_rename() {
    use std::os::unix::fs::MetadataExt;
    let d = scratch("samedisk");
    std::fs::create_dir_all(d.join("src/folder")).unwrap();
    std::fs::create_dir(d.join("dst")).unwrap();
    std::fs::write(d.join("src/big.bin"), vec![7u8; 100_000]).unwrap();
    std::fs::write(d.join("src/folder/f.txt"), "f").unwrap();
    let ino = std::fs::metadata(d.join("src/big.bin")).unwrap().ino();
    let (src, dst) = (s(&d.join("src")), s(&d.join("dst")));
    assert_eq!(move_by_rename(&LocalFs, &src, "big.bin", &dst, "big.bin").await.unwrap(), MoveStep::Moved);
    assert_eq!(move_by_rename(&LocalFs, &src, "folder", &dst, "new").await.unwrap(), MoveStep::Moved);
    // The same file (inode), not a copy.
    assert_eq!(std::fs::metadata(d.join("dst/big.bin")).unwrap().ino(), ino);
    assert_eq!(names(&d.join("src")), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(d.join("dst/new/f.txt")).unwrap(), "f");
    // An existing target goes through the copy (which applies the conflict choice).
    std::fs::write(d.join("src/big.bin"), "new").unwrap();
    assert_eq!(move_by_rename(&LocalFs, &src, "big.bin", &dst, "big.bin").await.unwrap(), MoveStep::NeedsCopy);
    assert!(d.join("src/big.bin").exists());
    // A folder into itself is refused.
    assert!(move_by_rename(&LocalFs, &dst, "new", &s(&d.join("dst/new")), "new").await.is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

/// LocalFs as another disk: rename(2) fails with EXDEV, reads can break.
struct OtherDisk {
    /// Reads of this file fail with a disk error (not a lost connection).
    broken: Option<&'static str>,
}

struct BrokenReader;

impl tokio::io::AsyncRead for BrokenReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("disk input/output error")))
    }
}

#[async_trait]
impl Vfs for OtherDisk {
    fn label(&self) -> String {
        "other-disk".into()
    }
    fn is_remote(&self) -> bool {
        false
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
        LocalFs.list(p).await
    }
    async fn stat(&self, p: &str) -> VfsResult<Entry> {
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
        if f.contains("/src/") {
            return Err(VfsError::Io(std::io::Error::from_raw_os_error(18)));
        }
        LocalFs.rename(f, t).await
    }
    async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
        if self.broken.is_some_and(|b| p.ends_with(b)) {
            return Ok(Box::new(BrokenReader));
        }
        LocalFs.open_read_at(p, o).await
    }
    async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
        LocalFs.open_write_at(p, o).await
    }
    async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
        LocalFs.set_attrs(p, m, t).await
    }
}

fn tree(d: &Path) {
    std::fs::create_dir_all(d.join("src/folder/nested")).unwrap();
    std::fs::create_dir(d.join("dst")).unwrap();
    std::fs::write(d.join("src/a.bin"), vec![1u8; 300_000]).unwrap();
    std::fs::write(d.join("src/folder/b.txt"), "b").unwrap();
    std::fs::write(d.join("src/folder/nested/c.txt"), "c").unwrap();
}

#[tokio::test]
async fn move_between_disks_copies_then_deletes_the_source() {
    let d = scratch("otherdisk");
    tree(&d);
    let fs = OtherDisk { broken: None };
    let (src, dst) = (s(&d.join("src")), s(&d.join("dst")));
    // Different disks: rename is not possible, the item goes through the copy.
    assert_eq!(move_by_rename(&fs, &src, "a.bin", &dst, "a.bin").await.unwrap(), MoveStep::NeedsCopy);
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb).moving();
    let items = [(src.clone(), "a.bin".to_string()), (src.clone(), "folder".to_string())];
    job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.unwrap();
    // Until the copy is done, nothing is deleted.
    assert!(d.join("src/a.bin").exists() && d.join("src/folder/nested/c.txt").exists());
    let c = job.delete_moved_sources().await;
    assert_eq!((c.deleted, c.kept.len(), c.kept_dirs), (3, 0, 0));
    assert_eq!(names(&d.join("src")), Vec::<String>::new());
    assert_eq!(std::fs::read(d.join("dst/a.bin")).unwrap(), vec![1u8; 300_000]);
    assert_eq!(std::fs::read_to_string(d.join("dst/folder/nested/c.txt")).unwrap(), "c");
    std::fs::remove_dir_all(&d).unwrap();
}

#[tokio::test]
async fn single_item_gets_its_new_name_in_the_target() {
    let d = scratch("newname");
    tree(&d);
    let fs = OtherDisk { broken: None };
    let (src, dst) = (s(&d.join("src")), s(&d.join("dst")));
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb)
        .moving()
        .with_target_name(Some("renamed.bin".into()));
    job.copy_items_retrying(&[(src.clone(), "a.bin".to_string())], &dst, 0, |_| async { false }).await.unwrap();
    job.delete_moved_sources().await;
    assert_eq!(names(&d.join("dst")), ["renamed.bin"]);
    assert!(!d.join("src/a.bin").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[tokio::test]
async fn the_source_stays_after_an_error_a_skip_or_a_change() {
    let d = scratch("keep");
    tree(&d);
    let (src, dst) = (s(&d.join("src")), s(&d.join("dst")));
    let cb = |_: &Progress| {};

    // A read error in the middle: the job fails, the caller deletes nothing.
    let fs = OtherDisk { broken: Some("c.txt") };
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb).moving();
    let items = [(src.clone(), "a.bin".to_string()), (src.clone(), "folder".to_string())];
    assert!(job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.is_err());
    drop(job);
    assert!(d.join("src/a.bin").exists() && d.join("src/folder/nested/c.txt").exists());

    // Canceled before the start: nothing written, nothing to delete.
    let fs = OtherDisk { broken: None };
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(true)), &cb).moving();
    assert!(job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.is_err());
    assert_eq!(job.delete_moved_sources().await.deleted, 0);
    assert!(d.join("src/a.bin").exists());

    // "Skip existing": the skipped source stays, and so does its folder.
    let _ = std::fs::remove_dir_all(d.join("dst"));
    std::fs::create_dir_all(d.join("dst/folder")).unwrap();
    std::fs::write(d.join("dst/folder/b.txt"), "older").unwrap();
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb).moving().with_conflict(Conflict::Skip);
    job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.unwrap();
    // A source changed after its copy is kept as well.
    std::fs::write(d.join("src/a.bin"), "changed during the move").unwrap();
    let c = job.delete_moved_sources().await;
    assert_eq!(c.deleted, 1, "{c:?}"); // only c.txt
    assert_eq!(c.kept, [s(&d.join("src/a.bin"))]);
    assert_eq!(c.kept_dirs, 1); // "folder" still holds b.txt; "nested" is gone
    assert_eq!(std::fs::read_to_string(d.join("src/folder/b.txt")).unwrap(), "b");
    assert!(!d.join("src/folder/nested").exists());
    assert_eq!(std::fs::read_to_string(d.join("dst/folder/b.txt")).unwrap(), "older");
    std::fs::remove_dir_all(&d).unwrap();
}

#[tokio::test]
async fn links_are_not_moved_between_disks() {
    let d = scratch("links");
    tree(&d);
    std::os::unix::fs::symlink(d.join("src/folder"), d.join("src/link")).unwrap();
    std::os::unix::fs::symlink(d.join("src/a.bin"), d.join("src/folder/link-a")).unwrap();
    let fs = OtherDisk { broken: None };
    let (src, dst) = (s(&d.join("src")), s(&d.join("dst")));
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb).moving();
    // A link itself is refused (its target's files are never deleted through it).
    assert!(job.copy_items_retrying(&[(src.clone(), "link".to_string())], &dst, 0, |_| async { false }).await.is_err());
    // A link inside a folder stays there, with the folder.
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &cb).moving();
    job.copy_items_retrying(&[(src.clone(), "folder".to_string())], &dst, 0, |_| async { false }).await.unwrap();
    let c = job.delete_moved_sources().await;
    assert_eq!((c.deleted, c.kept_dirs), (2, 1));
    assert!(d.join("src/folder/link-a").symlink_metadata().is_ok());
    assert!(d.join("src/a.bin").exists());
    std::fs::remove_dir_all(&d).unwrap();
}
