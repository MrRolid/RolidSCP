//! Download for editing and safe upload back.
//!
//! Upload writes `name.filepart`, checks its size, sets the permissions of the
//! original file and only then replaces the original:
//!
//! - if the backend can replace atomically ([`Vfs::rename_replace`]: local rename,
//!   SFTP `posix-rename@openssh.com`), the part file is renamed over the original
//!   in one step – there is no moment without the file;
//! - otherwise (plain SFTPv3 `rename` does not overwrite) the original is first
//!   renamed aside (`name.rolidscp-old`), the new file renamed into place and the
//!   old one deleted; if the second rename fails the original is renamed back.
//!   The step is written to a local journal first, so a replacement cut off by
//!   a lost connection or a crash is finished or undone at the next connection
//!   ([`rolid_core::replace::repair`]).
//!
//! At no point is there a truncated file under the real name.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rolid_core::copy::PARTIAL_SUFFIX;
use rolid_core::rate::RateLimit;
use rolid_core::{Vfs, VfsError, VfsResult};
use rolid_text::txt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUF: usize = 128 * 1024;

pub struct Ctl<'a> {
    pub limit: Option<Arc<RateLimit>>,
    pub cancel: Arc<AtomicBool>,
    /// Called with the number of bytes copied so far.
    pub progress: &'a (dyn Fn(u64) + Send + Sync),
}

async fn pump(src: &dyn Vfs, from: &str, dst: &dyn Vfs, to: &str, ctl: &Ctl<'_>) -> VfsResult<u64> {
    let mut r = src.open_read(from).await?;
    let mut w = dst.open_write(to).await?;
    let mut buf = vec![0u8; BUF];
    let mut done = 0u64;
    loop {
        if ctl.cancel.load(Ordering::Relaxed) {
            return Err(VfsError::Backend(txt!("canceled")));
        }
        let chunk = ctl.limit.as_ref().map_or(BUF, |l| l.chunk(BUF));
        let n = r.read(&mut buf[..chunk]).await?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).await?;
        done += n as u64;
        (ctl.progress)(done);
        if let Some(l) = &ctl.limit {
            l.consume(n as u64).await;
        }
    }
    w.shutdown().await?;
    Ok(done)
}

/// Copy `src_path` (any backend) to the local file `dst_path`.
pub async fn download(src: &dyn Vfs, src_path: &str, local: &dyn Vfs, dst_path: &str, ctl: &Ctl<'_>) -> VfsResult<u64> {
    pump(src, src_path, local, dst_path, ctl).await
}

/// Replace `target` with `part` on `fs` without ever leaving a truncated or missing
/// file under the real name (see the module docs and [`rolid_core::replace`]).
pub async fn safe_replace(fs: &dyn Vfs, part: &str, target: &str) -> VfsResult<()> {
    rolid_core::replace::safe_replace(fs, part, target).await
}

/// Upload `local_path` to `dst_path`, keeping permission bits `mode` (of the original).
/// Returns the number of bytes uploaded.
pub async fn upload_replace(
    local: &dyn Vfs,
    local_path: &str,
    dst: &dyn Vfs,
    dst_path: &str,
    mode: Option<u32>,
    ctl: &Ctl<'_>,
) -> VfsResult<u64> {
    let part = format!("{dst_path}{PARTIAL_SUFFIX}");
    let r = async {
        let size = local.stat(local_path).await?.size;
        let n = pump(local, local_path, dst, &part, ctl).await?;
        let written = dst.stat(&part).await?.size;
        if n != size || written != size {
            return Err(VfsError::Backend(txt!(
                "upload incomplete ({written} of {size} B) – the original file was left unchanged",
                written = written,
                size = size
            )));
        }
        if let Some(m) = mode {
            // Best effort: some servers refuse chmod; the content matters more.
            let _ = dst.set_attrs(&part, Some(m & 0o7777), None).await;
        }
        safe_replace(dst, &part, dst_path).await?;
        Ok(n)
    }
    .await;
    if r.is_err() {
        let _ = dst.remove_file(&part).await;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolid_core::vfs::{Reader, Writer};
    use rolid_core::Entry;
    use rolid_local::LocalFs;

    /// LocalFs that refuses to rename anything onto `deny_to` (simulates a server
    /// error at the worst moment: original already moved aside).
    struct FailRename {
        deny_to: String,
    }

    #[async_trait::async_trait]
    impl Vfs for FailRename {
        fn label(&self) -> String {
            "test".into()
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
        async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
            if to == self.deny_to && from.ends_with(PARTIAL_SUFFIX) {
                return Err(VfsError::raw("Permission denied"));
            }
            LocalFs.rename(from, to).await
        }
        async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<Reader> {
            LocalFs.open_read_at(p, o).await
        }
        async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<Writer> {
            LocalFs.open_write_at(p, o).await
        }
        async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<std::time::SystemTime>) -> VfsResult<()> {
            LocalFs.set_attrs(p, m, t).await
        }
    }

    #[tokio::test]
    async fn failure_after_moving_the_original_aside_restores_it() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(p(&src, "a.txt"), "new").unwrap();
        std::fs::write(p(&dst, "a.txt"), "original").unwrap();
        let fs = FailRename { deny_to: p(&dst, "a.txt") };
        let r = upload_replace(&LocalFs, &p(&src, "a.txt"), &fs, &p(&dst, "a.txt"), None, &ctl()).await;
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(p(&dst, "a.txt")).unwrap(), "original");
        let names: Vec<_> = std::fs::read_dir(dst.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["a.txt"], "no .filepart / .rolidscp-old left");
    }

    fn ctl() -> Ctl<'static> {
        static NOOP: fn(u64) = |_| {};
        Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &NOOP }
    }

    fn p(d: &tempfile::TempDir, n: &str) -> String {
        d.path().join(n).to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn upload_replaces_and_keeps_the_mode() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(p(&src, "a.txt"), "new content\n").unwrap();
        std::fs::write(p(&dst, "a.txt"), "old").unwrap();
        let fs = LocalFs;
        let n = upload_replace(&fs, &p(&src, "a.txt"), &fs, &p(&dst, "a.txt"), Some(0o640), &ctl()).await.unwrap();
        assert_eq!(n, "new content\n".len() as u64);
        assert_eq!(std::fs::read_to_string(p(&dst, "a.txt")).unwrap(), "new content\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(p(&dst, "a.txt")).unwrap().permissions().mode() & 0o777, 0o640);
        }
        // Nothing left behind.
        let names: Vec<_> = std::fs::read_dir(dst.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["a.txt"]);
    }

    #[tokio::test]
    async fn failed_upload_leaves_the_original_untouched() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(p(&dst, "a.txt"), "original").unwrap();
        let fs = LocalFs;
        // Source missing: fails before anything is written.
        assert!(upload_replace(&fs, &p(&src, "nope"), &fs, &p(&dst, "a.txt"), None, &ctl()).await.is_err());
        assert_eq!(std::fs::read_to_string(p(&dst, "a.txt")).unwrap(), "original");
        // Canceled mid-way: the .filepart is removed, the original stays.
        std::fs::write(p(&src, "big"), vec![b'x'; 1024 * 1024]).unwrap();
        let c = ctl();
        c.cancel.store(true, Ordering::Relaxed);
        assert!(upload_replace(&fs, &p(&src, "big"), &fs, &p(&dst, "a.txt"), None, &c).await.is_err());
        assert_eq!(std::fs::read_to_string(p(&dst, "a.txt")).unwrap(), "original");
        assert!(!std::path::Path::new(&format!("{}.filepart", p(&dst, "a.txt"))).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unwritable_directory_keeps_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(p(&src, "a.txt"), "new").unwrap();
        std::fs::write(p(&dst, "a.txt"), "original").unwrap();
        std::fs::set_permissions(dst.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = upload_replace(&LocalFs, &p(&src, "a.txt"), &LocalFs, &p(&dst, "a.txt"), None, &ctl()).await;
        std::fs::set_permissions(dst.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if r.is_err() {
            assert_eq!(std::fs::read_to_string(p(&dst, "a.txt")).unwrap(), "original");
        } // root can write anyway
    }

    #[tokio::test]
    async fn safe_replace_without_existing_target_and_with_leftover() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(p(&d, "n.filepart"), "1").unwrap();
        safe_replace(&LocalFs, &p(&d, "n.filepart"), &p(&d, "n")).await.unwrap();
        assert_eq!(std::fs::read_to_string(p(&d, "n")).unwrap(), "1");
        // A leftover ".rolidscp-old" from an interrupted upload does not block.
        std::fs::write(p(&d, "n.rolidscp-old"), "leftover").unwrap();
        std::fs::write(p(&d, "n.filepart"), "2").unwrap();
        safe_replace(&LocalFs, &p(&d, "n.filepart"), &p(&d, "n")).await.unwrap();
        assert_eq!(std::fs::read_to_string(p(&d, "n")).unwrap(), "2");
        assert!(!std::path::Path::new(&p(&d, "n.rolidscp-old")).exists());
    }

    #[tokio::test]
    async fn download_copies_exactly() {
        let a = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(p(&a, "x.bin"), &data).unwrap();
        let n = download(&LocalFs, &p(&a, "x.bin"), &LocalFs, &p(&a, "y.bin"), &ctl()).await.unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(std::fs::read(p(&a, "y.bin")).unwrap(), data);
    }
}
