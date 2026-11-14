//! Searching in the viewer: the whole file in the background with ripgrep's
//! engine (grep-regex), from the shown position to the end first and then
//! from the start (the first hit is there at once), with progress and
//! cancel. On a server `grep -b -o` runs over SSH when the server has a
//! shell and grep (safe quoting); otherwise the file is read over SFTP.
//!
//! Hits never contain a line end. Blocks are cut after a line end; inside an
//! extremely long line a block is cut anyway and [`KEEP`] bytes are searched
//! again with the next block (a hit longer than that may be missed there).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use rolid_core::vfs::{open_random, RandomFile};
use rolid_core::Vfs;
use rolid_text::{txt, Text};

use crate::enc::Enc;

/// Overlap of blocks cut inside a long line.
pub const KEEP: usize = 64 << 10;
/// Hits kept (more are only counted).
pub const MAX_HITS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub text: String,
    /// Upper and lower case differ.
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

/// How the file was searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Read here (a local file, or a server file over SFTP).
    Read,
    /// `grep` on the server over SSH.
    Grep,
}

/// What a search found so far.
#[derive(Debug, Default)]
pub struct Found {
    /// Hits before the start position and from it on (both ascending).
    before: Vec<(u64, u32)>,
    after: Vec<(u64, u32)>,
    /// All hits (more than kept if [`Self::capped`]).
    pub count: u64,
    pub capped: bool,
    /// Bytes searched so far (`None` = not known: grep on the server).
    pub scanned: Option<u64>,
    pub total: u64,
    pub done: bool,
    pub error: Option<Text>,
    pub method: Option<Method>,
    /// Where the search started.
    pub from: u64,
}

impl Found {
    pub fn len(&self) -> usize {
        self.before.len() + self.after.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Hit `i` in file order.
    pub fn get(&self, i: usize) -> Option<(u64, u32)> {
        if i < self.before.len() {
            Some(self.before[i])
        } else {
            self.after.get(i - self.before.len()).copied()
        }
    }

    /// Index of the first hit starting at or after `pos`.
    pub fn first_from(&self, pos: u64) -> usize {
        let b = self.before.partition_point(|h| h.0 < pos);
        if b < self.before.len() {
            return b;
        }
        self.before.len() + self.after.partition_point(|h| h.0 < pos)
    }

    /// Hits overlapping `[a, b)`.
    pub fn within(&self, a: u64, b: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut i = self.first_from(a.saturating_sub(KEEP as u64));
        while let Some((o, l)) = self.get(i) {
            if o >= b {
                break;
            }
            if o + l as u64 > a {
                out.push((o, o + l as u64));
            }
            i += 1;
        }
        out
    }

    fn push(&mut self, off: u64, len: u32) {
        self.count += 1;
        if self.len() >= MAX_HITS {
            self.capped = true;
            return;
        }
        if off < self.from {
            self.before.push((off, len));
        } else {
            self.after.push((off, len));
        }
    }
}

/// Running a command on the server (SSH exec).
#[async_trait]
pub trait Exec: Send + Sync {
    /// Run `cmd`, give the standard output to `out` (which returns `false`
    /// to stop); returns the exit status and the start of the error output.
    async fn run(&self, cmd: &str, cancel: &AtomicBool, out: &mut (dyn for<'b> FnMut(&'b [u8]) -> bool + Send)) -> Result<(Option<u32>, String), String>;

    /// [`run`](Self::run) for a short check of what the server can do: ends
    /// with an error when it does not answer in time (the search then reads
    /// the file itself).
    async fn probe(&self, cmd: &str, cancel: &AtomicBool, out: &mut (dyn for<'b> FnMut(&'b [u8]) -> bool + Send)) -> Result<(Option<u32>, String), String> {
        self.run(cmd, cancel, out).await
    }
}

/// What to search.
pub struct Target {
    pub fs: Arc<dyn Vfs>,
    pub path: String,
    pub size: u64,
    pub enc: Enc,
    /// Where the text starts (after a BOM).
    pub start: u64,
    /// Start here, then from the beginning.
    pub from: u64,
    /// The server's shell, if any.
    pub exec: Option<Arc<dyn Exec>>,
    /// Bytes read at once.
    pub chunk: usize,
}

/// A running search (canceled when dropped).
pub struct Search {
    pub found: Arc<Mutex<Found>>,
    cancel: Arc<AtomicBool>,
    pub query: Query,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Search {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// The matcher for a query (`Err` = a bad regular expression).
pub fn matcher(q: &Query) -> Result<RegexMatcher, String> {
    let mut b = RegexMatcherBuilder::new();
    b.case_insensitive(!q.case).word(q.word).fixed_strings(!q.regex).line_terminator(Some(b'\n'));
    b.build(&q.text).map_err(|e| e.to_string())
}

/// Start searching `q` in `t`; `Err` = the query is not valid.
pub fn start(rt: &tokio::runtime::Handle, t: Target, q: Query, repaint: Arc<dyn Fn() + Send + Sync>) -> Result<Search, String> {
    if q.text.is_empty() {
        return Err(String::new());
    }
    let m = matcher(&q)?;
    let found = Arc::new(Mutex::new(Found { total: t.size, from: t.from, ..Default::default() }));
    let cancel = Arc::new(AtomicBool::new(false));
    let (f2, c2, q2) = (found.clone(), cancel.clone(), q.clone());
    rt.spawn(async move {
        let r = run(&t, &q2, m, &f2, &c2, &repaint).await;
        let mut f = f2.lock().unwrap();
        if let Err(e) = r {
            if !c2.load(Ordering::SeqCst) {
                f.error = Some(e);
            }
        }
        f.done = true;
        drop(f);
        repaint();
    });
    Ok(Search { found, cancel, query: q })
}

async fn run(t: &Target, q: &Query, m: RegexMatcher, found: &Mutex<Found>, cancel: &AtomicBool, repaint: &Arc<dyn Fn() + Send + Sync>) -> Result<(), Text> {
    if let Some(exec) = &t.exec {
        if let Some(cmd) = grep_command(exec.as_ref(), t, q, cancel).await {
            found.lock().unwrap().method = Some(Method::Grep);
            repaint();
            match grep(exec.as_ref(), &cmd, found, cancel, repaint).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // Something went wrong on the server: read the file instead.
                    tracing::info!("viewer search: grep on the server failed ({e}); reading over SFTP");
                    let mut f = found.lock().unwrap();
                    let from = f.from;
                    *f = Found { total: t.size, from, ..Default::default() };
                }
            }
        }
    }
    found.lock().unwrap().method = Some(Method::Read);
    found.lock().unwrap().scanned = Some(0);
    let mut file = open_random(&t.fs, &t.path).await.map_err(|e| e.text())?;
    let transcode = !(t.enc == Enc::Utf8 || (t.enc.ascii_compatible() && q.text.is_ascii() && !q.word));
    let from = t.from.clamp(t.start, t.size.max(t.start));
    let from = from - (from - t.start) % t.enc.unit() as u64;
    let mut sc = Scan { file: file.as_mut(), enc: t.enc, transcode, m: &m, chunk: t.chunk, found, cancel, repaint, done_before: 0 };
    // From the shown place to the end, then the start up to it.
    sc.scan(from, t.size, u64::MAX).await?;
    sc.done_before = t.size - from;
    let end = (from + KEEP as u64).min(t.size);
    if from > t.start {
        sc.scan(t.start, end, from).await?;
    }
    Ok(())
}

struct Scan<'a> {
    file: &'a mut dyn RandomFile,
    enc: Enc,
    transcode: bool,
    m: &'a RegexMatcher,
    chunk: usize,
    found: &'a Mutex<Found>,
    cancel: &'a AtomicBool,
    repaint: &'a Arc<dyn Fn() + Send + Sync>,
    /// Bytes searched in an earlier pass (progress).
    done_before: u64,
}

impl Scan<'_> {
    /// Search `[a, b)`; hits starting at `limit` or later are not reported.
    async fn scan(&mut self, a: u64, b: u64, limit: u64) -> Result<(), Text> {
        let u = self.enc.unit();
        let mut buf: Vec<u8> = Vec::new();
        let mut base = a;
        let mut pos = a;
        let mut last_paint = std::time::Instant::now();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let want = (self.chunk as u64).min(b - pos) as usize;
            let want = want - want % u.max(1);
            let data = if want == 0 { Vec::new() } else { self.file.read_at(pos, want).await.map_err(|e| e.text())? };
            let eof = data.len() < want || pos + data.len() as u64 >= b || want == 0;
            pos += data.len() as u64;
            buf.extend_from_slice(&data);
            // Where to cut: after the last line end; inside a long line anyway.
            let (cut, forced) = if eof {
                (buf.len(), false)
            } else {
                match self.enc.rfind_newline(&buf, buf.len()) {
                    Some(p) => (p + u, false),
                    None if buf.len() > 2 * KEEP => (buf.len(), true),
                    None => (0, false),
                }
            };
            if cut > 0 {
                // A forced cut reports only hits ending before the last KEEP bytes.
                let report_end = if forced { cut - KEEP } else { cut };
                let mut first_open: Option<usize> = None;
                let mut hits = Vec::new();
                self.find(&buf[..cut], |s, e| {
                    if e <= report_end {
                        hits.push((s, e));
                    } else if first_open.is_none() {
                        first_open = Some(s);
                    }
                });
                {
                    let mut f = self.found.lock().unwrap();
                    for (s, e) in hits {
                        let off = base + s as u64;
                        if off < limit {
                            f.push(off, (e - s) as u32);
                        }
                    }
                    f.scanned = Some((self.done_before + (base + report_end as u64 - a)).min(f.total));
                }
                let keep_from = if forced {
                    let k = first_open.unwrap_or(report_end).min(report_end);
                    k - k % u
                } else {
                    cut
                };
                buf.drain(..keep_from);
                base += keep_from as u64;
                if last_paint.elapsed().as_millis() > 100 {
                    last_paint = std::time::Instant::now();
                    (self.repaint)();
                }
            }
            if eof {
                return Ok(());
            }
        }
    }

    /// Hits in `hay` (file bytes): `(start, end)` relative to it.
    fn find(&self, hay: &[u8], mut out: impl FnMut(usize, usize)) {
        if !self.transcode {
            let _ = self.m.find_iter(hay, |m| {
                if m.end() > m.start() {
                    out(m.start(), m.end());
                }
                true
            });
            return;
        }
        let (text, src) = self.enc.decode_mapped(hay);
        let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let to_src = |b: usize| -> usize {
            match starts.binary_search(&b) {
                Ok(k) => src[k] as usize,
                Err(_) => hay.len(),
            }
        };
        let _ = self.m.find_iter(text.as_bytes(), |m| {
            if m.end() > m.start() {
                let s = to_src(m.start());
                let e = if m.end() >= text.len() { hay.len() } else { to_src(m.end()) };
                out(s, e);
            }
            true
        });
    }
}

/// Single quotes for the shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The grep command for the query on this server, if the server can run it
/// with the same meaning (checked by a small test run first).
async fn grep_command(exec: &dyn Exec, t: &Target, q: &Query, cancel: &AtomicBool) -> Option<String> {
    let ascii = q.text.is_ascii();
    // Non-ASCII text, case or word rules need a UTF-8 locale on the server.
    let locale = if t.enc == Enc::Utf8 {
        if ascii && q.case && !q.word {
            "C"
        } else if probe(exec, "C.UTF-8", "-i -F", "\u{17d}", "\u{17e}", cancel).await { // i18n-ok: a test letter for the server
            "C.UTF-8"
        } else {
            return None;
        }
    } else if t.enc.ascii_compatible() && ascii && !q.word && q.case {
        "C"
    } else {
        return None;
    };
    // A regular expression needs Perl syntax (closest to the one here).
    let kind = if q.regex {
        if !probe(exec, locale, "-P", "x\\d", "x1", cancel).await {
            return None;
        }
        "-P"
    } else {
        if !probe(exec, locale, "-F", "a1", "a1", cancel).await {
            return None;
        }
        "-F"
    };
    // Hits as they are found (a pipe would otherwise get them in big blocks, the first one
    // perhaps only at the end of a huge file); GNU and BSD grep know it, others may not.
    let lb = if probe(exec, locale, &format!("{kind} --line-buffered"), "a1", "a1", cancel).await { " --line-buffered" } else { "" };
    let mut cmd = format!("LC_ALL={locale} grep -a -b -o{lb} {kind}");
    if !q.case {
        cmd.push_str(" -i");
    }
    if q.word {
        cmd.push_str(" -w");
    }
    cmd.push_str(&format!(" -e {} -- {}", sh_quote(&q.text), sh_quote(&t.path)));
    Some(cmd)
}

/// Does `grep OPTS -e PATTERN` find `sample` with its offset on the server?
async fn probe(exec: &dyn Exec, locale: &str, opts: &str, pattern: &str, sample: &str, cancel: &AtomicBool) -> bool {
    let cmd = format!("command -v grep >/dev/null 2>&1 && printf '%s\\n' {} | LC_ALL={locale} grep -a -b -o {opts} -e {}", sh_quote(&format!("_{sample}")), sh_quote(pattern));
    let mut out = Vec::new();
    let ok = exec
        .probe(&cmd, cancel, &mut |d: &[u8]| {
            out.extend_from_slice(d);
            out.len() < 4096
        })
        .await;
    matches!(ok, Ok((Some(0), _))) && out == format!("1:{sample}\n").as_bytes()
}

/// Run grep and take in its `offset:match` lines.
async fn grep(exec: &dyn Exec, cmd: &str, found: &Mutex<Found>, cancel: &AtomicBool, repaint: &Arc<dyn Fn() + Send + Sync>) -> Result<(), Text> {
    let mut line: Vec<u8> = Vec::new();
    let mut last_paint = std::time::Instant::now();
    let mut bad = false;
    let mut take = |data: &[u8]| -> bool {
        for &b in data {
            if b != b'\n' {
                if line.len() < (1 << 20) {
                    line.push(b);
                }
                continue;
            }
            let colon = line.iter().position(|&c| c == b':');
            match colon.and_then(|i| std::str::from_utf8(&line[..i]).ok()?.parse::<u64>().ok().map(|o| (o, line.len() - i - 1))) {
                Some((off, len)) if len > 0 => found.lock().unwrap().push(off, len as u32),
                Some(_) => {}
                None => bad = true,
            }
            line.clear();
        }
        if last_paint.elapsed().as_millis() > 100 {
            last_paint = std::time::Instant::now();
            repaint();
        }
        !bad
    };
    let (status, err) = exec.run(cmd, cancel, &mut take).await.map_err(|e| txt!("{error}", error = e))?;
    if bad {
        return Err(txt!("unexpected output of grep on the server"));
    }
    match status {
        Some(0) | Some(1) => Ok(()),
        _ if cancel.load(Ordering::SeqCst) => Ok(()),
        _ => Err(txt!("grep on the server failed: {error}", error = err.trim().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    fn file(data: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, data).unwrap();
        (dir, p.to_str().unwrap().to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn search(rt: &tokio::runtime::Runtime, path: &str, size: u64, enc: Enc, from: u64, chunk: usize, q: Query, exec: Option<Arc<dyn Exec>>) -> (Vec<(u64, u32)>, Found) {
        let t = Target { fs: Arc::new(rolid_local::LocalFs), path: path.into(), size, enc, start: 0, from, exec, chunk };
        let s = start(rt.handle(), t, q, Arc::new(|| {})).unwrap();
        let t0 = Instant::now();
        loop {
            if s.found.lock().unwrap().done {
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
        let f = std::mem::take(&mut *s.found.lock().unwrap());
        let hits = (0..f.len()).map(|i| f.get(i).unwrap()).collect();
        (hits, f)
    }

    fn naive(data: &[u8], q: &Query) -> Vec<(u64, u32)> {
        let m = matcher(q).unwrap();
        let mut v = Vec::new();
        // Line by line (hits never cross a line end).
        let mut at = 0;
        for line in data.split(|&b| b == b'\n') {
            let _ = m.find_iter(line, |x| {
                if x.end() > x.start() {
                    v.push(((at + x.start()) as u64, (x.end() - x.start()) as u32));
                }
                true
            });
            at += line.len() + 1;
        }
        v
    }

    fn q(text: &str, case: bool, word: bool, regex: bool) -> Query {
        Query { text: text.into(), case, word, regex }
    }

    #[test]
    fn hits_across_block_boundaries_are_found_once_and_in_place() {
        let rt = rt();
        let mut d = String::new();
        for i in 0..2000 {
            d.push_str(&format!("line {i}: Needle needle NEEDLE xneedle needles {}\n", "ab".repeat(i % 37)));
        }
        let (_dir, p) = file(d.as_bytes());
        for query in [q("needle", false, false, false), q("needle", true, false, false), q("needle", false, true, false), q(r"ne+dl\w*", false, false, true), q("le 4", true, false, false)] {
            let want = naive(d.as_bytes(), &query);
            for chunk in [97, 4096, 1 << 20] {
                for from in [0u64, 12_345, d.len() as u64 - 3] {
                    let (got, f) = search(&rt, &p, d.len() as u64, Enc::Utf8, from, chunk, query.clone(), None);
                    assert_eq!(got, want, "{query:?} chunk {chunk} from {from}");
                    assert_eq!(f.count, want.len() as u64);
                    assert_eq!(f.method, Some(Method::Read));
                    assert_eq!(f.scanned, Some(d.len() as u64));
                }
            }
        }
    }

    #[test]
    fn an_extremely_long_line_is_searched_in_pieces() {
        let rt = rt();
        let mut d = b"start\n".to_vec();
        for i in 0..200_000 {
            d.extend(format!("w{i} ").as_bytes());
        }
        d.extend(b"\nend w7 w19999\n");
        let (_dir, p) = file(&d);
        let query = q(r"w1999\d", true, true, true);
        let want = naive(&d, &query);
        assert!(want.len() > 10);
        let (got, _) = search(&rt, &p, d.len() as u64, Enc::Utf8, 777_777, 50_000, query, None);
        assert_eq!(got, want);
    }

    #[test]
    fn other_encodings_are_searched_as_text() {
        let rt = rt();
        let text = "Žluťoučký kůň\nžluťoučký KŮŇ a kůň\n".repeat(300);
        for enc in [Enc::Win1250, Enc::Cp852, Enc::Utf16Le] {
            let bytes = enc.encode(&text).unwrap();
            let (_dir, p) = file(&bytes);
            let (got, _) = search(&rt, &p, bytes.len() as u64, enc, 0, 1000, q("kůň", false, true, false), None);
            assert_eq!(got.len(), 900, "{enc:?}");
            // The positions point at the word in the file's own bytes.
            for (o, l) in got.iter().take(5) {
                let s = enc.decode(&bytes[*o as usize..(*o + *l as u64) as usize]).to_lowercase();
                assert_eq!(s, "kůň", "{enc:?}");
            }
        }
    }

    #[test]
    fn the_found_list_answers_next_and_visible() {
        let mut f = Found { from: 100, ..Default::default() };
        for o in [120, 150, 400] {
            f.push(o, 3);
        }
        for o in [5, 60] {
            f.push(o, 3);
        }
        assert_eq!((0..5).map(|i| f.get(i).unwrap().0).collect::<Vec<_>>(), vec![5, 60, 120, 150, 400]);
        assert_eq!(f.first_from(61), 2);
        assert_eq!(f.first_from(0), 0);
        assert_eq!(f.first_from(500), 5);
        assert_eq!(f.within(59, 151), vec![(60, 63), (120, 123), (150, 153)]);
    }

    /// The local shell standing in for a server's.
    struct Sh;

    #[async_trait]
    impl Exec for Sh {
        async fn run(&self, cmd: &str, cancel: &AtomicBool, out: &mut (dyn for<'b> FnMut(&'b [u8]) -> bool + Send)) -> Result<(Option<u32>, String), String> {
            let _ = cancel;
            let o = std::process::Command::new("sh").arg("-c").arg(cmd).output().map_err(|e| e.to_string())?;
            out(&o.stdout);
            Ok((o.status.code().map(|c| c as u32), String::from_utf8_lossy(&o.stderr).into_owned()))
        }
    }

    #[test]
    fn grep_on_the_server_gives_the_same_hits() {
        if std::process::Command::new("grep").arg("--version").output().is_err() {
            return;
        }
        let rt = rt();
        let mut d = String::new();
        for i in 0..3000 {
            d.push_str(&format!("{i} it's -e 'quoted' \"x\" $HOME ŽLUŤ žluť a1 b22 c333\n"));
        }
        let dir = tempfile::tempdir().unwrap();
        // A file name the shell must not misread.
        let p = dir.path().join("-n it's $x.txt");
        std::fs::write(&p, &d).unwrap();
        let p = p.to_str().unwrap();
        for query in [q("it's -e 'quoted'", true, false, false), q("$HOME", true, false, false), q("žluť", false, false, false), q(r"[a-c]\d{2,}", true, true, true)] {
            let want = naive(d.as_bytes(), &query);
            let (got, f) = search(&rt, p, d.len() as u64, Enc::Utf8, 0, 1 << 20, query.clone(), Some(Arc::new(Sh)));
            assert_eq!(got, want, "{query:?} by {:?}", f.method);
            assert_eq!(f.method, Some(Method::Grep), "{query:?}");
        }
        // Hits come line by line where grep can do that.
        let t = Target { fs: Arc::new(rolid_local::LocalFs), path: p.into(), size: d.len() as u64, enc: Enc::Utf8, start: 0, from: 0, exec: None, chunk: 1 << 20 };
        let cancel = AtomicBool::new(false);
        let cmd = rt.block_on(grep_command(&Sh, &t, &q("a1", true, false, false), &cancel)).unwrap();
        assert!(cmd.contains("--line-buffered"), "{cmd}");
        // UTF-16 is read (grep cannot).
        let (_, f) = search(&rt, p, d.len() as u64, Enc::Utf16Le, 0, 1 << 20, q("x", true, false, false), Some(Arc::new(Sh)));
        assert_eq!(f.method, Some(Method::Read));
    }

    #[test]
    fn a_bad_regular_expression_is_refused() {
        let rt = rt();
        let (_dir, p) = file(b"abc");
        let t = Target { fs: Arc::new(rolid_local::LocalFs), path: p, size: 3, enc: Enc::Utf8, start: 0, from: 0, exec: None, chunk: 100 };
        assert!(start(rt.handle(), t, q("a(", true, false, true), Arc::new(|| {})).is_err());
    }
}
