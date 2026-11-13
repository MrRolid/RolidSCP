//! A file open in the viewer: read in blocks by a background task, never
//! whole. The UI asks for windows of bytes without waiting (missing blocks
//! are requested and the UI is repainted when they come); the line index is
//! built by another task. The file is watched for changes (size, time);
//! after a lost connection reading continues by itself once it is back.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rolid_core::vfs::{open_random, RandomFile};
use rolid_core::{Vfs, VfsError};
use rolid_text::Text;

use crate::cache::BlockCache;
use crate::enc::{detect, Detected, Enc};
use crate::index::{skip_lines, LineIndex, LineNo, MARK};

/// How the file is read.
#[derive(Debug, Clone, Copy)]
pub struct Opts {
    /// Block size of the cache.
    pub block: usize,
    /// Most memory for the cached blocks of this file.
    pub cache_limit: usize,
    /// Blocks read ahead after the last wanted one.
    pub readahead: u64,
    /// Bytes read at once by the line index.
    pub index_chunk: usize,
    /// The line index reads at most this much (on a server the whole file
    /// would have to be downloaded): past it line numbers are estimates.
    pub index_limit: u64,
    /// How often the file is checked for changes.
    pub poll: Duration,
    /// Pause before trying again after an error.
    pub retry: Duration,
}

impl Opts {
    pub fn local() -> Self {
        Opts {
            block: 256 << 10,
            cache_limit: 128 << 20,
            readahead: 2,
            index_chunk: 4 << 20,
            index_limit: u64::MAX,
            poll: Duration::from_millis(1500),
            retry: Duration::from_secs(2),
        }
    }

    pub fn remote() -> Self {
        Opts { block: 512 << 10, index_chunk: 1 << 20, index_limit: 64 << 20, poll: Duration::from_secs(3), ..Self::local() }
    }
}

/// Can the file be read?
#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    Ok,
    /// The connection is lost: reading continues when it is back.
    Lost(Text),
    /// Reading failed (tried again after a pause or on reload).
    Failed(Text),
}

/// The file changed on the disk / server while it was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// How far the line index is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexState {
    pub scanned: u64,
    pub done: bool,
    /// Stopped at [`Opts::index_limit`].
    pub limited: bool,
    pub longest: u64,
    pub total: LineNo,
}

/// Where a line starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSeek {
    Found(u64),
    /// The data is being read: ask again.
    Pending,
    /// Past the indexed part: an estimate.
    About(u64),
}

struct State {
    size: u64,
    modified: Option<SystemTime>,
    cache: BlockCache,
    /// Wanted blocks, the most recent last.
    wanted: Vec<u64>,
    /// The block after the last wanted one (read-ahead starts there).
    ahead_from: Option<u64>,
    health: Health,
    changed: Option<Change>,
    follow: bool,
    /// Raised by a reload: data read before it is thrown away.
    generation: u64,
    detected: Option<Detected>,
    index: Option<LineIndex>,
    index_error: Option<Text>,
}

struct Inner {
    st: Mutex<State>,
    wake: tokio::sync::Notify,
    repaint: Arc<dyn Fn() + Send + Sync>,
    closed: AtomicBool,
    /// The line index waits (an image is shown: nothing to count).
    index_paused: AtomicBool,
    fs: Arc<dyn Vfs>,
    path: String,
    opts: Opts,
    /// Bytes read from the file (statistics).
    read_bytes: AtomicU64,
}

pub struct Doc {
    inner: Arc<Inner>,
}

impl Drop for Doc {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.wake.notify_waiters();
    }
}

impl Doc {
    /// Start reading `path` (of `size` bytes, as listed) on the runtime.
    pub fn open(
        rt: &tokio::runtime::Handle,
        fs: Arc<dyn Vfs>,
        path: String,
        size: u64,
        modified: Option<SystemTime>,
        opts: Opts,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Doc {
        let inner = Arc::new(Inner {
            st: Mutex::new(State {
                size,
                modified,
                cache: BlockCache::new(opts.block, opts.cache_limit),
                wanted: Vec::new(),
                ahead_from: None,
                health: Health::Ok,
                changed: None,
                follow: false,
                generation: 0,
                detected: None,
                index: None,
                index_error: None,
            }),
            wake: tokio::sync::Notify::new(),
            repaint,
            closed: AtomicBool::new(false),
            index_paused: AtomicBool::new(false),
            fs,
            path,
            opts,
            read_bytes: AtomicU64::new(0),
        });
        rt.spawn(reader(inner.clone()));
        rt.spawn(indexer(inner.clone()));
        // The first block (encoding detection) right away.
        inner.st.lock().unwrap().wanted.push(0);
        inner.wake.notify_one();
        Doc { inner }
    }

    pub fn fs(&self) -> &Arc<dyn Vfs> {
        &self.inner.fs
    }

    pub fn path(&self) -> &str {
        &self.inner.path
    }

    pub fn opts(&self) -> Opts {
        self.inner.opts
    }

    pub fn size(&self) -> u64 {
        self.inner.st.lock().unwrap().size
    }

    pub fn modified(&self) -> Option<SystemTime> {
        self.inner.st.lock().unwrap().modified
    }

    pub fn health(&self) -> Health {
        self.inner.st.lock().unwrap().health.clone()
    }

    pub fn changed(&self) -> Option<Change> {
        self.inner.st.lock().unwrap().changed
    }

    pub fn detected(&self) -> Option<Detected> {
        self.inner.st.lock().unwrap().detected
    }

    pub fn read_bytes(&self) -> u64 {
        self.inner.read_bytes.load(Ordering::Relaxed)
    }

    pub fn cached_bytes(&self) -> usize {
        self.inner.st.lock().unwrap().cache.bytes()
    }

    pub fn index_state(&self) -> Option<IndexState> {
        let st = self.inner.st.lock().unwrap();
        let limit = self.inner.opts.index_limit;
        st.index.as_ref().map(|ix| IndexState { scanned: ix.scanned(), done: ix.done, limited: !ix.done && ix.scanned() >= limit && ix.scanned() < st.size, longest: ix.longest, total: ix.total(st.size) })
    }

    pub fn index_error(&self) -> Option<Text> {
        self.inner.st.lock().unwrap().index_error.clone()
    }

    pub fn follow(&self) -> bool {
        self.inner.st.lock().unwrap().follow
    }

    /// Stop or go on building the line index (an image does not need it,
    /// and on a server it would download the file).
    pub fn pause_index(&self, on: bool) {
        self.inner.index_paused.store(on, Ordering::SeqCst);
        if !on {
            self.inner.wake.notify_waiters();
        }
    }

    /// Follow the end of the file (a growing log): growth is taken in
    /// without asking, and checked for more often.
    pub fn set_follow(&self, on: bool) {
        let mut st = self.inner.st.lock().unwrap();
        st.follow = on;
        if on {
            // A change noticed before is the growth being followed now.
            if let Some(c) = st.changed.take() {
                if c.size >= st.size {
                    grow(&mut st, c);
                } else {
                    st.changed = Some(c);
                }
            }
        }
        drop(st);
        self.inner.wake.notify_one();
    }

    /// The encoding the line index counts in (a different unit width starts
    /// it again).
    pub fn set_index_enc(&self, enc: Enc, start: u64) {
        let mut st = self.inner.st.lock().unwrap();
        let same = st.index.as_ref().is_some_and(|ix| ix.enc().unit() == enc.unit());
        if !same {
            st.index = Some(LineIndex::new(enc, start).keep_hint(st.index.as_ref()));
            st.index_error = None;
        }
        drop(st);
        self.inner.wake.notify_waiters();
    }

    /// Read the file again from the start (after a change, or an error).
    pub fn reload(&self) {
        let mut st = self.inner.st.lock().unwrap();
        st.generation += 1;
        st.cache.clear();
        if let Some(c) = st.changed.take() {
            st.size = c.size;
            st.modified = c.modified;
        }
        st.health = Health::Ok;
        st.wanted.clear();
        st.wanted.push(0);
        st.ahead_from = None;
        if let Some(ix) = &st.index {
            st.index = Some(LineIndex::new(ix.enc(), st.detected.map_or(0, |d| d.bom as u64)).keep_hint(Some(ix)));
        }
        st.index_error = None;
        drop(st);
        self.inner.wake.notify_waiters();
        self.inner.wake.notify_one();
    }

    /// The bytes `[from, from + len)` (cut at the end of the file) if they
    /// are cached; otherwise the missing blocks are requested and `None`
    /// is returned (never waits).
    pub fn window(&self, from: u64, len: usize) -> Option<Vec<u8>> {
        let mut st = self.inner.st.lock().unwrap();
        let r = window(&mut st, from, len);
        if r.is_none() {
            drop(st);
            self.inner.wake.notify_one();
        }
        r
    }

    /// Like [`Self::window`] but gives what is there from `from` on (maybe
    /// shorter than `len`, maybe empty); missing blocks are requested.
    pub fn window_partial(&self, from: u64, len: usize) -> Vec<u8> {
        let mut st = self.inner.st.lock().unwrap();
        let size = st.size;
        let end = (from + len as u64).min(size);
        let block = st.cache.block() as u64;
        let mut out = Vec::new();
        let mut missing = false;
        let mut at = from;
        while at < end {
            let idx = at / block;
            match st.cache.get(idx) {
                Some(b) if !missing => {
                    let s = (at - idx * block) as usize;
                    let e = ((end - idx * block) as usize).min(b.len());
                    if s >= e {
                        break;
                    }
                    out.extend_from_slice(&b[s..e]);
                }
                Some(_) => {}
                None => {
                    missing = true;
                    want(&mut st, idx);
                }
            }
            at = (idx + 1) * block;
        }
        if missing {
            drop(st);
            self.inner.wake.notify_one();
        }
        out
    }

    /// The number of the line containing `off` (exact in the indexed part
    /// when the bytes since the last mark are cached).
    pub fn line_at(&self, off: u64) -> LineNo {
        let mut st = self.inner.st.lock().unwrap();
        let Some(ix) = st.index.clone() else { return LineNo::About(1) };
        let mark = LineIndex::mark_before(off);
        let bytes = if off <= ix.scanned() { window(&mut st, mark, (off - mark) as usize) } else { None };
        ix.line_at(off, bytes.as_deref())
    }

    /// The start of line `n` (from 1).
    pub fn seek_line(&self, n: u64) -> LineSeek {
        let mut st = self.inner.st.lock().unwrap();
        let Some(ix) = st.index.clone() else { return LineSeek::Pending };
        match ix.seek_line(n) {
            Some((mark, skip)) => match window(&mut st, mark, MARK as usize) {
                Some(b) => match skip_lines(ix.enc(), &b, skip) {
                    Some(at) => LineSeek::Found(mark + at as u64),
                    None => LineSeek::About(ix.estimate_offset(n)),
                },
                None => {
                    drop(st);
                    self.inner.wake.notify_one();
                    LineSeek::Pending
                }
            },
            None if ix.done => LineSeek::Found(st.size),
            None => LineSeek::About(ix.estimate_offset(n).min(st.size)),
        }
    }
}

fn want(st: &mut State, idx: u64) {
    st.wanted.retain(|&w| w != idx);
    st.wanted.push(idx);
    // Keep the request list short: only recent requests matter.
    if st.wanted.len() > 64 {
        st.wanted.remove(0);
    }
}

fn window(st: &mut State, from: u64, len: usize) -> Option<Vec<u8>> {
    let size = st.size;
    let end = (from + len as u64).min(size);
    if from >= end {
        return Some(Vec::new());
    }
    let block = st.cache.block() as u64;
    let (first, last) = (from / block, (end - 1) / block);
    let mut missing = false;
    for idx in first..=last {
        if !st.cache.contains(idx) {
            missing = true;
            want(st, idx);
        }
    }
    if missing {
        st.ahead_from = Some(last + 1);
        return None;
    }
    st.ahead_from = Some(last + 1);
    let mut out = Vec::with_capacity((end - from) as usize);
    for idx in first..=last {
        let b = st.cache.get(idx)?;
        let s = from.saturating_sub(idx * block) as usize;
        let e = ((end - idx * block) as usize).min(b.len());
        if s < e {
            out.extend_from_slice(&b[s..e]);
        }
    }
    // A block shorter than expected (the file shrank): what is there.
    Some(out)
}

/// Take in a growth of the followed file.
fn grow(st: &mut State, c: Change) {
    let block = st.cache.block() as u64;
    // The last block was cut at the old end: read it again.
    let old = st.size;
    st.cache.drop_from(old / block);
    st.size = c.size;
    st.modified = c.modified;
}

/// The next block to read: a wanted one first (the most recent request),
/// then read-ahead.
fn pick(st: &mut State, readahead: u64) -> Option<(u64, u64)> {
    let block = st.cache.block() as u64;
    let blocks = st.size.div_ceil(block);
    while let Some(idx) = st.wanted.pop() {
        if idx < blocks && !st.cache.contains(idx) {
            return Some((st.generation, idx));
        }
    }
    let from = st.ahead_from?;
    for idx in from..(from + readahead).min(blocks) {
        if !st.cache.contains(idx) {
            return Some((st.generation, idx));
        }
    }
    st.ahead_from = None;
    None
}

fn note_error(inner: &Inner, e: &VfsError) {
    let mut st = inner.st.lock().unwrap();
    st.health = if e.is_connection_lost() { Health::Lost(e.text()) } else { Health::Failed(e.text()) };
    drop(st);
    (inner.repaint)();
}

async fn sleep_or_wake(inner: &Inner, d: Duration) {
    tokio::select! {
        _ = inner.wake.notified() => {}
        _ = tokio::time::sleep(d) => {}
    }
}

/// Reads wanted blocks; checks the file for changes when idle.
async fn reader(inner: Arc<Inner>) {
    let mut file: Option<Box<dyn RandomFile>> = None;
    let mut last_stat = Instant::now();
    loop {
        if inner.closed.load(Ordering::SeqCst) {
            return;
        }
        let job = {
            let mut st = inner.st.lock().unwrap();
            if matches!(st.health, Health::Failed(_)) && last_stat.elapsed() < inner.opts.retry {
                None
            } else {
                pick(&mut st, inner.opts.readahead)
            }
        };
        if let Some((gen, idx)) = job {
            if file.is_none() {
                match open_random(&inner.fs, &inner.path).await {
                    Ok(f) => file = Some(f),
                    Err(e) => {
                        inner.st.lock().unwrap().wanted.push(idx);
                        note_error(&inner, &e);
                        last_stat = Instant::now();
                        tokio::time::sleep(inner.opts.retry).await;
                        continue;
                    }
                }
            }
            let block = inner.opts.block as u64;
            let size = inner.st.lock().unwrap().size;
            let off = idx * block;
            let len = (size.saturating_sub(off)).min(block) as usize;
            if len == 0 {
                continue;
            }
            match file.as_mut().unwrap().read_at(off, len).await {
                Ok(data) => {
                    inner.read_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                    let mut st = inner.st.lock().unwrap();
                    if st.generation == gen {
                        if idx == 0 && st.detected.is_none() {
                            let d = detect(&data[..data.len().min(64 << 10)], data.len() as u64 >= st.size);
                            st.detected = Some(d);
                            if st.index.is_none() {
                                let mut ix = LineIndex::new(d.enc, d.bom as u64);
                                ix.set_hint(&data);
                                st.index = Some(ix);
                            }
                        }
                        st.cache.insert(idx, data.into());
                        if st.health != Health::Ok {
                            st.health = Health::Ok;
                        }
                    }
                    drop(st);
                    inner.wake.notify_waiters();
                    (inner.repaint)();
                }
                Err(e) => {
                    file = None;
                    if e.is_connection_lost() {
                        inner.st.lock().unwrap().wanted.push(idx);
                    }
                    note_error(&inner, &e);
                    last_stat = Instant::now();
                    tokio::time::sleep(inner.opts.retry).await;
                }
            }
            continue;
        }
        let (follow, lost) = {
            let st = inner.st.lock().unwrap();
            (st.follow, !matches!(st.health, Health::Ok))
        };
        let poll = if follow { inner.opts.poll / 3 } else { inner.opts.poll };
        let left = poll.saturating_sub(last_stat.elapsed());
        if !left.is_zero() {
            sleep_or_wake(&inner, left).await;
            continue;
        }
        last_stat = Instant::now();
        match inner.fs.stat(&inner.path).await {
            Ok(e) => {
                let mut st = inner.st.lock().unwrap();
                if lost && matches!(st.health, Health::Lost(_)) {
                    // Back: read what was wanted again.
                    st.health = Health::Ok;
                    file = None;
                }
                let now = Change { size: e.size, modified: e.modified };
                let differs = e.size != st.size || (e.modified.is_some() && e.modified != st.modified);
                if differs {
                    if st.follow && e.size >= st.size {
                        grow(&mut st, now);
                        // Show the new end.
                        let last = st.size.saturating_sub(1) / inner.opts.block as u64;
                        want(&mut st, last);
                    } else if st.changed != Some(now) {
                        st.changed = Some(now);
                    } else {
                        continue;
                    }
                    drop(st);
                    inner.wake.notify_waiters();
                    (inner.repaint)();
                }
            }
            Err(e) => {
                if e.is_connection_lost() {
                    note_error(&inner, &e);
                }
            }
        }
    }
}

/// Builds the line index from the start of the file (and further when a
/// followed file grows).
async fn indexer(inner: Arc<Inner>) {
    let mut file: Option<Box<dyn RandomFile>> = None;
    let mut last_paint = Instant::now();
    loop {
        if inner.closed.load(Ordering::SeqCst) {
            return;
        }
        if inner.index_paused.load(Ordering::SeqCst) {
            sleep_or_wake(&inner, Duration::from_millis(500)).await;
            continue;
        }
        let job = {
            let mut st = inner.st.lock().unwrap();
            let size = st.size;
            let follow = st.follow;
            let gen = st.generation;
            let error = st.index_error.is_some();
            match st.index.as_mut() {
                Some(ix) if !error && ix.scanned() < size && ix.scanned() < inner.opts.index_limit => {
                    ix.done = false;
                    Some((gen, ix.scanned(), (size - ix.scanned()).min(inner.opts.index_chunk as u64) as usize, ix.enc()))
                }
                Some(ix) if !ix.done && !follow && ix.scanned() >= size => {
                    ix.finish();
                    drop(st);
                    (inner.repaint)();
                    None
                }
                _ => None,
            }
        };
        let Some((gen, at, len, enc)) = job else {
            sleep_or_wake(&inner, Duration::from_millis(500)).await;
            continue;
        };
        if file.is_none() {
            match open_random(&inner.fs, &inner.path).await {
                Ok(f) => file = Some(f),
                Err(e) => {
                    index_failed(&inner, e).await;
                    continue;
                }
            }
        }
        // UTF-16: whole units (the last odd byte only at the end).
        let len = if enc.unit() == 2 && len > 1 && len % 2 == 1 { len - 1 } else { len };
        match file.as_mut().unwrap().read_at(at, len).await {
            Ok(data) if data.is_empty() => {
                // The file is shorter than thought: the reader notices the change.
                sleep_or_wake(&inner, Duration::from_millis(500)).await;
            }
            Ok(data) => {
                inner.read_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                {
                    let mut st = inner.st.lock().unwrap();
                    let size = st.size;
                    let follow = st.follow;
                    if st.generation == gen {
                        if let Some(ix) = st.index.as_mut().filter(|ix| ix.scanned() == at && ix.enc() == enc) {
                            ix.feed(&data);
                            if ix.scanned() >= size && !follow {
                                ix.finish();
                            }
                        }
                    }
                }
                if last_paint.elapsed() > Duration::from_millis(250) {
                    last_paint = Instant::now();
                    (inner.repaint)();
                }
                // Let the block reader go first (it serves what is on the screen).
                tokio::task::yield_now().await;
            }
            Err(e) => {
                file = None;
                index_failed(&inner, e).await;
            }
        }
    }
}

async fn index_failed(inner: &Inner, e: VfsError) {
    if e.is_connection_lost() {
        tokio::time::sleep(inner.opts.retry).await;
    } else {
        inner.st.lock().unwrap().index_error = Some(e.text());
        (inner.repaint)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rolid_core::vfs::{Reader, Writer};
    use rolid_core::{Entry, VfsResult};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    fn quick() -> Opts {
        Opts { block: 4096, cache_limit: 128 << 10, readahead: 1, index_chunk: 10_000, index_limit: u64::MAX, poll: Duration::from_millis(50), retry: Duration::from_millis(30) }
    }

    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let t = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn open(rt: &tokio::runtime::Runtime, fs: Arc<dyn Vfs>, path: &str) -> Doc {
        let e = rt.block_on(fs.stat(path)).unwrap();
        Doc::open(rt.handle(), fs, path.into(), e.size, e.modified, quick(), Arc::new(|| {}))
    }

    #[test]
    fn windows_come_from_blocks_and_the_index_is_built() {
        let rt = rt();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        let mut text = String::new();
        for i in 0..20_000 {
            text.push_str(&format!("{i}\n"));
        }
        std::fs::write(&p, &text).unwrap();
        let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
        let doc = open(&rt, fs, p.to_str().unwrap());
        let w = wait(|| doc.window(10_000, 9000));
        assert_eq!(w, text.as_bytes()[10_000..19_000]);
        // Block 0 (with the detection) may come after the blocks wanted first.
        assert_eq!(wait(|| doc.detected()).enc, Enc::Utf8);
        let ix = wait(|| doc.index_state().filter(|i| i.done));
        assert_eq!(ix.total, LineNo::Exact(20_000));
        let off = text.find("\n12345\n").unwrap() as u64 + 1;
        let n = wait(|| Some(doc.line_at(off)).filter(|n| n.exact()));
        assert_eq!(n, LineNo::Exact(12_346));
        assert_eq!(wait(|| match doc.seek_line(12_346) { LineSeek::Found(o) => Some(o), _ => None }), off);
        // The cache stays under its limit while the whole file is read.
        for at in (0..text.len() as u64).step_by(4096) {
            wait(|| doc.window(at, 10));
            assert!(doc.cached_bytes() <= 128 << 10);
        }
    }

    #[test]
    fn changes_are_noticed_and_following_takes_growth_in() {
        let rt = rt();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("log");
        std::fs::write(&p, "one\n").unwrap();
        let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
        let doc = open(&rt, fs, p.to_str().unwrap());
        wait(|| doc.window(0, 4));
        std::fs::write(&p, "one\ntwo\n").unwrap();
        let c = wait(|| doc.changed());
        assert_eq!(c.size, 8);
        assert_eq!(doc.size(), 4);
        doc.set_follow(true);
        assert_eq!(doc.size(), 8);
        assert_eq!(wait(|| doc.window(0, 100).filter(|w| w.len() == 8)), b"one\ntwo\n");
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"three\n").unwrap();
        assert_eq!(wait(|| doc.window(0, 100).filter(|w| w.len() == 14)), b"one\ntwo\nthree\n");
        assert!(doc.changed().is_none());
        let ix = wait(|| doc.index_state().filter(|i| i.scanned == 14));
        assert_eq!(ix.total.get(), 3);
        // Reload after a change that was not followed.
        doc.set_follow(false);
        std::fs::write(&p, "x\n").unwrap();
        wait(|| doc.changed());
        doc.reload();
        assert_eq!(wait(|| doc.window(0, 100).filter(|w| w.len() == 2)), b"x\n");
    }

    #[test]
    fn the_index_stops_at_its_limit_and_numbers_are_estimates_after() {
        let rt = rt();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        let text = "line\n".repeat(50_000);
        std::fs::write(&p, &text).unwrap();
        let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
        let e = rt.block_on(fs.stat(p.to_str().unwrap())).unwrap();
        let doc = Doc::open(rt.handle(), fs, p.to_str().unwrap().into(), e.size, e.modified, Opts { index_limit: 100_000, ..quick() }, Arc::new(|| {}));
        let ix = wait(|| doc.index_state().filter(|i| i.limited));
        assert!(ix.scanned >= 100_000 && ix.scanned < 110_000);
        assert!(!ix.done && !ix.total.exact());
        assert!((ix.total.get() as i64 - 50_000).abs() < 100, "{:?}", ix.total);
        let end = text.len() as u64 - 5;
        wait(|| doc.window(end - 70_000, 70_000));
        assert!(!doc.line_at(end).exact());
        assert_eq!(doc.line_at(500), LineNo::Exact(101));
    }

    /// A server whose connection drops and comes back.
    struct Flaky {
        data: Vec<u8>,
        down: AtomicBool,
    }

    #[async_trait]
    impl Vfs for Flaky {
        fn label(&self) -> String {
            "flaky".into()
        }
        fn is_remote(&self) -> bool {
            true
        }
        fn join(&self, d: &str, n: &str) -> String {
            format!("{d}/{n}")
        }
        fn parent(&self, _: &str) -> Option<String> {
            None
        }
        async fn home(&self) -> VfsResult<String> {
            Ok("/".into())
        }
        async fn canonicalize(&self, p: &str) -> VfsResult<String> {
            Ok(p.into())
        }
        async fn list(&self, _: &str) -> VfsResult<Vec<Entry>> {
            Ok(vec![])
        }
        async fn stat(&self, _: &str) -> VfsResult<Entry> {
            if self.down.load(Ordering::SeqCst) {
                return Err(VfsError::ConnectionLost(Text::new("down")));
            }
            Ok(Entry { name: "f".into(), kind: rolid_core::EntryKind::File, link_to_dir: false, size: self.data.len() as u64, modified: None, mode: None })
        }
        async fn mkdir(&self, _: &str) -> VfsResult<()> {
            Ok(())
        }
        async fn remove_file(&self, _: &str) -> VfsResult<()> {
            Ok(())
        }
        async fn remove_dir(&self, _: &str) -> VfsResult<()> {
            Ok(())
        }
        async fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
            Ok(())
        }
        async fn open_read_at(&self, _: &str, offset: u64) -> VfsResult<Reader> {
            if self.down.load(Ordering::SeqCst) {
                return Err(VfsError::ConnectionLost(Text::new("down")));
            }
            Ok(Box::new(std::io::Cursor::new(self.data[offset as usize..].to_vec())))
        }
        async fn open_write_at(&self, _: &str, _: u64) -> VfsResult<Writer> {
            unimplemented!()
        }
        async fn set_attrs(&self, _: &str, _: Option<u32>, _: Option<SystemTime>) -> VfsResult<()> {
            Ok(())
        }
    }

    #[test]
    fn reading_continues_after_the_connection_is_back() {
        let rt = rt();
        let data: Vec<u8> = (0..100_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let fs = Arc::new(Flaky { data: data.clone(), down: AtomicBool::new(false) });
        let doc = open(&rt, fs.clone(), "/f");
        wait(|| doc.window(0, 100));
        fs.down.store(true, Ordering::SeqCst);
        assert!(doc.window(80_000, 100).is_none());
        wait(|| matches!(doc.health(), Health::Lost(_)).then_some(()));
        fs.down.store(false, Ordering::SeqCst);
        let w = wait(|| doc.window(80_000, 100));
        assert_eq!(w, data[80_000..80_100]);
        wait(|| (doc.health() == Health::Ok).then_some(()));
    }
}
