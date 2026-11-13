//! F4 upload replaces the remote file: atomically via posix-rename@openssh.com when
//! the server offers it, otherwise by the rename-aside fallback. Both against a real
//! OpenSSH server.

mod sshd;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, Vfs, VfsResult};
use rolid_edit::transfer::{upload_replace, Ctl};
use rolid_local::LocalFs;
use rolid_ssh::{SftpFs, SshConnection};

/// Counts how the target was replaced.
struct Spy {
    inner: SftpFs,
    renames: AtomicUsize,
    atomic: AtomicUsize,
}

#[async_trait]
impl Vfs for Spy {
    fn label(&self) -> String {
        self.inner.label()
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn join(&self, d: &str, n: &str) -> String {
        self.inner.join(d, n)
    }
    fn parent(&self, p: &str) -> Option<String> {
        self.inner.parent(p)
    }
    async fn home(&self) -> VfsResult<String> {
        self.inner.home().await
    }
    async fn canonicalize(&self, p: &str) -> VfsResult<String> {
        self.inner.canonicalize(p).await
    }
    async fn list(&self, p: &str) -> VfsResult<Vec<Entry>> {
        self.inner.list(p).await
    }
    async fn stat(&self, p: &str) -> VfsResult<Entry> {
        self.inner.stat(p).await
    }
    async fn mkdir(&self, p: &str) -> VfsResult<()> {
        self.inner.mkdir(p).await
    }
    async fn remove_file(&self, p: &str) -> VfsResult<()> {
        self.inner.remove_file(p).await
    }
    async fn remove_dir(&self, p: &str) -> VfsResult<()> {
        self.inner.remove_dir(p).await
    }
    async fn rename(&self, f: &str, t: &str) -> VfsResult<()> {
        self.renames.fetch_add(1, Ordering::SeqCst);
        self.inner.rename(f, t).await
    }
    async fn rename_replace(&self, f: &str, t: &str) -> VfsResult<bool> {
        let r = self.inner.rename_replace(f, t).await;
        if matches!(r, Ok(true)) {
            self.atomic.fetch_add(1, Ordering::SeqCst);
        }
        r
    }
    async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
        self.inner.open_read_at(p, o).await
    }
    async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
        self.inner.open_write_at(p, o).await
    }
    async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
        self.inner.set_attrs(p, m, t).await
    }
}

fn ctl() -> Ctl<'static> {
    static NOOP: fn(u64) = |_| {};
    Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &NOOP }
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_replaces_atomically_and_by_fallback() {
    use std::os::unix::fs::PermissionsExt;
    let Some(srv) = sshd::Sshd::start("posix-rename") else { return };
    let remote = srv.data_dir("remote");
    let local = srv.data_dir("local");
    let target = remote.join("index.php");
    std::fs::write(&target, "original\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();

    let conn = SshConnection::connect(&srv.params(srv.port)).await.expect("connect");
    let spy = Spy { inner: conn.sftp().await.unwrap(), renames: AtomicUsize::new(0), atomic: AtomicUsize::new(0) };
    let dst = target.to_string_lossy().into_owned();
    let src = local.join("index.php").to_string_lossy().into_owned();

    // 1) OpenSSH offers posix-rename: one atomic step, no rename-aside.
    std::fs::write(&src, "version 1\n").unwrap();
    let inode_before = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap());
    upload_replace(&LocalFs, &src, &spy, &dst, Some(0o640), &ctl()).await.unwrap();
    assert_eq!(spy.atomic.load(Ordering::SeqCst), 1, "posix-rename used");
    assert_eq!(spy.renames.load(Ordering::SeqCst), 0, "no rename aside");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "version 1\n");
    assert_ne!(std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap()), inode_before);
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(names(&remote), ["index.php"], "no .filepart / .rolidscp-old left");

    // 2) Server without the extension (simulated): rename aside, rename in, delete old.
    spy.inner.disable_posix_rename();
    std::fs::write(&src, "version 2 – longer content\n").unwrap();
    upload_replace(&LocalFs, &src, &spy, &dst, Some(0o640), &ctl()).await.unwrap();
    assert_eq!(spy.atomic.load(Ordering::SeqCst), 1, "not used again");
    assert_eq!(spy.renames.load(Ordering::SeqCst), 2, "target → .rolidscp-old, .filepart → target");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "version 2 – longer content\n");
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(names(&remote), ["index.php"]);

    // 3) New file (no target yet) works with posix-rename too.
    let spy2 = Spy { inner: conn.sftp().await.unwrap(), renames: AtomicUsize::new(0), atomic: AtomicUsize::new(0) };
    let new_dst = remote.join("new.txt").to_string_lossy().into_owned();
    upload_replace(&LocalFs, &src, &spy2, &new_dst, None, &ctl()).await.unwrap();
    assert_eq!(std::fs::read_to_string(&new_dst).unwrap(), "version 2 – longer content\n");
    assert_eq!(names(&remote), ["index.php", "new.txt"]);
}
