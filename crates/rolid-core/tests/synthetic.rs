//! Terabytes to exabytes through the real copy code (roadmap 0.8.16), without
//! a disk that big: a synthetic file system whose files are a pattern computed
//! from the position (reading costs no disk) and whose target checks every
//! byte written against that pattern instead of storing it. A part file can be
//! "fast-forwarded" to near the end of a petabyte, as if the first run had
//! written it, so the resume, the counters and the totals are tried at sizes no
//! test disk holds; what is really transferred is the tail.
//!
//! `terabyte_through_the_copy_code` (ignored) streams a whole file of
//! `ROLIDSCP_SYNTH_GIB` GiB (default 1024) and reports the speed.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use rolid_core::copy::{self, Conflict, CopyJob, Progress, Totals, PARTIAL_SUFFIX, RESUME_MARGIN};
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, EntryKind, Vfs, VfsError, VfsResult};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;
const TIB: u64 = 1024 * GIB;
const PIB: u64 = 1024 * TIB;
const EIB: u64 = 1024 * PIB;

/// The pattern: 8 bytes for each 8-byte word, from its index.
fn word(k: u64) -> u64 {
    let mut z = k.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The pattern's bytes at `pos` into `buf`.
fn fill(pos: u64, buf: &mut [u8]) {
    let mut i = 0;
    let mut p = pos;
    // Up to the next word boundary, then whole words.
    while i < buf.len() && !p.is_multiple_of(8) {
        buf[i] = word(p / 8).to_le_bytes()[(p % 8) as usize];
        i += 1;
        p = p.wrapping_add(1);
    }
    while buf.len() - i >= 8 {
        buf[i..i + 8].copy_from_slice(&word(p / 8).to_le_bytes());
        i += 8;
        p = p.wrapping_add(8);
    }
    while i < buf.len() {
        buf[i] = word(p / 8).to_le_bytes()[(p % 8) as usize];
        i += 1;
        p = p.wrapping_add(1);
    }
}

enum Node {
    Dir,
    /// The pattern, `size` bytes (a source file, or a target written so far).
    Pattern(u64),
    /// Stored bytes (the `.filepart-info` files the copy writes).
    Bytes(Vec<u8>),
}

#[derive(Default)]
struct Inner {
    nodes: Mutex<HashMap<String, Node>>,
    /// Bytes written that differ from the pattern at their position.
    wrong: AtomicU64,
    /// Bytes checked by the target.
    written: AtomicU64,
    /// Writes to a part file that did not start where the file ends.
    gaps: AtomicU64,
    /// A full disk: part files cannot grow past this many bytes, and the
    /// error a write past it gets.
    limit: Mutex<Option<(u64, Fail)>>,
    /// What [`Vfs::space`] tells (`None`: the backend cannot tell).
    avail: Mutex<Option<u64>>,
}

#[derive(Clone, Copy, Debug)]
enum Fail {
    /// A local disk: ENOSPC.
    Full,
    /// A local disk: EDQUOT.
    Quota,
    /// An SFTP server: only "Failure".
    Failure,
    /// The connection broke.
    Lost,
}

impl Fail {
    fn error(self) -> io::Error {
        match self {
            Fail::Full => io::Error::from(io::ErrorKind::StorageFull),
            Fail::Quota => io::Error::from(io::ErrorKind::QuotaExceeded),
            Fail::Failure => io::Error::other("Failure"),
            Fail::Lost => io::Error::from(io::ErrorKind::ConnectionReset),
        }
    }
}

#[derive(Clone)]
struct Synth {
    remote: bool,
    inner: Arc<Inner>,
}

impl Synth {
    fn new(remote: bool) -> Synth {
        let s = Synth { remote, inner: Arc::default() };
        s.put("/", Node::Dir);
        s
    }
    fn put(&self, path: &str, n: Node) {
        self.inner.nodes.lock().unwrap().insert(path.to_string(), n);
    }
    fn size(&self, path: &str) -> Option<u64> {
        match self.inner.nodes.lock().unwrap().get(path)? {
            Node::Pattern(s) => Some(*s),
            Node::Bytes(b) => Some(b.len() as u64),
            Node::Dir => None,
        }
    }
    fn wrong(&self) -> u64 {
        self.inner.wrong.load(Ordering::Relaxed)
    }
    fn written(&self) -> u64 {
        self.inner.written.load(Ordering::Relaxed)
    }
}

fn entry(name: &str, n: &Node) -> Entry {
    let (kind, size) = match n {
        Node::Dir => (EntryKind::Dir, 0),
        Node::Pattern(s) => (EntryKind::File, *s),
        Node::Bytes(b) => (EntryKind::File, b.len() as u64),
    };
    Entry { name: name.to_string(), kind, link_to_dir: false, size, modified: Some(SystemTime::UNIX_EPOCH), mode: Some(0o644) }
}

fn missing() -> VfsError {
    VfsError::Io(io::Error::new(io::ErrorKind::NotFound, "no such file"))
}

struct PatternReader {
    pos: u64,
    end: u64,
}

impl AsyncRead for PatternReader {
    fn poll_read(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let n = (buf.remaining() as u64).min(self.end - self.pos) as usize;
        let pos = self.pos;
        fill(pos, buf.initialize_unfilled_to(n));
        buf.advance(n);
        self.pos += n as u64;
        Poll::Ready(Ok(()))
    }
}

/// Writes into a part file: checked against the pattern, the file's size
/// follows (as on a disk, for a resume after an interruption).
struct CheckingWriter {
    inner: Arc<Inner>,
    path: String,
    pos: u64,
    scratch: Vec<u8>,
}

impl AsyncWrite for CheckingWriter {
    fn poll_write(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        // A full disk takes what still fits, then refuses.
        let limit = *this.inner.limit.lock().unwrap();
        let buf = match limit {
            Some((at, fail)) if this.pos >= at => return Poll::Ready(Err(fail.error())),
            Some((at, _)) => &buf[..buf.len().min((at - this.pos) as usize)],
            None => buf,
        };
        this.scratch.resize(buf.len(), 0);
        fill(this.pos, &mut this.scratch);
        if this.scratch != buf {
            let bad = this.scratch.iter().zip(buf).filter(|(a, b)| a != b).count();
            this.inner.wrong.fetch_add(bad as u64, Ordering::Relaxed);
        }
        this.inner.written.fetch_add(buf.len() as u64, Ordering::Relaxed);
        this.pos += buf.len() as u64;
        this.inner.nodes.lock().unwrap().insert(this.path.clone(), Node::Pattern(this.pos));
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Writes into any other file: stored.
struct StoringWriter {
    inner: Arc<Inner>,
    path: String,
    data: Vec<u8>,
}

impl AsyncWrite for StoringWriter {
    fn poll_write(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.data.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        let data = std::mem::take(&mut self.data);
        self.inner.nodes.lock().unwrap().insert(self.path.clone(), Node::Bytes(data));
        Poll::Ready(Ok(()))
    }
}

#[async_trait]
impl Vfs for Synth {
    fn label(&self) -> String {
        "synthetic".into()
    }
    fn is_remote(&self) -> bool {
        self.remote
    }
    fn join(&self, dir: &str, name: &str) -> String {
        if dir.ends_with('/') {
            format!("{dir}{name}")
        } else {
            format!("{dir}/{name}")
        }
    }
    fn parent(&self, path: &str) -> Option<String> {
        let i = path.rfind('/')?;
        (path != "/").then(|| if i == 0 { "/".to_string() } else { path[..i].to_string() })
    }
    async fn home(&self) -> VfsResult<String> {
        Ok("/".into())
    }
    async fn canonicalize(&self, path: &str) -> VfsResult<String> {
        Ok(path.into())
    }
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>> {
        let nodes = self.inner.nodes.lock().unwrap();
        let mut out = Vec::new();
        for (p, n) in nodes.iter() {
            if p != "/" && self.parent(p).as_deref() == Some(path) {
                out.push(entry(&p[p.rfind('/').unwrap() + 1..], n));
            }
        }
        Ok(out)
    }
    async fn stat(&self, path: &str) -> VfsResult<Entry> {
        let nodes = self.inner.nodes.lock().unwrap();
        let n = nodes.get(path).ok_or_else(missing)?;
        Ok(entry(&path[path.rfind('/').unwrap() + 1..], n))
    }
    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.put(path, Node::Dir);
        Ok(())
    }
    async fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.nodes.lock().unwrap().remove(path).map(|_| ()).ok_or_else(missing)
    }
    async fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.remove_file(path).await
    }
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let mut nodes = self.inner.nodes.lock().unwrap();
        let n = nodes.remove(from).ok_or_else(missing)?;
        nodes.insert(to.to_string(), n);
        Ok(())
    }
    async fn rename_replace(&self, from: &str, to: &str) -> VfsResult<bool> {
        self.rename(from, to).await.map(|()| true)
    }
    async fn rename_noreplace(&self, from: &str, to: &str) -> VfsResult<bool> {
        if self.inner.nodes.lock().unwrap().contains_key(to) {
            return Err(VfsError::Io(io::Error::new(io::ErrorKind::AlreadyExists, "exists")));
        }
        self.rename(from, to).await.map(|()| true)
    }
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader> {
        let nodes = self.inner.nodes.lock().unwrap();
        match nodes.get(path).ok_or_else(missing)? {
            Node::Pattern(size) => Ok(Box::new(PatternReader { pos: offset.min(*size), end: *size })),
            Node::Bytes(b) => Ok(Box::new(io::Cursor::new(b[(offset as usize).min(b.len())..].to_vec()))),
            Node::Dir => Err(missing()),
        }
    }
    async fn open_write_at(&self, path: &str, offset: u64) -> VfsResult<Writer> {
        if path.ends_with(PARTIAL_SUFFIX) {
            let mut nodes = self.inner.nodes.lock().unwrap();
            let have = match nodes.get(path) {
                Some(Node::Pattern(s)) => *s,
                _ => 0,
            };
            if offset > have {
                self.inner.gaps.fetch_add(1, Ordering::Relaxed);
            }
            nodes.insert(path.to_string(), Node::Pattern(offset.min(have)));
            return Ok(Box::new(CheckingWriter { inner: self.inner.clone(), path: path.into(), pos: offset, scratch: Vec::new() }));
        }
        Ok(Box::new(StoringWriter { inner: self.inner.clone(), path: path.into(), data: Vec::new() }))
    }
    async fn set_attrs(&self, _: &str, _: Option<u32>, _: Option<SystemTime>) -> VfsResult<()> {
        Ok(())
    }
    async fn space(&self, _: &str) -> VfsResult<Option<rolid_core::vfs::Space>> {
        Ok(self.inner.avail.lock().unwrap().map(|avail| rolid_core::vfs::Space { avail, total: 100 * GIB }))
    }
}

/// What the progress callback saw.
#[derive(Default)]
struct Seen {
    calls: AtomicU64,
    shown: AtomicU64,
    backwards: AtomicU64,
    done: AtomicU64,
    resumed: AtomicU64,
}

impl Seen {
    fn record(&self, p: &Progress) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let shown = p.bytes_done.saturating_add(p.bytes_resumed);
        if self.shown.swap(shown, Ordering::Relaxed) > shown {
            self.backwards.fetch_add(1, Ordering::Relaxed);
        }
        self.done.store(p.bytes_done, Ordering::Relaxed);
        self.resumed.store(p.bytes_resumed, Ordering::Relaxed);
    }
}

/// Copy `/src/<name>` of `src` into `/dst` of `dst` (both folders exist).
async fn copy(src: &Synth, dst: &Synth, names: &[&str], conflict: Conflict, cancel: Arc<AtomicBool>, seen: &Seen) -> VfsResult<()> {
    let cb = |p: &Progress| seen.record(p);
    let mut job = CopyJob::new(src, dst, cancel, &cb).with_conflict(conflict);
    let items: Vec<(String, String)> = names.iter().map(|n| ("/src".to_string(), n.to_string())).collect();
    job.copy_items_retrying(&items, "/dst", 0, |_| async { false }).await
}

fn pair(remote_target: bool) -> (Synth, Synth) {
    let (src, dst) = (Synth::new(false), Synth::new(remote_target));
    src.put("/src", Node::Dir);
    dst.put("/dst", Node::Dir);
    (src, dst)
}

/// A first run that is canceled at once (it leaves the part file and its
/// info file), then the part file "fast-forwarded" to `have` bytes, as if the
/// first run had written them. Returns what the run that continues saw.
async fn resume_near_the_end(size: u64, have: u64, remote: bool) -> (Synth, Seen) {
    let (src, dst) = pair(remote);
    src.put("/src/big.bin", Node::Pattern(size));
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let cb = move |p: &Progress| {
        if p.bytes_done > 0 {
            c2.store(true, Ordering::Relaxed);
        }
    };
    {
        let mut job = CopyJob::new(&src, &dst, cancel.clone(), &cb);
        let r = job.copy_items_retrying(&[("/src".into(), "big.bin".into())], "/dst", 0, |_| async { false }).await;
        assert!(r.as_ref().is_err_and(|e| e.is_canceled()), "{:?}", r.err());
    }
    assert!(dst.size("/dst/big.bin.filepart-info").is_some(), "the first run left its info file");
    dst.put("/dst/big.bin.filepart", Node::Pattern(have));
    let seen = Seen::default();
    copy(&src, &dst, &["big.bin"], Conflict::Resume, Arc::new(AtomicBool::new(false)), &seen).await.expect("resume");
    (dst, seen)
}

#[tokio::test]
async fn files_of_a_terabyte_to_the_top_of_u64_resume_near_their_end() {
    for size in [TIB, PIB, 8 * EIB, u64::MAX] {
        for remote in [false, true] {
            let tail = 64 * MIB + 12_345;
            let (dst, seen) = resume_near_the_end(size, size - tail, remote).await;
            let margin = if remote { RESUME_MARGIN } else { 0 };
            let what = format!("{size} B, remote target: {remote}");
            assert_eq!(dst.size("/dst/big.bin"), Some(size), "{what}: the whole file is in place");
            assert_eq!(dst.size("/dst/big.bin.filepart"), None, "{what}");
            assert_eq!(dst.size("/dst/big.bin.filepart-info"), None, "{what}");
            assert_eq!(dst.wrong(), 0, "{what}: wrong bytes");
            assert_eq!(dst.inner.gaps.load(Ordering::Relaxed), 0, "{what}: a write after the end of the part file");
            let (done, resumed) = (seen.done.load(Ordering::Relaxed), seen.resumed.load(Ordering::Relaxed));
            assert_eq!(done, tail + margin, "{what}: transferred");
            assert_eq!(resumed, size - tail - margin, "{what}: counted as already there");
            assert_eq!(done + resumed, size, "{what}: done + resumed");
            assert_eq!(seen.backwards.load(Ordering::Relaxed), 0, "{what}: the counter went back");
        }
    }
}

#[tokio::test]
async fn totals_of_files_bigger_than_u64_together_stop_at_its_top() {
    let src = Synth::new(true);
    src.put("/src", Node::Dir);
    src.put("/src/d", Node::Dir);
    for i in 0..3 {
        src.put(&format!("/src/d/x{i}"), Node::Pattern(8 * EIB));
    }
    src.put("/src/y", Node::Pattern(8 * EIB));
    let items = [("/src".to_string(), "d".to_string()), ("/src".to_string(), "y".to_string())];
    let t: Totals = copy::measure(&src, &items, &AtomicBool::new(false), &|_| {}).await;
    assert_eq!((t.files, t.bytes, t.done), (4, u64::MAX, true));
}

#[tokio::test]
async fn two_resumed_exabyte_files_in_one_job_do_not_overflow_the_counters() {
    let (src, dst) = pair(true);
    let size = 12 * EIB;
    let tail = 8 * MIB;
    // The part files and their info files of a first, canceled run.
    for name in ["a.bin", "b.bin"] {
        src.put(&format!("/src/{name}"), Node::Pattern(size));
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let cb = move |p: &Progress| {
            if p.bytes_done > 0 {
                c2.store(true, Ordering::Relaxed);
            }
        };
        let mut job = CopyJob::new(&src, &dst, cancel.clone(), &cb);
        let _ = job.copy_items_retrying(&[("/src".into(), name.into())], "/dst", 0, |_| async { false }).await;
        dst.put(&format!("/dst/{name}.filepart"), Node::Pattern(size - tail));
    }
    let seen = Seen::default();
    copy(&src, &dst, &["a.bin", "b.bin"], Conflict::Resume, Arc::new(AtomicBool::new(false)), &seen).await.expect("resume");
    assert_eq!(dst.size("/dst/a.bin"), Some(size));
    assert_eq!(dst.size("/dst/b.bin"), Some(size));
    assert_eq!(dst.wrong(), 0);
    assert_eq!(seen.done.load(Ordering::Relaxed), 2 * (tail + RESUME_MARGIN));
    // 2 × 12 EiB does not fit: the count stops at the top instead of wrapping to a small number.
    assert_eq!(seen.resumed.load(Ordering::Relaxed), u64::MAX);
}

#[tokio::test]
async fn a_whole_file_streamed_is_right_byte_for_byte() {
    let (src, dst) = pair(true);
    let size = 96 * MIB + 7;
    src.put("/src/f.bin", Node::Pattern(size));
    let seen = Seen::default();
    copy(&src, &dst, &["f.bin"], Conflict::Overwrite, Arc::new(AtomicBool::new(false)), &seen).await.expect("copy");
    assert_eq!(dst.size("/dst/f.bin"), Some(size));
    assert_eq!(dst.written(), size);
    assert_eq!(dst.wrong(), 0);
    assert_eq!(seen.done.load(Ordering::Relaxed), size);
    assert_eq!(seen.backwards.load(Ordering::Relaxed), 0);
}

/// A copy into a target that fills up at 40 MiB: the error the user sees,
/// then "Try again" (the same job continued) once there is room again.
async fn full_then_try_again(fail: Fail, avail: Option<u64>) -> (String, Synth, Seen) {
    let (src, dst) = pair(matches!(fail, Fail::Failure));
    let size = 100 * MIB;
    src.put("/src/f.bin", Node::Pattern(size));
    // An older file of the same name: it must stay as it is.
    dst.put("/dst/f.bin", Node::Bytes(b"the old one".to_vec()));
    *dst.inner.limit.lock().unwrap() = Some((40 * MIB, fail));
    *dst.inner.avail.lock().unwrap() = avail;
    let first = Seen::default();
    let cb = |p: &Progress| first.record(p);
    let mut job = CopyJob::new(&src, &dst, Arc::new(AtomicBool::new(false)), &cb);
    let items = [("/src".to_string(), "f.bin".to_string())];
    let err = job.copy_items_retrying(&items, "/dst", 0, |_| async { false }).await.expect_err("the disk is full");
    let text = err.text().to_string();
    let state = job.state().clone();
    drop(job);
    assert_eq!(dst.size("/dst/f.bin"), Some(11), "{fail:?}: the old file is untouched");
    assert!(dst.size("/dst/f.bin.filepart").is_some_and(|s| s <= 40 * MIB), "{fail:?}: the written part is kept");
    // Room again; "Try again" starts the job anew with what it had done (as
    // the transfer queue does).
    *dst.inner.limit.lock().unwrap() = None;
    let seen = Seen::default();
    let cb = |p: &Progress| seen.record(p);
    let mut job = CopyJob::new(&src, &dst, Arc::new(AtomicBool::new(false)), &cb).with_state(state);
    job.copy_items_retrying(&items, "/dst", 0, |_| async { false }).await.expect("try again");
    drop(job);
    (text, dst, seen)
}

#[tokio::test]
async fn a_full_disk_or_quota_is_named_and_try_again_continues() {
    let cases = [
        (Fail::Full, None, "The target disk is full"),
        (Fail::Quota, None, "The disk quota on the target is used up"),
        // A server that answers only "Failure" and has no room left.
        (Fail::Failure, Some(0), "The target disk is full"),
        // "Failure" with room (a quota does not show in the free space) or
        // a server that cannot tell: maybe full, maybe a quota.
        (Fail::Failure, Some(50 * GIB), "The server refused to write (Failure). Its disk may be full or your disk quota used up"),
        (Fail::Failure, None, "The server refused to write (Failure)"),
    ];
    for (fail, avail, want) in cases {
        let (text, dst, seen) = full_then_try_again(fail, avail).await;
        assert!(text.starts_with(want), "{fail:?} {avail:?}: {text}");
        assert!(text.contains("Try again"), "{text}");
        assert_eq!(dst.size("/dst/f.bin"), Some(100 * MIB), "{fail:?}: complete after trying again");
        assert_eq!(dst.wrong(), 0, "{fail:?}");
        let (done, resumed) = (seen.done.load(Ordering::Relaxed), seen.resumed.load(Ordering::Relaxed));
        // Continued from the part file, not from the start (a remote target
        // rewrites the last RESUME_MARGIN).
        let margin = if matches!(fail, Fail::Failure) { RESUME_MARGIN } else { 0 };
        assert_eq!(resumed, 40 * MIB - margin, "{fail:?}");
        assert_eq!(done + resumed, 100 * MIB, "{fail:?}");
    }
}

/// The connection breaks in the middle of a big file and the job continues by
/// itself once it is back: what the window shows must not count the part
/// written before the break twice.
#[tokio::test]
async fn a_job_continued_after_a_lost_connection_counts_each_byte_once() {
    let (src, dst) = pair(true);
    let size = 100 * MIB;
    src.put("/src/f.bin", Node::Pattern(size));
    *dst.inner.limit.lock().unwrap() = Some((40 * MIB, Fail::Lost));
    let seen = Seen::default();
    let cb = |p: &Progress| seen.record(p);
    let mut job = CopyJob::new(&src, &dst, Arc::new(AtomicBool::new(false)), &cb);
    let items = [("/src".to_string(), "f.bin".to_string())];
    let inner = dst.inner.clone();
    job.copy_items_retrying(&items, "/dst", 3, |_| {
        *inner.limit.lock().unwrap() = None;
        async { true }
    })
    .await
    .expect("continued");
    let p = job.progress().clone();
    drop(job);
    assert_eq!(dst.size("/dst/f.bin"), Some(size));
    assert_eq!(dst.wrong(), 0);
    assert_eq!(p.bytes_done.saturating_add(p.bytes_resumed), size, "shown at the end: {} + {}", p.bytes_done, p.bytes_resumed);
}

/// A move interrupted after some files were complete in the target, then
/// started again with "Resume": those files are moved too, but only when the
/// checksums are compared (and match).
#[tokio::test]
async fn a_move_continued_with_resume_deletes_sources_already_copied_only_after_checksums() {
    for verify in [true, false] {
        let (src, dst) = pair(true);
        src.put("/src/d", Node::Dir);
        src.put("/src/d/done.bin", Node::Pattern(3 * MIB));
        src.put("/src/d/todo.bin", Node::Pattern(5 * MIB));
        // The first run had copied done.bin completely.
        dst.put("/dst/d", Node::Dir);
        dst.put("/dst/d/done.bin", Node::Pattern(3 * MIB));
        static NOOP: fn(&Progress) = |_| {};
        let mut job = CopyJob::new(&src, &dst, Arc::new(AtomicBool::new(false)), &NOOP).with_conflict(Conflict::Resume).moving().verify_moved(verify);
        job.copy_items_retrying(&[("/src".into(), "d".into())], "/dst", 0, |_| async { false }).await.expect("move");
        let c = job.delete_moved_sources().await;
        assert!(c.mismatch.is_empty() && c.changed.is_empty(), "{c:?}");
        assert_eq!(src.size("/src/d/todo.bin"), None, "verify {verify}: copied now, source deleted");
        assert_eq!(dst.size("/dst/d/todo.bin"), Some(5 * MIB));
        assert_eq!(dst.size("/dst/d/done.bin"), Some(3 * MIB));
        let done_src = src.size("/src/d/done.bin");
        if verify {
            assert_eq!(done_src, None, "same checksum: the source of the file copied before is deleted");
        } else {
            assert_eq!(done_src, Some(3 * MIB), "without checksums the source stays");
        }
    }
}

/// Stopped after the finished file was put in place but before its info
/// file was deleted: a copy continued with "Resume" skips the file (it is
/// complete) and deletes the info file nothing refers to any more.
#[tokio::test]
async fn an_info_file_left_next_to_a_finished_file_is_cleaned_up() {
    let (src, dst) = pair(true);
    src.put("/src/f.bin", Node::Pattern(3 * MIB));
    dst.put("/dst/f.bin", Node::Pattern(3 * MIB));
    dst.put("/dst/f.bin.filepart-info", Node::Bytes(b"rolidscp-filepart 1\n".to_vec()));
    let seen = Seen::default();
    copy(&src, &dst, &["f.bin"], Conflict::Resume, Arc::new(AtomicBool::new(false)), &seen).await.expect("copy");
    assert_eq!(dst.size("/dst/f.bin"), Some(3 * MIB));
    assert_eq!(dst.size("/dst/f.bin.filepart-info"), None, "the leftover info file is gone");
    assert_eq!(dst.written(), 0, "the complete file is not written again");
}

#[test]
fn the_target_notices_a_wrong_byte() {
    let mut a = vec![0u8; 100];
    fill(5, &mut a);
    let mut b = vec![0u8; 100];
    fill(6, &mut b);
    assert_ne!(a, b);
    // Unaligned starts give the same bytes as aligned ones.
    let mut whole = vec![0u8; 64];
    fill(0, &mut whole);
    let mut part = vec![0u8; 51];
    fill(13, &mut part);
    assert_eq!(&whole[13..], &part[..]);
}

/// The whole file through the copy code, at the speed of memory: counters,
/// the remaining-time estimate, the speed. `ROLIDSCP_SYNTH_GIB` sets the size.
#[test]
#[ignore]
fn terabyte_through_the_copy_code() {
    let gib: u64 = std::env::var("ROLIDSCP_SYNTH_GIB").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    let size = gib * GIB;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (src, dst) = pair(true);
    src.put("/src/t.bin", Node::Pattern(size));
    let seen = Arc::new(Seen::default());
    let stop = Arc::new(AtomicBool::new(false));
    // What the transfer queue shows, once a second.
    let (s2, stop2) = (seen.clone(), stop.clone());
    let sampler = std::thread::spawn(move || {
        let mut win = rolid_core::eta::Window::default();
        let t0 = Instant::now();
        let mut worst: f64 = 0.0;
        let mut samples = 0u64;
        while !stop2.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let done = s2.done.load(Ordering::Relaxed);
            win.sample(t0.elapsed(), done);
            if let (rolid_core::eta::Eta::Secs(est), Some(speed)) = (win.eta(size - done), win.speed()) {
                // Against what the steady speed of the window says.
                let real = (size - done) as f64 / speed;
                worst = worst.max((est as f64 - real).abs());
                samples += 1;
            }
        }
        (worst, samples)
    });
    let t = Instant::now();
    rt.block_on(copy(&src, &dst, &["t.bin"], Conflict::Overwrite, Arc::new(AtomicBool::new(false)), &seen)).expect("copy");
    let secs = t.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let (worst, samples) = sampler.join().unwrap();
    println!("BENCH\tsynth.size\t{size}\tB");
    println!("BENCH\tsynth.seconds\t{secs:.1}\ts");
    println!("BENCH\tsynth.rate\t{:.0}\tMB/s", size as f64 / secs / 1e6);
    println!("BENCH\tsynth.wrong_bytes\t{}\tB", dst.wrong());
    println!("BENCH\tsynth.written\t{}\tB", dst.written());
    println!("BENCH\tsynth.progress_calls\t{}\tcalls", seen.calls.load(Ordering::Relaxed));
    println!("BENCH\tsynth.counter_backwards\t{}\ttimes", seen.backwards.load(Ordering::Relaxed));
    println!("BENCH\tsynth.eta_samples\t{samples}\tsamples");
    println!("BENCH\tsynth.eta_rounding_worst\t{worst:.1}\ts");
    assert_eq!(dst.size("/dst/t.bin"), Some(size));
    assert_eq!(dst.wrong(), 0);
    assert_eq!(seen.done.load(Ordering::Relaxed), size);
}
