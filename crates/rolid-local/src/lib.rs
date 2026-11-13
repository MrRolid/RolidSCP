//! Local filesystem backend.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tokio::io::AsyncSeekExt;

use async_trait::async_trait;
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, EntryKind, Vfs, VfsError, VfsResult};

#[derive(Debug, Default, Clone)]
pub struct LocalFs;

/// A local file read at any position (no seeking, no reopening).
struct LocalRandom(std::sync::Arc<std::fs::File>);

#[async_trait]
impl rolid_core::vfs::RandomFile for LocalRandom {
    async fn read_at(&mut self, offset: u64, len: usize) -> VfsResult<Vec<u8>> {
        let f = self.0.clone();
        tokio::task::spawn_blocking(move || read_at(&f, offset, len)).await.map_err(VfsError::raw)?
    }
}

/// Up to `len` bytes at `offset` (fewer only at the end of the file).
pub fn read_at(f: &std::fs::File, offset: u64, len: usize) -> VfsResult<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(f, &mut buf[got..], offset + got as u64);
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(f, &mut buf[got..], offset + got as u64);
        match n {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

fn to_entry(name: String, meta: &std::fs::Metadata, link_target_dir: bool) -> Entry {
    let ft = meta.file_type();
    let kind = if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode())
    };
    #[cfg(not(unix))]
    let mode = None;
    Entry {
        name,
        kind,
        link_to_dir: link_target_dir,
        size: if ft.is_file() { meta.len() } else { 0 },
        modified: meta.modified().ok(),
        mode,
    }
}

fn stat_sync(path: &Path) -> std::io::Result<Entry> {
    let meta = std::fs::symlink_metadata(path)?;
    let link_dir = meta.file_type().is_symlink()
        && std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    Ok(to_entry(name, &meta, link_dir))
}

fn list_sync(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for de in std::fs::read_dir(path)? {
        let Ok(de) = de else { continue };
        let name = de.file_name().to_string_lossy().into_owned();
        let Ok(meta) = de.metadata() else { continue };
        let link_dir = meta.file_type().is_symlink()
            && std::fs::metadata(de.path())
                .map(|m| m.is_dir())
                .unwrap_or(false);
        out.push(to_entry(name, &meta, link_dir));
    }
    Ok(out)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> VfsResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(VfsError::raw)?
        .map_err(VfsError::from)
}

#[async_trait]
impl Vfs for LocalFs {
    fn label(&self) -> String {
        rolid_text::n!("Computer").into()
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn is_local_disk(&self) -> bool {
        true
    }
    fn join(&self, dir: &str, name: &str) -> String {
        Path::new(dir).join(name).to_string_lossy().into_owned()
    }
    fn parent(&self, path: &str) -> Option<String> {
        Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
    }
    async fn home(&self) -> VfsResult<String> {
        Ok(std::env::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .to_string_lossy()
            .into_owned())
    }
    async fn canonicalize(&self, path: &str) -> VfsResult<String> {
        let p = PathBuf::from(path);
        blocking(move || std::fs::canonicalize(p).map(|p| p.to_string_lossy().into_owned())).await
    }
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>> {
        let p = PathBuf::from(path);
        blocking(move || list_sync(&p)).await
    }
    async fn stat(&self, path: &str) -> VfsResult<Entry> {
        let p = PathBuf::from(path);
        blocking(move || stat_sync(&p)).await
    }
    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        Ok(tokio::fs::create_dir(path).await?)
    }
    async fn remove_file(&self, path: &str) -> VfsResult<()> {
        Ok(tokio::fs::remove_file(path).await?)
    }
    async fn remove_dir(&self, path: &str) -> VfsResult<()> {
        Ok(tokio::fs::remove_dir(path).await?)
    }
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        Ok(tokio::fs::rename(from, to).await?)
    }
    async fn rename_replace(&self, from: &str, to: &str) -> VfsResult<bool> {
        // rename(2) / MoveFileEx(REPLACE_EXISTING) replace the target atomically.
        tokio::fs::rename(from, to).await?;
        Ok(true)
    }
    #[cfg(unix)]
    async fn space(&self, path: &str) -> VfsResult<Option<rolid_core::vfs::Space>> {
        let p = PathBuf::from(path);
        blocking(move || {
            let s = rustix::fs::statvfs(&p).map_err(std::io::Error::from)?;
            Ok(Some(rolid_core::vfs::Space {
                avail: s.f_bavail.saturating_mul(s.f_frsize),
                total: s.f_blocks.saturating_mul(s.f_frsize),
            }))
        })
        .await
    }
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader> {
        let mut f = tokio::fs::File::open(path).await?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }
    async fn open_random_native(&self, path: &str) -> VfsResult<Option<Box<dyn rolid_core::vfs::RandomFile>>> {
        let f = std::fs::File::open(path)?;
        Ok(Some(Box::new(LocalRandom(std::sync::Arc::new(f)))))
    }
    async fn open_write_at(&self, path: &str, offset: u64) -> VfsResult<Writer> {
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(offset == 0)
            .open(path)
            .await?;
        if offset > 0 {
            f.set_len(offset).await?;
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }
    async fn set_attrs(
        &self,
        path: &str,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
    ) -> VfsResult<()> {
        #[cfg(unix)]
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(m)).await?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        if let Some(t) = mtime {
            let p = PathBuf::from(path);
            blocking(move || {
                std::fs::File::options()
                    .write(true)
                    .open(p)?
                    .set_modified(t)
            })
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn free_space_of_the_temp_disk() {
        let sp = LocalFs.space(&std::env::temp_dir().to_string_lossy()).await.unwrap().unwrap();
        assert!(sp.total > 0 && sp.avail <= sp.total, "{sp:?}");
    }

    use super::*;
    use rolid_core::copy::CopyJob;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[tokio::test]
    async fn list_and_recursive_copy() {
        let base = std::env::temp_dir().join(format!("rolid-test-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.bin"), vec![7u8; 600_000]).unwrap();

        let fs = LocalFs;
        let mut names: Vec<_> = fs
            .list(src.to_str().unwrap())
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sub"]);

        let cb = |_: &rolid_core::copy::Progress| {};
        let mut job = CopyJob::new(&fs, &fs, Arc::new(AtomicBool::new(false)), &cb);
        job.copy_entry(base.to_str().unwrap(), "src", dst.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(std::fs::read(dst.join("src/a.txt")).unwrap(), b"hello");
        assert_eq!(
            std::fs::read(dst.join("src/sub/b.bin")).unwrap().len(),
            600_000
        );
        assert_eq!(job.progress().files_done, 2);
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Unreadable files and folders end up in the summary of failures; the rest
    /// is copied. Without keep-going the first one stops the job (as F5 does).
    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_files_are_reported_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmpdir("unreadable");
        let s = base.join("s/tree");
        std::fs::create_dir_all(s.join("sub/locked")).unwrap();
        std::fs::write(s.join("a.txt"), b"a").unwrap();
        std::fs::write(s.join("secret.txt"), b"x").unwrap();
        std::fs::write(s.join("sub/b.txt"), b"b").unwrap();
        std::fs::write(s.join("sub/locked/c.txt"), b"c").unwrap();
        std::fs::write(s.join("z.txt"), b"z").unwrap();
        let mode = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        mode(&s.join("secret.txt"), 0o000);
        mode(&s.join("sub/locked"), 0o000);
        if std::fs::read(s.join("secret.txt")).is_ok() {
            eprintln!("SKIP: running as root, permissions are not enforced");
            mode(&s.join("sub/locked"), 0o755);
            return;
        }
        let fs = LocalFs;
        let cb = |_: &rolid_core::copy::Progress| {};
        let (src, dst) = (base.join("s").to_string_lossy().into_owned(), base.join("d").to_string_lossy().into_owned());
        let items = vec![(src.clone(), "tree".to_string())];

        // Normal mode (F5): stops at the first unreadable file.
        let mut job = CopyJob::new(&fs, &fs, Arc::new(AtomicBool::new(false)), &cb);
        assert!(job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.is_err());

        // Keep going: everything readable is copied, the rest is listed.
        // (The aborted run created `locked` with the copied permissions 000.)
        let _ = std::process::Command::new("chmod").args(["-R", "u+rwx"]).arg(base.join("d")).status();
        std::fs::remove_dir_all(base.join("d")).unwrap();
        std::fs::create_dir_all(base.join("d")).unwrap();
        let mut job = CopyJob::new(&fs, &fs, Arc::new(AtomicBool::new(false)), &cb).keep_going();
        job.copy_items_retrying(&items, &dst, 0, |_| async { false }).await.unwrap();
        let d = base.join("d/tree");
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(d.join("z.txt")).unwrap(), b"z");
        assert_eq!(std::fs::read(d.join("sub/b.txt")).unwrap(), b"b");
        assert!(!d.join("secret.txt").exists() && !d.join("secret.txt.filepart").exists());
        assert!(!d.join("sub/locked").exists(), "an unreadable folder leaves no empty copy");
        let mut failed: Vec<String> = job.failures().iter().map(|f| f.path.clone()).collect();
        failed.sort();
        let want = |p: &str| s.join(p).to_string_lossy().into_owned();
        assert_eq!(failed, [want("secret.txt"), want("sub/locked")]);
        assert!(job.failures().iter().all(|f| f.error.to_string().contains("ermission")));
        assert_eq!(job.progress().files_done, 3);
        // The source is untouched.
        assert_eq!(std::fs::read(s.join("a.txt")).unwrap(), b"a");
        mode(&s.join("sub/locked"), 0o755);
        mode(&s.join("secret.txt"), 0o644);
        let _ = std::process::Command::new("chmod").args(["-R", "u+rwx"]).arg(&base).status();
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// chmod plan: the selected folder gets the dialog's bits, its contents the
    /// separate file/folder modes; symlinks are neither followed nor changed;
    /// contents come before their folder.
    #[cfg(unix)]
    #[tokio::test]
    async fn measure_counts_like_the_copy() {
        use rolid_core::copy::{measure, Totals};
        let base = tmpdir("measure");
        let s = base.join("s");
        std::fs::create_dir_all(s.join("sub/deep")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(s.join("a"), vec![0u8; 10]).unwrap();
        std::fs::write(s.join("sub/b"), vec![0u8; 20]).unwrap();
        std::fs::write(s.join("sub/deep/c"), vec![0u8; 30]).unwrap();
        std::fs::write(base.join("outside/big"), vec![0u8; 1000]).unwrap();
        std::fs::write(base.join("single"), vec![0u8; 5]).unwrap();
        // A folder symlink is not followed (the copy skips it too).
        std::os::unix::fs::symlink(base.join("outside"), s.join("link-dir")).unwrap();
        let b = base.to_str().unwrap().to_string();
        let seen = std::sync::Mutex::new(Vec::new());
        let upd = |t: Totals| seen.lock().unwrap().push(t);
        let stop = AtomicBool::new(false);
        let items = vec![(b.clone(), "s".to_string()), (b.clone(), "single".to_string())];
        let t = measure(&LocalFs, &items, &stop, &upd).await;
        assert_eq!(t, Totals { files: 4, bytes: 65, done: true });
        assert!(seen.lock().unwrap().len() > 1, "running totals are reported");
        // Stopped: not done.
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let t = measure(&LocalFs, &items[..1], &stop, &upd).await;
        assert!(!t.done);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[tokio::test]
    async fn chmod_plan_recursive_with_separate_modes() {
        use rolid_core::perms::{self, PermEdit, Recursion, Scope};
        use std::os::unix::fs::PermissionsExt;
        let base = tmpdir("chmod");
        let s = base.join("s");
        std::fs::create_dir_all(s.join("www/img")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(s.join("www/index.php"), b"x").unwrap();
        std::fs::write(s.join("www/img/a.png"), b"x").unwrap();
        std::fs::write(base.join("outside/secret"), b"x").unwrap();
        std::os::unix::fs::symlink(base.join("outside"), s.join("www/link-dir")).unwrap();
        std::os::unix::fs::symlink(base.join("outside/secret"), s.join("www/link-file")).unwrap();
        let mode = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        let get = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        mode(&s.join("www"), 0o2775); // setgid folder
        mode(&s.join("www/img"), 0o2700);
        mode(&s.join("www/index.php"), 0o600);
        mode(&s.join("www/img/a.png"), 0o666);
        mode(&base.join("outside/secret"), 0o600);
        mode(&base.join("outside"), 0o700);

        let fs = LocalFs;
        let www = s.join("www").to_string_lossy().into_owned();
        let e = fs.stat(&www).await.unwrap();
        let mut edit = PermEdit::from_mode(0o2775);
        edit.set_octal("750").unwrap();
        let no = AtomicBool::new(false);
        let p = perms::plan(&fs, &[(www.clone(), e.clone())], &edit, Some(Recursion::default()), &no).await.unwrap();
        assert_eq!((p.files, p.dirs), (2, 2), "{:?}", p.steps);
        let order: Vec<String> = p.steps.iter().map(|x| x.path.rsplit('/').next().unwrap().to_string()).collect();
        assert_eq!(order.last().unwrap(), "www", "the folder after its contents");
        let img = order.iter().position(|n| n == "img").unwrap();
        let png = order.iter().position(|n| n == "a.png").unwrap();
        assert!(png < img);
        perms::run(&fs, &p, &no, &|_| {}).await.0.unwrap();
        assert_eq!(get(&s.join("www")), 0o2750, "the dialog's bits, setgid kept");
        assert_eq!(get(&s.join("www/img")), 0o2755, "inner folders: 755, setgid kept");
        assert_eq!(get(&s.join("www/index.php")), 0o644);
        assert_eq!(get(&s.join("www/img/a.png")), 0o644);
        assert_eq!(get(&base.join("outside")), 0o700, "a symlinked folder is not followed");
        assert_eq!(get(&base.join("outside/secret")), 0o600, "a symlinked file is not changed");

        // Only folders / only files.
        let rec = Recursion { scope: Scope::Dirs, file_mode: 0o600, dir_mode: 0o711 };
        let p = perms::plan(&fs, &[(www.clone(), e.clone())], &edit, Some(rec), &no).await.unwrap();
        assert_eq!((p.files, p.dirs), (0, 2));
        let rec = Recursion { scope: Scope::Files, file_mode: 0o600, dir_mode: 0o711 };
        let p = perms::plan(&fs, &[(www.clone(), e.clone())], &edit, Some(rec), &no).await.unwrap();
        assert_eq!((p.files, p.dirs), (2, 1), "the selected folder itself is always included");
        // Without recursion: only the selected item; a selected symlink is refused.
        let link = s.join("www/link-file").to_string_lossy().into_owned();
        let le = fs.stat(&link).await.unwrap();
        let p = perms::plan(&fs, &[(www.clone(), e), (link, le)], &edit, None, &no).await.unwrap();
        assert_eq!((p.files, p.dirs), (0, 1));
        assert_eq!(p.failures.len(), 1);
        assert!(p.failures[0].error.to_string().contains("link"));
        mode(&base.join("outside"), 0o755);
        std::fs::remove_dir_all(&base).unwrap();
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rolid-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("s")).unwrap();
        std::fs::create_dir_all(d.join("d")).unwrap();
        d
    }

    async fn copy_with(
        base: &Path,
        name: &str,
        c: rolid_core::copy::Conflict,
    ) -> rolid_core::copy::Progress {
        let fs = LocalFs;
        let cb = |_: &rolid_core::copy::Progress| {};
        let mut job =
            CopyJob::new(&fs, &fs, Arc::new(AtomicBool::new(false)), &cb).with_conflict(c);
        job.copy_entry(
            base.join("s").to_str().unwrap(),
            name,
            base.join("d").to_str().unwrap(),
        )
        .await
        .unwrap();
        job.progress().clone()
    }

    #[tokio::test]
    async fn conflict_policies_and_resume() {
        use rolid_core::copy::Conflict;
        let b = tmpdir("conflict");
        let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(b.join("s/f.bin"), &data).unwrap();

        // Skip keeps the existing target.
        std::fs::write(b.join("d/f.bin"), b"old").unwrap();
        let p = copy_with(&b, "f.bin", Conflict::Skip).await;
        assert_eq!((p.files_skipped, p.files_done), (1, 0));
        assert_eq!(std::fs::read(b.join("d/f.bin")).unwrap(), b"old");

        // Newer: target is newer than source -> skip; older -> overwrite.
        let future = SystemTime::now() + std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(b.join("d/f.bin"))
            .unwrap()
            .set_modified(future)
            .unwrap();
        assert_eq!(
            copy_with(&b, "f.bin", Conflict::Newer).await.files_skipped,
            1
        );
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(b.join("d/f.bin"))
            .unwrap()
            .set_modified(past)
            .unwrap();
        assert_eq!(copy_with(&b, "f.bin", Conflict::Newer).await.files_done, 1);
        assert_eq!(std::fs::read(b.join("d/f.bin")).unwrap(), data);
        // mtime preserved from source
        let sm = std::fs::metadata(b.join("s/f.bin"))
            .unwrap()
            .modified()
            .unwrap();
        let dm = std::fs::metadata(b.join("d/f.bin"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(sm, dm);

        // Resume from an interrupted .filepart.
        std::fs::remove_file(b.join("d/f.bin")).unwrap();
        std::fs::write(b.join("d/f.bin.filepart"), &data[..300_000]).unwrap();
        let p = copy_with(&b, "f.bin", Conflict::Resume).await;
        assert_eq!(p.bytes_resumed, 300_000);
        assert_eq!(p.bytes_done, 700_000);
        assert_eq!(std::fs::read(b.join("d/f.bin")).unwrap(), data);
        assert!(!b.join("d/f.bin.filepart").exists());

        // Resume: identical size -> skip; shorter target -> append.
        assert_eq!(
            copy_with(&b, "f.bin", Conflict::Resume).await.files_skipped,
            1
        );
        std::fs::write(b.join("d/f.bin"), &data[..10]).unwrap();
        let p = copy_with(&b, "f.bin", Conflict::Resume).await;
        assert_eq!(p.bytes_resumed, 10);
        assert_eq!(std::fs::read(b.join("d/f.bin")).unwrap(), data);

        // Overwrite never leaves a .filepart behind.
        std::fs::write(b.join("d/f.bin"), b"x").unwrap();
        copy_with(&b, "f.bin", Conflict::Overwrite).await;
        assert_eq!(std::fs::read(b.join("d/f.bin")).unwrap(), data);
        assert!(!b.join("d/f.bin.filepart").exists());
        std::fs::remove_dir_all(&b).unwrap();
    }

    /// LocalFs whose reads break with "connection aborted" (like a dropped SSH
    /// connection) after `fail_after` bytes, `failures` times.
    struct FlakyFs {
        fail_after: u64,
        failures: std::sync::atomic::AtomicU32,
        opens: std::sync::Mutex<Vec<(String, u64)>>,
    }

    struct FlakyReader {
        inner: rolid_core::vfs::Reader,
        left: Option<u64>,
    }

    impl tokio::io::AsyncRead for FlakyReader {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.left == Some(0) {
                return std::task::Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "connection interrupted",
                )));
            }
            let before = buf.filled().len();
            let r = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
            if let (std::task::Poll::Ready(Ok(())), Some(l)) = (&r, self.left) {
                let n = (buf.filled().len() - before) as u64;
                if n > l {
                    // Only hand out up to the break point.
                    let keep = before + l as usize;
                    buf.set_filled(keep);
                    self.left = Some(0);
                } else {
                    self.left = Some(l - n);
                }
            }
            r
        }
    }

    #[async_trait]
    impl Vfs for FlakyFs {
        fn label(&self) -> String {
            "flaky".into()
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
            LocalFs.rename(f, t).await
        }
        async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<rolid_core::vfs::Reader> {
            self.opens.lock().unwrap().push((p.rsplit('/').next().unwrap().to_string(), o));
            let inner = LocalFs.open_read_at(p, o).await?;
            let left = if p.ends_with("big.bin")
                && self.failures.load(std::sync::atomic::Ordering::SeqCst) > 0
            {
                self.failures.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                Some(self.fail_after)
            } else {
                None
            };
            Ok(Box::new(FlakyReader { inner, left }))
        }
        async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<rolid_core::vfs::Writer> {
            LocalFs.open_write_at(p, o).await
        }
        async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
            LocalFs.set_attrs(p, m, t).await
        }
    }

    fn flaky(failures: u32) -> FlakyFs {
        FlakyFs {
            fail_after: 1_000_000,
            failures: std::sync::atomic::AtomicU32::new(failures),
            opens: Default::default(),
        }
    }

    #[tokio::test]
    async fn lost_connection_continues_where_it_stopped() {
        let base = tmpdir("retry");
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(base.join("s/a.txt"), b"first").unwrap();
        std::fs::write(base.join("s/big.bin"), &big).unwrap();
        std::fs::write(base.join("s/z.txt"), b"last").unwrap();
        let src = flaky(1);
        let cb = |_: &rolid_core::copy::Progress| {};
        let mut job = CopyJob::new(&src, &LocalFs, Arc::new(AtomicBool::new(false)), &cb);
        let s = base.join("s").to_string_lossy().into_owned();
        let items: Vec<(String, String)> =
            ["a.txt", "big.bin", "z.txt"].iter().map(|n| (s.clone(), n.to_string())).collect();
        let mut waits = Vec::new();
        job.copy_items_retrying(&items, base.join("d").to_str().unwrap(), 5, |n| {
            waits.push(n);
            async { true }
        })
        .await
        .unwrap();
        assert_eq!(waits, [1], "waited for the connection once");
        assert_eq!(std::fs::read(base.join("d/big.bin")).unwrap(), big, "byte for byte");
        assert_eq!(std::fs::read(base.join("d/a.txt")).unwrap(), b"first");
        assert_eq!(std::fs::read(base.join("d/z.txt")).unwrap(), b"last");
        let opens = src.opens.lock().unwrap().clone();
        assert_eq!(
            opens,
            [
                ("a.txt".to_string(), 0),
                ("big.bin".to_string(), 0),
                // continued from the .filepart (local destination: exactly where it stopped)
                ("big.bin".to_string(), 1_000_000),
                ("z.txt".to_string(), 0)
            ],
            "a.txt not copied again"
        );
        assert!(!base.join("d/big.bin.filepart").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// LocalFs whose writers keep data in memory until flushed (like
    /// `tokio::fs::File` with a write still in flight, or a pipelined SFTP
    /// writer).
    struct BufferedFs;

    #[async_trait]
    impl Vfs for BufferedFs {
        fn label(&self) -> String {
            "buffered".into()
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
            LocalFs.rename(f, t).await
        }
        async fn open_read_at(&self, p: &str, o: u64) -> VfsResult<rolid_core::vfs::Reader> {
            LocalFs.open_read_at(p, o).await
        }
        async fn open_write_at(&self, p: &str, o: u64) -> VfsResult<rolid_core::vfs::Writer> {
            let w = LocalFs.open_write_at(p, o).await?;
            Ok(Box::new(tokio::io::BufWriter::with_capacity(8 << 20, w)))
        }
        async fn set_attrs(&self, p: &str, m: Option<u32>, t: Option<SystemTime>) -> VfsResult<()> {
            LocalFs.set_attrs(p, m, t).await
        }
    }

    /// The bytes written before the connection broke reach the .filepart, so
    /// the retry continues exactly where it stopped (the cause of the once
    /// flaky test above: the last write was still in flight).
    #[tokio::test]
    async fn written_bytes_land_before_the_retry() {
        let base = tmpdir("buffered");
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 13 % 251) as u8).collect();
        std::fs::write(base.join("s/big.bin"), &big).unwrap();
        let src = flaky(1);
        let cb = |_: &rolid_core::copy::Progress| {};
        let mut job = CopyJob::new(&src, &BufferedFs, Arc::new(AtomicBool::new(false)), &cb);
        let items = vec![(base.join("s").to_string_lossy().into_owned(), "big.bin".to_string())];
        job.copy_items_retrying(&items, base.join("d").to_str().unwrap(), 5, |_| async { true })
            .await
            .unwrap();
        assert_eq!(std::fs::read(base.join("d/big.bin")).unwrap(), big);
        let opens = src.opens.lock().unwrap().clone();
        assert_eq!(opens, [("big.bin".to_string(), 0), ("big.bin".to_string(), 1_000_000)]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[tokio::test]
    async fn gives_up_after_the_retry_limit() {
        let base = tmpdir("giveup");
        // Every attempt gets 1 MB further: 20 MB need more than 5 retries.
        std::fs::write(base.join("s/big.bin"), vec![1u8; 20_000_000]).unwrap();
        let src = flaky(100);
        let cb = |_: &rolid_core::copy::Progress| {};
        let mut job = CopyJob::new(&src, &LocalFs, Arc::new(AtomicBool::new(false)), &cb);
        let items = vec![(base.join("s").to_string_lossy().into_owned(), "big.bin".to_string())];
        let mut waits = 0;
        let r = job
            .copy_items_retrying(&items, base.join("d").to_str().unwrap(), 5, |_| {
                waits += 1;
                async { true }
            })
            .await;
        let e = r.unwrap_err();
        assert!(e.is_connection_lost());
        assert!(e.to_string().contains("after 5 attempts"), "{e}");
        assert_eq!(waits, 5);
        assert!(!base.join("d/big.bin").exists(), "no truncated file under the real name");
        // Reconnect refused (user canceled): fails at once.
        let src = flaky(1);
        let mut job = CopyJob::new(&src, &LocalFs, Arc::new(AtomicBool::new(false)), &cb);
        let r = job.copy_items_retrying(&items, base.join("d").to_str().unwrap(), 5, |_| async { false }).await;
        assert!(r.unwrap_err().is_connection_lost());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
