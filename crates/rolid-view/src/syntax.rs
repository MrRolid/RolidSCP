//! Syntax highlighting (syntect with pure-Rust regular expressions, the
//! grammars of two-face). Only the lines on the screen are highlighted, on a
//! thread of their own: the parser state is saved every few lines, so
//! scrolling on continues from a saved state. After a jump far ahead the
//! visible part is first highlighted from a fresh state a little before it
//! (nearly always right) and then again exactly once the saved states reach
//! it. Big files and extremely long lines turn highlighting off.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rolid_core::vfs::{open_random, RandomFile};
use rolid_core::Vfs;
use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crate::enc::Enc;

/// Default size limit for highlighting.
pub const DEFAULT_LIMIT: u64 = 20 << 20;
/// A line this long turns highlighting off.
pub const LONG_LINE: u64 = 16 << 10;
/// Parser states are saved this often (lines).
const SAVE_EVERY: u64 = 200;
/// A jump further than this ahead of the exact states is highlighted from a fresh state first.
const FAR: u64 = 256 << 10;
/// How far before the visible part a fresh state starts.
const LEAD: u64 = 16 << 10;
/// Highlighted lines kept.
const KEEP_LINES: usize = 20_000;

fn sets() -> &'static (SyntaxSet, ThemeSet) {
    static S: OnceLock<(SyntaxSet, ThemeSet)> = OnceLock::new();
    S.get_or_init(|| (two_face::syntax::extra_newlines(), ThemeSet::load_defaults()))
}

/// Load the grammars (slow the first time: do it in the background).
pub fn preload() {
    let _ = sets();
}

/// The names of the languages for the menu (sorted).
pub fn names() -> Vec<String> {
    let mut v: Vec<String> = sets().0.syntaxes().iter().filter(|s| !s.hidden && s.name != "Plain Text").map(|s| s.name.clone()).collect();
    v.sort_by_key(|n| n.to_lowercase());
    v.dedup();
    v
}

/// The language of a file by its name (extension, or the whole name like
/// `Dockerfile`) or its first line (`#!/bin/sh`, `<?xml`, `<?php`).
pub fn guess(file_name: &str, first_line: &str) -> Option<String> {
    let ss = &sets().0;
    let lower = file_name.to_lowercase();
    let by_name = ss.find_syntax_by_extension(file_name).or_else(|| ss.find_syntax_by_extension(&lower));
    let by_ext = || lower.rsplit_once('.').and_then(|(_, e)| ss.find_syntax_by_extension(e));
    // Files without an extension that are known by their name.
    let special = || match lower.as_str() {
        n if n.starts_with("dockerfile") || n.ends_with(".dockerfile") || n == "containerfile" => ss.find_syntax_by_name("Dockerfile"),
        n if n.ends_with(".tsx") => ss.find_syntax_by_name("TypeScriptReact"),
        _ => None,
    };
    let found = special().or(by_name).or_else(by_ext).or_else(|| ss.find_syntax_by_first_line(first_line));
    found.filter(|s| s.name != "Plain Text").map(|s| s.name.clone())
}

/// A colored part of a line: bytes `[start, end)` of the line in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub rgb: [u8; 3],
    pub bold: bool,
    pub italic: bool,
}

/// Why highlighting is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Off {
    TooBig,
    LongLines,
    /// The file could not be read for it.
    Failed,
}

/// The theme for a light or dark look.
fn theme(dark: bool) -> &'static Theme {
    let t = &sets().1.themes;
    if dark { &t["base16-ocean.dark"] } else { &t["InspiredGitHub"] }
}

/// The background the colors must be readable on (the viewer's).
pub fn background(dark: bool) -> [u8; 3] {
    if dark { [0x1e, 0x1f, 0x22] } else { [0xff, 0xff, 0xff] }
}

fn lum(c: [u8; 3]) -> f64 {
    let f = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
}

/// WCAG contrast ratio.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `fg` moved towards black or white until it reads on `bg` (ratio 4.5).
pub fn readable(fg: [u8; 3], bg: [u8; 3]) -> [u8; 3] {
    let target = if lum(bg) > 0.5 { 0u8 } else { 255u8 };
    let mut c = fg;
    for step in 1..=20 {
        if contrast(c, bg) >= 4.5 {
            return c;
        }
        let t = step as f64 / 20.0;
        c = [0, 1, 2].map(|i| (fg[i] as f64 + (target as f64 - fg[i] as f64) * t).round() as u8);
    }
    c
}

#[derive(Clone)]
struct Saved {
    parse: ParseState,
    high: HighlightState,
}

struct Shared {
    /// Wanted: line starts from `from` up to `to`.
    want: Option<(u64, u64)>,
    lines: HashMap<u64, Arc<Vec<Span>>>,
    /// Lines highlighted from a fresh state (shown until exact ones replace them).
    approx: HashMap<u64, Arc<Vec<Span>>>,
    off: Option<Off>,
    busy: bool,
}

/// Highlighting of one file.
pub struct Highlighter {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    stop: Arc<AtomicBool>,
    pub syntax: String,
}

impl Drop for Highlighter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.shared.1.notify_all();
    }
}

/// What to highlight.
pub struct Source {
    pub fs: Arc<dyn Vfs>,
    pub path: String,
    pub size: u64,
    pub enc: Enc,
    /// Where the text starts (after a BOM).
    pub start: u64,
}

impl Highlighter {
    /// Start highlighting `src` as `syntax` (a name from [`names`]); `limit`
    /// is the size over which highlighting is off.
    pub fn new(rt: tokio::runtime::Handle, src: Source, syntax: String, dark: bool, limit: u64, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        let shared = Arc::new((Mutex::new(Shared { want: None, lines: HashMap::new(), approx: HashMap::new(), off: None, busy: false }), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        if src.size > limit {
            shared.0.lock().unwrap().off = Some(Off::TooBig);
        } else {
            let (sh, st, name) = (shared.clone(), stop.clone(), syntax.clone());
            std::thread::Builder::new()
                .name("highlight".into())
                .spawn(move || worker(rt, src, name, dark, sh, st, repaint))
                .expect("thread");
        }
        Highlighter { shared, stop, syntax }
    }

    /// Lines from the one starting at `from` to `to` are on the screen.
    pub fn show(&self, from: u64, to: u64) {
        let mut s = self.shared.0.lock().unwrap();
        if s.want != Some((from, to)) {
            s.want = Some((from, to));
            self.shared.1.notify_all();
        }
    }

    /// The colors of the line starting at `line` (`None` = not yet); the
    /// flag says they are exact.
    pub fn spans(&self, line: u64) -> Option<(Arc<Vec<Span>>, bool)> {
        let s = self.shared.0.lock().unwrap();
        s.lines.get(&line).map(|v| (v.clone(), true)).or_else(|| s.approx.get(&line).map(|v| (v.clone(), false)))
    }

    pub fn off(&self) -> Option<Off> {
        self.shared.0.lock().unwrap().off
    }

    pub fn busy(&self) -> bool {
        self.shared.0.lock().unwrap().busy
    }
}

/// Reads the file in pieces for the worker.
struct Reader {
    rt: tokio::runtime::Handle,
    file: Box<dyn RandomFile>,
    size: u64,
    enc: Enc,
    /// A piece of the file: `(base, bytes)`.
    buf: (u64, Vec<u8>),
}

impl Reader {
    /// The line starting at `at`: its bytes (with the line end) and the start
    /// of the next line. `Err(true)` = a line too long.
    fn line(&mut self, at: u64) -> Result<(Vec<u8>, u64), bool> {
        let mut out = Vec::new();
        let mut pos = at;
        loop {
            if pos >= self.size {
                return Ok((out, self.size));
            }
            let (base, ref data) = self.buf;
            if pos < base || pos >= base + data.len() as u64 {
                let len = (64 << 10).min((self.size - pos) as usize);
                let d = self.rt.block_on(self.file.read_at(pos, len)).map_err(|_| false)?;
                if d.is_empty() {
                    return Ok((out, pos));
                }
                self.buf = (pos, d);
                continue;
            }
            let i = (pos - base) as usize;
            match self.enc.find_newline(data, i) {
                Some(p) => {
                    let end = p + self.enc.unit();
                    out.extend_from_slice(&data[i..end.min(data.len())]);
                    let next = base + end as u64;
                    if end > data.len() {
                        // The second byte of a UTF-16 line end is in the next piece.
                        self.buf.1.clear();
                    }
                    if out.len() as u64 > LONG_LINE {
                        return Err(true);
                    }
                    return Ok((out, next));
                }
                None => {
                    // An odd tail of UTF-16 stays for the next piece.
                    let take = data.len() - i;
                    let take = take - take % self.enc.unit();
                    if take == 0 {
                        self.buf.1.clear();
                        let d = self.rt.block_on(self.file.read_at(pos, 64 << 10)).map_err(|_| false)?;
                        if d.len() < self.enc.unit() {
                            out.extend_from_slice(&d);
                            return Ok((out, self.size));
                        }
                        self.buf = (pos, d);
                        continue;
                    }
                    out.extend_from_slice(&data[i..i + take]);
                    pos += take as u64;
                }
            }
            if out.len() as u64 > LONG_LINE {
                return Err(true);
            }
        }
    }

    /// The start of a line at or before `off`, at most `LEAD` bytes back
    /// (or `off` itself if none is that near).
    fn line_start_near(&mut self, off: u64, start: u64) -> u64 {
        let from = off.saturating_sub(LEAD).max(start);
        let from = from - (from - start) % self.enc.unit() as u64;
        let Ok(d) = self.rt.block_on(self.file.read_at(from, (off - from) as usize)) else { return off };
        match self.enc.rfind_newline(&d, d.len()) {
            Some(p) => from + (p + self.enc.unit()) as u64,
            None if from == start => start,
            None => off,
        }
    }
}

/// Highlight one line: the colors by file bytes of the line.
fn colors(line: &[u8], enc: Enc, parse: &mut ParseState, high: &mut HighlightState, h: &SynHighlighter, dark: bool) -> Vec<Span> {
    let ss = &sets().0;
    let (text, src) = if enc == Enc::Utf8 {
        (String::from_utf8_lossy(line).into_owned(), Vec::new())
    } else {
        enc.decode_mapped(line)
    };
    // The grammars expect the line end.
    let mut text = text;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let Ok(ops) = parse.parse_line(&text, ss) else { return Vec::new() };
    let bg = background(dark);
    let mut out = Vec::new();
    let mut at = 0usize;
    // String byte → file byte (only needed when the text is not the bytes).
    let map = |b: usize| -> u32 {
        if src.is_empty() {
            return b.min(line.len()) as u32;
        }
        let ci = text[..b.min(text.len())].chars().count();
        src.get(ci).copied().unwrap_or(line.len() as u32)
    };
    let utf8_exact = enc == Enc::Utf8 && std::str::from_utf8(line).is_ok();
    for (style, piece) in HighlightIterator::new(high, &ops, &text, h) {
        let (s, e) = (at, at + piece.len());
        at = e;
        let (fs, fe) = if utf8_exact || !src.is_empty() { (map(s), map(e)) } else { (s as u32, e as u32) };
        if fs >= fe {
            continue;
        }
        let f = style.foreground;
        out.push(Span {
            start: fs,
            end: fe.min(line.len() as u32),
            rgb: readable([f.r, f.g, f.b], bg),
            bold: style.font_style.contains(FontStyle::BOLD),
            italic: style.font_style.contains(FontStyle::ITALIC),
        });
    }
    // Neighbours of one color become one span.
    out.dedup_by(|b, a| {
        if a.end == b.start && a.rgb == b.rgb && a.bold == b.bold && a.italic == b.italic {
            a.end = b.end;
            true
        } else {
            false
        }
    });
    out
}

fn worker(rt: tokio::runtime::Handle, src: Source, syntax: String, dark: bool, shared: Arc<(Mutex<Shared>, Condvar)>, stop: Arc<AtomicBool>, repaint: Arc<dyn Fn() + Send + Sync>) {
    let set_off = |off: Off| {
        shared.0.lock().unwrap().off = Some(off);
        repaint();
    };
    let ss = &sets().0;
    let Some(syn): Option<&SyntaxReference> = ss.find_syntax_by_name(&syntax) else { return set_off(Off::Failed) };
    let Ok(file) = rt.block_on(open_random(&src.fs, &src.path)) else { return set_off(Off::Failed) };
    let mut rd = Reader { rt, file, size: src.size, enc: src.enc, buf: (0, Vec::new()) };
    let theme = theme(dark);
    let h = SynHighlighter::new(theme);
    let fresh = || Saved { parse: ParseState::new(syn), high: HighlightState::new(&h, ScopeStack::new()) };
    // Exact states: by line start.
    let mut saved: BTreeMap<u64, Saved> = BTreeMap::new();
    saved.insert(src.start, fresh());
    let mut last_want = None;
    loop {
        // Wait for something to do.
        let want = {
            let mut s = shared.0.lock().unwrap();
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if s.want.is_some() && s.want != last_want {
                    break;
                }
                s.busy = false;
                s = shared.1.wait_timeout(s, Duration::from_millis(500)).unwrap().0;
            }
            s.busy = true;
            s.want
        };
        let Some((from, to)) = want else { continue };
        last_want = want;
        // Already done?
        let (&cp, _) = saved.range(..=from).next_back().unwrap();
        // Far ahead of the exact states: a quick approximate pass first.
        if from > cp + FAR {
            let begin = rd.line_start_near(from, src.start);
            let mut st = fresh();
            let mut at = begin;
            let mut out = Vec::new();
            while at < to.max(from + 1) && at < src.size {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                match rd.line(at) {
                    Ok((line, next)) => {
                        let spans = colors(&line, src.enc, &mut st.parse, &mut st.high, &h, dark);
                        if at >= from {
                            out.push((at, Arc::new(spans)));
                        }
                        if next <= at {
                            break;
                        }
                        at = next;
                    }
                    Err(true) => return set_off(Off::LongLines),
                    Err(false) => return set_off(Off::Failed),
                }
            }
            let mut s = shared.0.lock().unwrap();
            if s.approx.len() > KEEP_LINES {
                s.approx.clear();
            }
            s.approx.extend(out);
            drop(s);
            repaint();
        }
        // Exact: from the last saved state on, up to the end of the wanted part.
        let mut at = cp;
        let mut st = saved[&cp].clone();
        let mut n = 0u64;
        let mut batch = Vec::new();
        let mut last_paint = Instant::now();
        while at < to.max(from + 1) && at < src.size {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            // A new wish (scrolled away) is served first; the saved states stay.
            if n.is_multiple_of(64) {
                let s = shared.0.lock().unwrap();
                if s.want != last_want {
                    break;
                }
            }
            match rd.line(at) {
                Ok((line, next)) => {
                    let spans = colors(&line, src.enc, &mut st.parse, &mut st.high, &h, dark);
                    if at >= from.saturating_sub(LEAD) {
                        batch.push((at, Arc::new(spans)));
                    }
                    n += 1;
                    if next <= at {
                        break;
                    }
                    at = next;
                    if n.is_multiple_of(SAVE_EVERY) {
                        saved.insert(at, st.clone());
                    }
                    if batch.len() >= 256 || last_paint.elapsed() > Duration::from_millis(100) {
                        flush(&shared, &mut batch, &repaint);
                        last_paint = Instant::now();
                    }
                }
                Err(true) => return set_off(Off::LongLines),
                Err(false) => return set_off(Off::Failed),
            }
        }
        saved.insert(at, st);
        flush(&shared, &mut batch, &repaint);
    }
}

fn flush(shared: &Arc<(Mutex<Shared>, Condvar)>, batch: &mut Vec<(u64, Arc<Vec<Span>>)>, repaint: &Arc<dyn Fn() + Send + Sync>) {
    if batch.is_empty() {
        return;
    }
    let mut s = shared.0.lock().unwrap();
    if s.lines.len() > KEEP_LINES {
        s.lines.clear();
    }
    for (k, v) in batch.drain(..) {
        s.approx.remove(&k);
        s.lines.insert(k, v);
    }
    drop(s);
    repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_found_by_name_extension_and_first_line() {
        for (name, first, want) in [
            ("a.xml", "", "XML"),
            ("a.json", "", "JSON"),
            ("Cargo.toml", "", "TOML"),
            ("x.ts", "", "TypeScript"),
            ("Dockerfile", "", "Dockerfile"),
            ("php.ini", "", "INI"),
            ("index.php", "", "PHP"),
            ("run", "#!/bin/sh", "Bourne Again Shell (bash)"),
            ("noext", "<?xml version=\"1.0\"?>", "XML"),
            ("README.md", "", "Markdown"),
            ("q.sql", "", "SQL"),
            ("a.yaml", "", "YAML"),
            ("m.rs", "", "Rust"),
            ("x.py", "", "Python"),
        ] {
            assert_eq!(guess(name, first).as_deref(), Some(want), "{name}");
        }
        assert_eq!(guess("notes.unknownext", "just text"), None);
        assert!(names().len() > 100);
    }

    #[test]
    fn theme_colors_are_readable_on_both_looks() {
        for dark in [false, true] {
            let bg = background(dark);
            for item in &theme(dark).scopes {
                if let Some(f) = item.style.foreground {
                    let c = readable([f.r, f.g, f.b], bg);
                    assert!(contrast(c, bg) >= 4.5, "{dark} {:?}", c);
                }
            }
        }
    }

    fn run_to_end(path: &std::path::Path, name: &str, limit: u64) -> Highlighter {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let size = std::fs::metadata(path).unwrap().len();
        let src = Source { fs: Arc::new(rolid_local_fs()), path: path.to_str().unwrap().into(), size, enc: Enc::Utf8, start: 0 };
        let h = Highlighter::new(rt.handle().clone(), src, name.into(), false, limit, Arc::new(|| {}));
        std::mem::forget(rt); // the worker keeps using the handle
        h
    }

    fn rolid_local_fs() -> rolid_local::LocalFs {
        rolid_local::LocalFs
    }

    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let t = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t.elapsed() < Duration::from_secs(30), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn highlighting_turns_itself_off() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.json");
        std::fs::write(&big, "[1]\n".repeat(1000)).unwrap();
        let h = run_to_end(&big, "JSON", 100);
        assert_eq!(h.off(), Some(Off::TooBig));
        let long = dir.path().join("min.js");
        let mut s = "var a = 1;\n".to_string();
        s.push_str(&"x=1;".repeat(10_000));
        s.push('\n');
        std::fs::write(&long, &s).unwrap();
        let h = run_to_end(&long, "JavaScript", DEFAULT_LIMIT);
        h.show(0, s.len() as u64);
        wait(|| h.off()).eq(&Off::LongLines).then_some(()).unwrap();
    }

    #[test]
    fn a_jump_into_the_middle_is_highlighted_and_then_exact() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        let mut s = String::new();
        let mut starts = Vec::new();
        for i in 0..40_000 {
            starts.push(s.len() as u64);
            s.push_str(&format!("{{\"key{i}\": [{i}, true, \"text\"]}}\n"));
        }
        std::fs::write(&p, &s).unwrap();
        let h = run_to_end(&p, "JSON", DEFAULT_LIMIT);
        let (from, to) = (starts[30_000], starts[30_010]);
        h.show(from, to);
        // Colors come (approximate first is fine), then exact ones.
        let (spans, _) = wait(|| h.spans(from));
        assert!(spans.len() > 3, "{spans:?}");
        let (exact, ok) = wait(|| h.spans(from).filter(|(_, e)| *e));
        assert!(ok);
        // The key string and the number have different colors.
        let line = &s.as_bytes()[from as usize..];
        let color_at = |b: u32| exact.iter().find(|sp| sp.start <= b && b < sp.end).map(|sp| sp.rgb);
        let key = line.iter().position(|&c| c == b'k').unwrap() as u32;
        let num = line.windows(3).position(|w| w == b": [").unwrap() as u32 + 3;
        assert_ne!(color_at(key), color_at(num));
        assert!(h.off().is_none());
    }

    #[test]
    fn colors_of_other_encodings_point_at_file_bytes() {
        let ss = &sets().0;
        let syn = ss.find_syntax_by_name("JSON").unwrap();
        let h = SynHighlighter::new(theme(false));
        let mut parse = ParseState::new(syn);
        let mut high = HighlightState::new(&h, ScopeStack::new());
        let line: Vec<u8> = "{\"žluť\": 1}\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let spans = colors(&line, Enc::Utf16Le, &mut parse, &mut high, &h, false);
        assert!(spans.iter().all(|s| s.start % 2 == 0 && s.end as usize <= line.len()));
        assert!(spans.last().unwrap().end as usize >= line.len() - 2);
    }
}
