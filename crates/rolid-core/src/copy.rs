//! Recursive copy between any two `Vfs` backends, with progress reporting,
//! conflict policies and safe partial files (`name.filepart`, like WinSCP).
//!
//! A file is first written to `name.filepart` and put in place of the target
//! only when complete ([`crate::replace::safe_replace`]: atomically where the
//! backend can), so an interrupted transfer never leaves a truncated file under
//! the real name.
//!
//! With [`Conflict::Resume`] a `.filepart` is continued only when it is known to
//! belong to the same source: a big file gets `name.filepart-info` next to it
//! (source path, size and modification time). A part file that does not match,
//! or cannot be checked, is not continued; it is left alone and reported (the
//! user can start over with "Overwrite").

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use rolid_text::{txt, txtn, Text};

use crate::rate::RateLimit;
use crate::vfs::{validate_name, Entry, EntryKind, Vfs, VfsError, VfsResult};

const BUF_SIZE: usize = 256 * 1024;
pub const PARTIAL_SUFFIX: &str = ".filepart";
/// Next to a `.filepart`: which source it belongs to (see the module docs).
pub const PARTIAL_INFO_SUFFIX: &str = ".filepart-info";
/// Files from this size get a [`PARTIAL_INFO_SUFFIX`] file and can be resumed
/// after the program was restarted. A smaller part file without one is simply
/// written again from the start.
pub const RESUME_INFO_MIN: u64 = 1024 * 1024;
const INFO_HEADER: &str = "rolidscp-filepart 1";

/// When continuing a `.filepart` on a remote destination, start this much before
/// its end. Writes are pipelined (many in flight); a server that does not apply
/// them strictly in order could leave a hole just before the end when the
/// connection breaks. Rewriting the tail makes the result correct either way.
pub const RESUME_MARGIN: u64 = 4 * 1024 * 1024;

/// How many times one transfer is continued automatically after the connection
/// was lost (then it fails and can be retried by hand).
pub const MAX_AUTO_RETRIES: u32 = 5;

/// What a copy job already did – kept across retries, so a repeated run skips
/// finished files and continues the interrupted one from its `.filepart`.
#[derive(Debug, Clone, Default)]
pub struct CopyState {
    completed: std::collections::HashSet<String>,
    in_flight: Option<String>,
    /// The next run continues after an interruption.
    retrying: bool,
    /// Moving: files really written to the target.
    moved_files: Vec<Moved>,
    /// Moving: source folders whose contents were copied (children first).
    moved_dirs: Vec<String>,
}

/// A file a move wrote to the target, with the source as it was when read.
#[derive(Debug, Clone)]
struct Moved {
    src: String,
    dst: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

/// What deleting the sources after a move left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveCleanup {
    /// Source files deleted.
    pub deleted: usize,
    /// Source files kept: skipped by the conflict choice, or not found in the
    /// target with the same size.
    pub kept: Vec<String>,
    /// Source files kept because they changed while they were being moved
    /// (another size or modification time than when they were read).
    pub changed: Vec<String>,
    /// Source files kept because the checksum of the copy differs from the source.
    pub mismatch: Vec<String>,
    /// Files whose copy was compared by checksum before the source was deleted.
    pub verified: usize,
    /// Source folders kept because something stayed in them.
    pub kept_dirs: usize,
}

impl CopyState {
    /// Destination paths finished (or skipped) so far.
    pub fn completed(&self) -> usize {
        self.completed.len()
    }
}

/// What to do when the destination file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conflict {
    #[default]
    Overwrite,
    Skip,
    /// Overwrite only if the source is newer (by modification time).
    Newer,
    /// Continue interrupted transfers (`.filepart`, or a shorter target);
    /// identical-size targets are skipped, anything else is overwritten.
    Resume,
}

/// A file or folder the job could not copy (in [`CopyJob::keep_going`] mode the
/// job continues without it and reports it at the end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub path: String,
    pub error: Text,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub current: String,
    pub files_done: u64,
    pub files_skipped: u64,
    pub bytes_done: u64,
    /// Bytes not transferred because they were already present (resume).
    pub bytes_resumed: u64,
    pub started: Option<Instant>,
    /// Extracting / testing an archive: bytes of the archive read and its size
    /// (the progress, speed and remaining time go by these).
    pub read: Option<(u64, u64)>,
}

/// How much a copy job has in all (counted in the background while it runs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub files: u64,
    pub bytes: u64,
    /// Counting finished (otherwise the numbers are "at least").
    pub done: bool,
}

impl Totals {
    /// "file 3 of 120" for the status line. While counting: "file 3 of 120+".
    pub fn file_text(&self, p: &Progress) -> Text {
        let seen = p.files_done + p.files_skipped;
        if self.files == 0 {
            return if self.done { txtn!("{n} file", "{n} files", seen) } else { txt!("file {num}", num = seen + 1) };
        }
        let n = (seen + 1).min(self.files.max(seen));
        let total = self.files.max(seen);
        if self.done {
            txt!("file {num} of {total}", num = n, total = total)
        } else {
            txt!("file {num} of {total}+", num = n, total = total)
        }
    }
}

/// Count files and bytes of `items` (`(dir, name)`) like the copy walks them:
/// directory symlinks are not followed, special files are skipped, unreadable
/// folders count as empty. `update` gets the running totals.
pub async fn measure(
    fs: &dyn Vfs,
    items: &[(String, String)],
    stop: &AtomicBool,
    update: &(dyn Fn(Totals) + Send + Sync),
) -> Totals {
    let mut t = Totals::default();
    for (dir, name) in items {
        let path = fs.join(dir, name);
        let Ok(e) = fs.stat(&path).await else { continue };
        if e.is_dir_like() {
            if measure_dir(fs, &path, stop, &mut t, update).await.is_err() {
                return t;
            }
        } else {
            t.files += 1;
            t.bytes += e.size;
        }
        update(t);
    }
    t.done = !stop.load(Ordering::Relaxed);
    update(t);
    t
}

fn measure_dir<'a>(
    fs: &'a dyn Vfs,
    dir: &'a str,
    stop: &'a AtomicBool,
    t: &'a mut Totals,
    update: &'a (dyn Fn(Totals) + Send + Sync),
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ()>> + Send + 'a>> {
    Box::pin(async move {
        if stop.load(Ordering::Relaxed) {
            return Err(());
        }
        let Ok(entries) = fs.list(dir).await else { return Ok(()) };
        for e in entries {
            match e.kind {
                EntryKind::Dir => measure_dir(fs, &fs.join(dir, &e.name), stop, t, update).await?,
                EntryKind::Symlink if e.link_to_dir => {}
                EntryKind::Other => {}
                _ => {
                    t.files += 1;
                    t.bytes += e.size;
                }
            }
        }
        update(*t);
        Ok(())
    })
}

impl Progress {
    /// Transfer rate in bytes per second (excluding resumed bytes).
    pub fn rate(&self) -> f64 {
        match self.started {
            Some(t) if t.elapsed().as_secs_f64() > 0.2 => {
                self.bytes_done as f64 / t.elapsed().as_secs_f64()
            }
            _ => 0.0,
        }
    }
}

pub struct CopyJob<'a> {
    pub src: &'a dyn Vfs,
    pub dst: &'a dyn Vfs,
    pub cancel: Arc<AtomicBool>,
    pub on_progress: &'a (dyn Fn(&Progress) + Send + Sync),
    pub conflict: Conflict,
    /// Shared bandwidth limit (all transfers together).
    pub limit: Option<Arc<RateLimit>>,
    progress: Progress,
    state: CopyState,
    keep_going: bool,
    failures: Vec<Failure>,
    /// Moving (F6 between disks): remember what was written, for deleting the sources.
    moving: bool,
    /// Name of the (single) item in the target, when it differs.
    target_name: Option<String>,
    /// Moving: compare source and copy by checksum before deleting the source.
    verify: bool,
    /// Part files not continued because they could not be matched to their source.
    not_resumed: Vec<String>,
}

enum Plan {
    Skip,
    /// Already copied by an earlier run of this job.
    Done,
    /// Write `.filepart` from this offset, then put it in place of the target.
    Write {
        offset: u64,
    },
    /// Resume was asked for, but the `.filepart` cannot be matched to this
    /// source: leave it alone and report it.
    NotResumed,
}

impl<'a> CopyJob<'a> {
    pub fn new(
        src: &'a dyn Vfs,
        dst: &'a dyn Vfs,
        cancel: Arc<AtomicBool>,
        on_progress: &'a (dyn Fn(&Progress) + Send + Sync),
    ) -> Self {
        Self {
            src,
            dst,
            cancel,
            on_progress,
            conflict: Conflict::default(),
            limit: None,
            progress: Progress {
                started: Some(Instant::now()),
                ..Default::default()
            },
            state: CopyState::default(),
            keep_going: false,
            failures: Vec::new(),
            moving: false,
            target_name: None,
            verify: false,
            not_resumed: Vec::new(),
        }
    }

    /// Moving: compare each copy with its source by checksum (reading both
    /// again) before the source is deleted.
    pub fn verify_moved(mut self, on: bool) -> Self {
        self.verify = on;
        self
    }

    /// Part files (`.filepart`) that were not continued because they could not
    /// be matched to their source (target paths).
    pub fn not_resumed(&self) -> &[String] {
        &self.not_resumed
    }

    /// A move: the job remembers what it really wrote, so that
    /// [`CopyJob::delete_moved_sources`] deletes only those sources afterwards.
    /// Symbolic links are not moved (they stay in the source).
    pub fn moving(mut self) -> Self {
        self.moving = true;
        self
    }

    /// The copied item gets this name in the target (only for a single item).
    pub fn with_target_name(mut self, name: Option<String>) -> Self {
        self.target_name = name;
        self
    }

    /// After a successful move: delete each source file that is in the target
    /// with its size and still has the size and modification time it had when
    /// it was read (and, with [`CopyJob::verify_moved`], the same checksum as
    /// the copy); then the source folders that became empty. Nothing else is
    /// deleted.
    pub async fn delete_moved_sources(&mut self) -> MoveCleanup {
        let mut out = MoveCleanup::default();
        for m in std::mem::take(&mut self.state.moved_files) {
            let same_dst = self.dst.stat(&m.dst).await.is_ok_and(|e| e.size == m.size);
            let src_now = self.src.stat(&m.src).await;
            let Ok(src_now) = src_now else {
                out.kept.push(m.src);
                continue;
            };
            if src_now.size != m.size || src_now.modified != m.modified {
                out.changed.push(m.src);
                continue;
            }
            if !same_dst {
                out.kept.push(m.src);
                continue;
            }
            if self.verify {
                self.progress.current = m.src.clone();
                (self.on_progress)(&self.progress);
                match (checksum(self.src, &m.src, &self.cancel).await, checksum(self.dst, &m.dst, &self.cancel).await) {
                    (Ok(a), Ok(b)) if a == b => out.verified += 1,
                    (Ok(_), Ok(_)) => {
                        out.mismatch.push(m.src);
                        continue;
                    }
                    _ => {
                        out.kept.push(m.src);
                        continue;
                    }
                }
            }
            if self.src.remove_file(&m.src).await.is_ok() {
                out.deleted += 1;
            } else {
                out.kept.push(m.src);
            }
        }
        for dir in std::mem::take(&mut self.state.moved_dirs) {
            if self.src.remove_dir(&dir).await.is_err() {
                out.kept_dirs += 1;
            }
        }
        out
    }

    /// Do not stop at a file that cannot be copied (no permission to read it, an
    /// invalid name…): record it in [`CopyJob::failures`] and go on. A lost
    /// connection and canceling still stop the job.
    pub fn keep_going(mut self) -> Self {
        self.keep_going = true;
        self
    }

    /// What could not be copied (only in [`CopyJob::keep_going`] mode).
    pub fn failures(&self) -> &[Failure] {
        &self.failures
    }

    /// In keep-going mode a per-item error is recorded and swallowed; anything
    /// else (normal mode, lost connection, cancel) is passed on.
    fn tolerate(&mut self, path: &str, r: VfsResult<()>) -> VfsResult<()> {
        match r {
            Err(e) if self.keep_going && !e.is_connection_lost() && !self.cancel.load(Ordering::Relaxed) => {
                if !self.failures.iter().any(|f| f.path == path) {
                    let mut error = e.text();
                    if path.contains('\u{FFFD}') {
                        error = txt!("{error} (the name is not valid UTF-8, it cannot be transferred)", error = error);
                    }
                    self.failures.push(Failure { path: path.to_owned(), error });
                }
                Ok(())
            }
            r => r,
        }
    }

    /// Continue an interrupted job: finished files are skipped, the file that was
    /// being written continues from its `.filepart`.
    pub fn with_state(mut self, mut state: CopyState) -> Self {
        state.retrying = true;
        self.state = state;
        self
    }

    pub fn state(&self) -> &CopyState {
        &self.state
    }

    /// Prepare the next run after an interruption (connection lost).
    pub fn prepare_retry(&mut self) {
        self.state.retrying = true;
    }

    /// Copy `items` (`(source dir, name)`) into `dst_dir`. When the connection is
    /// lost, `wait_reconnect(n)` is awaited (n = 1, 2, … up to `max_retries`); if it
    /// returns true the job continues where it stopped, otherwise it fails.
    pub async fn copy_items_retrying<F, Fut>(
        &mut self,
        items: &[(String, String)],
        dst_dir: &str,
        max_retries: u32,
        mut wait_reconnect: F,
    ) -> VfsResult<()>
    where
        F: FnMut(u32) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let mut retries = 0;
        let mut i = 0;
        while i < items.len() {
            let (dir, name) = &items[i];
            let r = self.copy_entry(dir, name, dst_dir).await;
            let r = self.tolerate(&self.src.join(dir, name), r);
            match r {
                Ok(()) => i += 1,
                Err(e) if e.is_connection_lost() && !self.cancel.load(Ordering::Relaxed) && max_retries > 0 => {
                    if retries >= max_retries {
                        return Err(VfsError::ConnectionLost(txtn!(
                            "{error} (the transfer gave up after {n} attempt – try it again manually)",
                            "{error} (the transfer gave up after {n} attempts – try it again manually)",
                            max_retries,
                            error = e.text()
                        )));
                    }
                    retries += 1;
                    if !wait_reconnect(retries).await {
                        return Err(e);
                    }
                    self.prepare_retry();
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn with_conflict(mut self, c: Conflict) -> Self {
        self.conflict = c;
        self
    }

    pub fn with_limit(mut self, limit: Arc<RateLimit>) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    fn check_cancel(&self) -> VfsResult<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(VfsError::canceled())
        } else {
            Ok(())
        }
    }

    /// Copy entry `name` from directory `src_dir` into directory `dst_dir`.
    pub async fn copy_entry(&mut self, src_dir: &str, name: &str, dst_dir: &str) -> VfsResult<()> {
        validate_name(name)?;
        let src_path = self.src.join(src_dir, name);
        let dst_name = self.target_name.clone().unwrap_or_else(|| name.to_owned());
        validate_name(&dst_name)?;
        let dst_path = self.dst.join(dst_dir, &dst_name);
        let entry = self.src.stat(&src_path).await?;
        if self.moving && entry.kind == EntryKind::Symlink {
            return Err(VfsError::Backend(txt!(
                "“{name}” is a symbolic link: it cannot be moved between disks (copy it and delete it manually)",
                name = name
            )));
        }
        if entry.is_dir_like() {
            self.copy_dir(&src_path, &dst_path, &entry).await
        } else {
            self.copy_file(&src_path, &dst_path, &entry).await
        }
    }

    fn copy_dir<'b>(
        &'b mut self,
        src_path: &'b str,
        dst_path: &'b str,
        entry: &'b Entry,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = VfsResult<()>> + Send + 'b>>
    where
        'a: 'b,
    {
        Box::pin(async move {
            self.check_cancel()?;
            // Read the source first: an unreadable folder leaves nothing behind
            // (no empty copy with its permissions).
            let entries = match self.src.list(src_path).await {
                Ok(v) => v,
                Err(e) => return self.tolerate(src_path, Err(e)),
            };
            if self.dst.stat(dst_path).await.is_err() {
                if let Err(e) = self.dst.mkdir(dst_path).await {
                    // Without the folder its contents cannot be copied either.
                    return self.tolerate(src_path, Err(e));
                }
                if let Some(m) = entry.mode {
                    let _ = self.dst.set_attrs(dst_path, Some(m & 0o777), None).await;
                }
            }
            for e in entries {
                let s = self.src.join(src_path, &e.name);
                if let Err(err) = validate_name(&e.name) {
                    self.tolerate(&s, Err(err))?;
                    continue;
                }
                let d = self.dst.join(dst_path, &e.name);
                let r = match e.kind {
                    EntryKind::Dir => self.copy_dir(&s, &d, &e).await,
                    // A move leaves links where they are (the folder stays too).
                    EntryKind::Symlink if self.moving => continue,
                    // Do not follow directory symlinks recursively (loops, escaping the tree).
                    EntryKind::Symlink if e.link_to_dir => continue,
                    EntryKind::Other => continue,
                    _ => self.copy_file(&s, &d, &e).await,
                };
                self.tolerate(&s, r)?;
            }
            if self.moving && !self.state.moved_dirs.iter().any(|d| d == src_path) {
                self.state.moved_dirs.push(src_path.to_owned());
            }
            Ok(())
        })
    }

    async fn plan(&self, src_path: &str, src: &Entry, dst_path: &str, part_path: &str) -> Plan {
        if self.state.completed.contains(dst_path) {
            return Plan::Done;
        }
        let target = self.dst.stat(dst_path).await.ok();
        let partial = self.dst.stat(part_path).await.ok();
        // Our own interrupted file (this job, this run of the program): continue
        // it, whatever the conflict policy.
        if self.state.retrying && self.state.in_flight.as_deref() == Some(dst_path) {
            if let Some(p) = partial.as_ref().filter(|p| p.size <= src.size) {
                return Plan::Write { offset: self.resume_offset(p.size) };
            }
        }
        if let (Conflict::Resume, Some(p)) = (self.conflict, partial.as_ref()) {
            let info = read_info(self.dst, &info_path(dst_path)).await;
            return match info {
                Some(i) if i.matches(src_path, src) && p.size <= src.size => Plan::Write { offset: self.resume_offset(p.size) },
                None if src.size < RESUME_INFO_MIN => Plan::Write { offset: 0 },
                _ => Plan::NotResumed,
            };
        }
        match (self.conflict, target) {
            (_, None) => Plan::Write { offset: 0 },
            (Conflict::Overwrite, Some(_)) => Plan::Write { offset: 0 },
            (Conflict::Skip, Some(_)) => Plan::Skip,
            (Conflict::Newer, Some(t)) => match (src.modified, t.modified) {
                // Filesystems differ in timestamp precision: compare whole seconds.
                (Some(s), Some(d)) if secs(s) > secs(d) => Plan::Write { offset: 0 },
                (Some(_), Some(_)) => Plan::Skip,
                _ => Plan::Write { offset: 0 },
            },
            (Conflict::Resume, Some(t)) if t.kind == EntryKind::File && t.size == src.size => {
                Plan::Skip
            }
            // Never written into under the real name: a different (also a
            // shorter) target is replaced through a new .filepart.
            (Conflict::Resume, Some(_)) => Plan::Write { offset: 0 },
        }
    }

    async fn copy_file(&mut self, src_path: &str, dst_path: &str, src: &Entry) -> VfsResult<()> {
        self.check_cancel()?;
        self.progress.current = src_path.to_owned();
        (self.on_progress)(&self.progress);

        let part_path = format!("{dst_path}{PARTIAL_SUFFIX}");
        let offset = match self.plan(src_path, src, dst_path, &part_path).await {
            Plan::Done => return Ok(()),
            Plan::Skip => {
                self.progress.files_skipped += 1;
                self.state.completed.insert(dst_path.to_owned());
                (self.on_progress)(&self.progress);
                return Ok(());
            }
            Plan::NotResumed => {
                self.not_resumed.push(dst_path.to_owned());
                self.progress.files_skipped += 1;
                self.state.completed.insert(dst_path.to_owned());
                (self.on_progress)(&self.progress);
                return Ok(());
            }
            Plan::Write { offset } => offset,
        };

        self.state.in_flight = Some(dst_path.to_owned());
        let info = info_path(dst_path);
        if offset == 0 && src.size >= RESUME_INFO_MIN {
            // Which source this part file belongs to (for resuming it later).
            // Best effort: without it the file is just not resumable.
            let _ = write_info(self.dst, &info, &PartInfo::of(src_path, src)).await;
        }
        let mut r = self.src.open_read_at(src_path, offset).await?;
        let mut w = self.dst.open_write_at(&part_path, offset).await?;
        self.progress.bytes_resumed += offset;
        let mut buf = vec![0u8; BUF_SIZE];
        let copied: VfsResult<()> = async {
            loop {
                self.check_cancel()?;
                let chunk = self.limit.as_ref().map_or(BUF_SIZE, |l| l.chunk(BUF_SIZE));
                let n = r.read(&mut buf[..chunk]).await?;
                if n == 0 {
                    return Ok(());
                }
                w.write_all(&buf[..n]).await?;
                self.progress.bytes_done += n as u64;
                (self.on_progress)(&self.progress);
                if let Some(l) = &self.limit {
                    l.consume(n as u64).await;
                }
            }
        }
        .await;
        if let Err(e) = copied {
            // Let writes still in flight land (tokio::fs::File writes in the
            // background): a retry measures the .filepart to resume from it.
            let _ = w.flush().await;
            return Err(e);
        }
        w.shutdown().await?;
        drop(w);
        drop(r);

        // In place of the target: atomically where possible, never deleting
        // the old target before the new one is there.
        crate::replace::safe_replace(self.dst, &part_path, dst_path).await?;
        if self.dst.stat(&info).await.is_ok() {
            let _ = self.dst.remove_file(&info).await;
        }
        self.state.completed.insert(dst_path.to_owned());
        self.state.in_flight = None;
        if self.moving {
            self.state.moved_files.push(Moved {
                src: src_path.to_owned(),
                dst: dst_path.to_owned(),
                size: src.size,
                modified: src.modified,
            });
        }
        // Keep permission bits (without setuid/setgid/sticky) and modification time.
        let _ = self
            .dst
            .set_attrs(dst_path, src.mode.map(|m| m & 0o777), src.modified)
            .await;
        self.progress.files_done += 1;
        (self.on_progress)(&self.progress);
        Ok(())
    }
}

impl CopyJob<'_> {
    /// Where to continue a partial file of `size` bytes (see [`RESUME_MARGIN`]).
    fn resume_offset(&self, size: u64) -> u64 {
        if self.dst.is_remote() {
            size.saturating_sub(RESUME_MARGIN)
        } else {
            size
        }
    }
}

/// `name.filepart-info` for the target `dst_path`.
pub fn info_path(dst_path: &str) -> String {
    format!("{dst_path}{PARTIAL_INFO_SUFFIX}")
}

/// Which source a `.filepart` belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PartInfo {
    src: String,
    size: u64,
    mtime: Option<u64>,
}

impl PartInfo {
    fn of(src_path: &str, e: &Entry) -> PartInfo {
        PartInfo { src: src_path.to_owned(), size: e.size, mtime: e.modified.map(secs) }
    }
    fn matches(&self, src_path: &str, e: &Entry) -> bool {
        *self == PartInfo::of(src_path, e)
    }
    fn encode(&self) -> String {
        let path: String = self.src.bytes().map(|b| format!("{b:02x}")).collect();
        let mtime = self.mtime.map_or("-".to_string(), |t| t.to_string());
        format!("{INFO_HEADER}\n{path}\n{}\n{mtime}\n", self.size)
    }
    fn decode(text: &str) -> Option<PartInfo> {
        let mut l = text.lines();
        if l.next()? != INFO_HEADER {
            return None;
        }
        let hex = l.next()?;
        if hex.len() % 2 != 0 {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
        let src = String::from_utf8(bytes?).ok()?;
        let size = l.next()?.parse().ok()?;
        let mtime = match l.next()? {
            "-" => None,
            t => Some(t.parse().ok()?),
        };
        Some(PartInfo { src, size, mtime })
    }
}

async fn write_info(fs: &dyn Vfs, path: &str, info: &PartInfo) -> VfsResult<()> {
    let mut w = fs.open_write(path).await?;
    w.write_all(info.encode().as_bytes()).await?;
    w.shutdown().await?;
    Ok(())
}

async fn read_info(fs: &dyn Vfs, path: &str) -> Option<PartInfo> {
    let mut r = fs.open_read(path).await.ok()?;
    let mut buf = Vec::new();
    (&mut r).take(64 * 1024).read_to_end(&mut buf).await.ok()?;
    PartInfo::decode(std::str::from_utf8(&buf).ok()?)
}

/// BLAKE3 of a whole file, read through `fs`.
pub async fn checksum(fs: &dyn Vfs, path: &str, cancel: &AtomicBool) -> VfsResult<[u8; 32]> {
    let mut r = fs.open_read(path).await?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(VfsError::canceled());
        }
        let n = r.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(*h.finalize().as_bytes())
}

fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_info_round_trip() {
        let i = PartInfo { src: "/srv/a b/ž\n.bin".into(), size: 5_000_000, mtime: Some(1_700_000_000) };
        assert_eq!(PartInfo::decode(&i.encode()), Some(i.clone()));
        let j = PartInfo { mtime: None, ..i };
        assert_eq!(PartInfo::decode(&j.encode()), Some(j));
        assert_eq!(PartInfo::decode("something else"), None);
    }

    #[test]
    fn file_counter_text() {
        let mut p = Progress::default();
        let mut t = Totals { files: 120, bytes: 0, done: true };
        // The first file is being copied: "1 of 120", not "0 files".
        assert_eq!(t.file_text(&p).to_string(), "file 1 of 120");
        p.files_done = 2;
        assert_eq!(t.file_text(&p).to_string(), "file 3 of 120");
        p.files_skipped = 1;
        assert_eq!(t.file_text(&p).to_string(), "file 4 of 120");
        p.files_done = 119;
        assert_eq!(t.file_text(&p).to_string(), "file 120 of 120");
        t.done = false;
        p = Progress { files_done: 4, ..Default::default() };
        assert_eq!(t.file_text(&p).to_string(), "file 5 of 120+");
        let empty = Totals { files: 0, bytes: 0, done: false };
        assert_eq!(empty.file_text(&p).to_string(), "file 5");
        let done_empty = Totals { files: 0, bytes: 0, done: true };
        assert_eq!(done_empty.file_text(&Progress::default()).to_string(), "0 files");
    }
}
