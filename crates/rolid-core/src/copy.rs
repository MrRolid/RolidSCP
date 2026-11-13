//! Recursive copy between any two `Vfs` backends, with progress reporting,
//! conflict policies and safe partial files (`name.filepart`, like WinSCP).
//!
//! A file is first written to `name.filepart` and renamed over the target only
//! when complete, so an interrupted transfer never leaves a truncated file under
//! the real name. With [`Conflict::Resume`] an existing `.filepart` is continued.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use rolid_text::{txt, txtn, Text};

use crate::rate::RateLimit;
use crate::vfs::{validate_name, Entry, EntryKind, Vfs, VfsError, VfsResult};

const BUF_SIZE: usize = 256 * 1024;
pub const PARTIAL_SUFFIX: &str = ".filepart";

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
    /// Moving: files really written to the target (source, target, size).
    moved_files: Vec<(String, String, u64)>,
    /// Moving: source folders whose contents were copied (children first).
    moved_dirs: Vec<String>,
}

/// What deleting the sources after a move left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveCleanup {
    /// Source files deleted.
    pub deleted: usize,
    /// Source files kept: skipped by the conflict choice, changed meanwhile or
    /// not found in the target with the same size.
    pub kept: Vec<String>,
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
}

enum Plan {
    Skip,
    /// Already copied by an earlier run of this job.
    Done,
    /// Write `.filepart` from this offset, then rename over the target.
    Write {
        offset: u64,
    },
    /// Append directly into the existing (shorter) target from this offset.
    AppendTarget {
        offset: u64,
    },
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
        }
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
    /// with the size it had when copied (and still has in the source), then the
    /// source folders that became empty. Nothing else is deleted.
    pub async fn delete_moved_sources(&mut self) -> MoveCleanup {
        let mut out = MoveCleanup::default();
        for (src, dst, size) in std::mem::take(&mut self.state.moved_files) {
            let same_dst = self.dst.stat(&dst).await.is_ok_and(|e| e.size == size);
            let same_src = self.src.stat(&src).await.is_ok_and(|e| e.size == size);
            if same_dst && same_src && self.src.remove_file(&src).await.is_ok() {
                out.deleted += 1;
            } else {
                out.kept.push(src);
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

    async fn plan(&self, src: &Entry, dst_path: &str, part_path: &str) -> Plan {
        if self.state.completed.contains(dst_path) {
            return Plan::Done;
        }
        let target = self.dst.stat(dst_path).await.ok();
        let partial = self.dst.stat(part_path).await.ok();
        // Our own interrupted file: continue it, whatever the conflict policy.
        if self.state.retrying && self.state.in_flight.as_deref() == Some(dst_path) {
            if let Some(p) = partial.as_ref().filter(|p| p.size <= src.size) {
                return Plan::Write { offset: self.resume_offset(p.size) };
            }
        }
        match (self.conflict, target) {
            (Conflict::Resume, _) if partial.as_ref().is_some_and(|p| p.size <= src.size) => {
                Plan::Write {
                    offset: self.resume_offset(partial.map(|p| p.size).unwrap_or(0)),
                }
            }
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
            (Conflict::Resume, Some(t)) if t.kind == EntryKind::File && t.size < src.size => {
                Plan::AppendTarget { offset: self.resume_offset(t.size) }
            }
            (Conflict::Resume, Some(_)) => Plan::Write { offset: 0 },
        }
    }

    async fn copy_file(&mut self, src_path: &str, dst_path: &str, src: &Entry) -> VfsResult<()> {
        self.check_cancel()?;
        self.progress.current = src_path.to_owned();
        (self.on_progress)(&self.progress);

        let part_path = format!("{dst_path}{PARTIAL_SUFFIX}");
        let (write_path, offset, rename) = match self.plan(src, dst_path, &part_path).await {
            Plan::Done => return Ok(()),
            Plan::Skip => {
                self.progress.files_skipped += 1;
                self.state.completed.insert(dst_path.to_owned());
                (self.on_progress)(&self.progress);
                return Ok(());
            }
            Plan::Write { offset } => (part_path.as_str(), offset, true),
            Plan::AppendTarget { offset } => (dst_path, offset, false),
        };

        self.state.in_flight = Some(dst_path.to_owned());
        let mut r = self.src.open_read_at(src_path, offset).await?;
        let mut w = self.dst.open_write_at(write_path, offset).await?;
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

        if rename {
            // SFTPv3 rename does not overwrite: remove the old target first.
            if self.dst.stat(dst_path).await.is_ok() {
                self.dst.remove_file(dst_path).await?;
            }
            self.dst.rename(&part_path, dst_path).await?;
        }
        self.state.completed.insert(dst_path.to_owned());
        self.state.in_flight = None;
        if self.moving {
            self.state.moved_files.push((src_path.to_owned(), dst_path.to_owned(), src.size));
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

fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
