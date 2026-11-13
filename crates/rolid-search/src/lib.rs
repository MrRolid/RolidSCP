//! File search: local (parallel walk, ripgrep's engine), remote over an SSH
//! command (`find` / `grep`, results streamed) and a generic fallback that walks
//! any [`Vfs`] (used for SFTP-only servers).
//!
//! The same [`Matcher`] filters results of every method, so name/size/date
//! semantics are identical everywhere; server-side filters only reduce traffic.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rolid_core::vfs::raw_text;
use rolid_core::{Entry, EntryKind, Vfs};
use rolid_text::{n, txt, Text};

pub mod remote;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NameMode {
    /// Masks like `*.rs;*.toml` (`;` separates alternatives).
    #[default]
    Glob,
    /// Name contains the text.
    Contains,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum KindFilter {
    #[default]
    Any,
    Files,
    Dirs,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Query {
    /// Directory to search in (absolute).
    pub root: String,
    pub name: String,
    pub name_mode: NameMode,
    pub case_sensitive: bool,
    /// Text to look for inside files (empty = name search only).
    pub content: String,
    pub content_regex: bool,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Modified within the last N days.
    pub modified_days: Option<u32>,
    pub kind: KindFilter,
    pub hidden: bool,
    /// Respect .gitignore/.ignore (local search only).
    pub gitignore: bool,
    pub max_results: usize,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            root: String::new(),
            name: String::new(),
            name_mode: NameMode::Glob,
            case_sensitive: false,
            content: String::new(),
            content_regex: false,
            min_size: None,
            max_size: None,
            modified_days: None,
            kind: KindFilter::Any,
            hidden: false,
            gitignore: false,
            max_results: 10_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    /// Directory containing the entry.
    pub dir: String,
    /// Path relative to the search root (for display).
    pub rel: String,
    pub entry: Entry,
    /// First matching line (number, text) for content searches, if known.
    pub line: Option<(u64, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Local,
    /// `find`/`grep` executed on the server.
    RemoteShell,
    /// Directory walk through the backend (SFTP without a shell, inside an archive).
    Walk,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Method::Local => n!("locally"),
            Method::RemoteShell => n!("by a command on the server"),
            Method::Walk => n!("by walking the folders"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub method: Method,
    pub hits: usize,
    pub truncated: bool,
    pub canceled: bool,
    pub elapsed: Duration,
}

/// Parse sizes like `500`, `10k`, `1.5M`, `2G` (binary units).
pub fn parse_size(s: &str) -> Result<Option<u64>, Text> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let (num, mul) = match s.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => {
            let unit = s[i..].trim().to_ascii_lowercase();
            let mul: u64 = match unit.trim_end_matches(['b', 'i']) {
                "" => 1,
                "k" => 1 << 10,
                "m" => 1 << 20,
                "g" => 1 << 30,
                "t" => 1 << 40,
                _ => return Err(txt!("unknown unit “{unit}”", unit = unit)),
            };
            (s[..i].trim(), mul)
        }
        None => (s, 1),
    };
    let n: f64 = num.replace(',', ".").parse().map_err(|_| txt!("invalid size “{size}”", size = s))?;
    if n < 0.0 {
        return Err(txt!("negative size"));
    }
    Ok(Some((n * mul as f64) as u64))
}

enum NameMatcher {
    All,
    Glob(globset::GlobSet),
    Contains(String, bool),
    Regex(regex::Regex),
}

/// Compiled query filters shared by all search methods.
pub struct Matcher {
    name: NameMatcher,
    pub content: Option<regex::bytes::Regex>,
    q: Query,
    newer_than: Option<SystemTime>,
}

impl Matcher {
    pub fn new(q: &Query) -> Result<Self, Text> {
        let pat = q.name.trim();
        let name = if pat.is_empty() {
            NameMatcher::All
        } else {
            match q.name_mode {
                NameMode::Glob => {
                    let mut b = globset::GlobSetBuilder::new();
                    for m in pat.split(';').map(str::trim).filter(|m| !m.is_empty()) {
                        // A bare word means "contains", like in Total Commander.
                        let m = if m.contains(['*', '?', '[']) { m.to_string() } else { format!("*{m}*") };
                        let g = globset::GlobBuilder::new(&m)
                            .case_insensitive(!q.case_sensitive)
                            .literal_separator(true)
                            .build()
                            .map_err(|e| txt!("mask “{mask}”: {error}", mask = &m, error = e.to_string()))?;
                        b.add(g);
                    }
                    NameMatcher::Glob(b.build().map_err(raw_text)?)
                }
                NameMode::Contains => NameMatcher::Contains(
                    if q.case_sensitive { pat.to_string() } else { pat.to_lowercase() },
                    q.case_sensitive,
                ),
                NameMode::Regex => NameMatcher::Regex(
                    regex::RegexBuilder::new(pat)
                        .case_insensitive(!q.case_sensitive)
                        .build()
                        .map_err(|e| txt!("regular expression: {error}", error = e.to_string()))?,
                ),
            }
        };
        let content = if q.content.is_empty() {
            None
        } else {
            let p = if q.content_regex { q.content.clone() } else { regex::escape(&q.content) };
            Some(
                regex::bytes::RegexBuilder::new(&p)
                    .case_insensitive(!q.case_sensitive)
                    .multi_line(true)
                    .build()
                    .map_err(|e| txt!("search text: {error}", error = e.to_string()))?,
            )
        };
        let newer_than = q
            .modified_days
            .map(|d| SystemTime::now() - Duration::from_secs(d as u64 * 86_400));
        Ok(Self { name, content, q: q.clone(), newer_than })
    }

    pub fn name_matches(&self, name: &str) -> bool {
        match &self.name {
            NameMatcher::All => true,
            NameMatcher::Glob(g) => g.is_match(name),
            NameMatcher::Contains(t, true) => name.contains(t.as_str()),
            NameMatcher::Contains(t, false) => name.to_lowercase().contains(t.as_str()),
            NameMatcher::Regex(r) => r.is_match(name),
        }
    }

    /// Kind, size and date filters (content is checked separately).
    pub fn meta_matches(&self, e: &Entry) -> bool {
        let is_dir = e.is_dir_like();
        match self.q.kind {
            KindFilter::Files if is_dir => return false,
            KindFilter::Dirs if !is_dir => return false,
            _ => {}
        }
        // Content search only makes sense for files.
        if self.content.is_some() && is_dir {
            return false;
        }
        if !is_dir {
            if self.q.min_size.is_some_and(|m| e.size < m) || self.q.max_size.is_some_and(|m| e.size > m) {
                return false;
            }
        } else if self.q.min_size.is_some() || self.q.max_size.is_some() {
            return false;
        }
        if let Some(t) = self.newer_than {
            if e.modified.is_none_or(|m| m < t) {
                return false;
            }
        }
        true
    }

    pub fn query(&self) -> &Query {
        &self.q
    }

    /// First matching line of `data` (1-based line number, text), if any.
    pub fn content_match(&self, data: &[u8]) -> Option<(u64, String)> {
        let re = self.content.as_ref()?;
        let m = re.find(data)?;
        let line_no = data[..m.start()].iter().filter(|b| **b == b'\n').count() as u64 + 1;
        let start = data[..m.start()].iter().rposition(|b| *b == b'\n').map(|p| p + 1).unwrap_or(0);
        let end = data[m.end()..].iter().position(|b| *b == b'\n').map(|p| m.end() + p).unwrap_or(data.len());
        Some((line_no, preview(&data[start..end])))
    }
}

fn preview(line: &[u8]) -> String {
    let s = String::from_utf8_lossy(line);
    let s = s.trim();
    if s.chars().count() > 200 {
        s.chars().take(200).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// Counts hits and enforces `max_results`. `emit` is called for each accepted hit.
pub struct Sink<'a> {
    emit: &'a (dyn Fn(Hit) + Send + Sync),
    count: AtomicUsize,
    max: usize,
    pub cancel: &'a AtomicBool,
}

impl<'a> Sink<'a> {
    pub fn new(emit: &'a (dyn Fn(Hit) + Send + Sync), max: usize, cancel: &'a AtomicBool) -> Self {
        Self { emit, count: AtomicUsize::new(0), max: max.max(1), cancel }
    }

    /// Returns false when the search should stop (limit reached or canceled).
    pub fn push(&self, hit: Hit) -> bool {
        if self.stopped() {
            return false;
        }
        let n = self.count.fetch_add(1, Ordering::Relaxed);
        if n >= self.max {
            return false;
        }
        (self.emit)(hit);
        n + 1 < self.max
    }

    pub fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.count.load(Ordering::Relaxed) >= self.max
    }

    fn outcome(&self, method: Method, started: Instant) -> Outcome {
        let n = self.count.load(Ordering::Relaxed);
        Outcome {
            method,
            hits: n.min(self.max),
            truncated: n >= self.max,
            canceled: self.cancel.load(Ordering::Relaxed) && n < self.max,
            elapsed: started.elapsed(),
        }
    }
}

pub(crate) fn rel_path(root: &str, path: &str) -> String {
    let r = path.strip_prefix(root).unwrap_or(path);
    r.trim_start_matches(['/', '\\']).to_string()
}

// ------------------------------------------------------------------ local

fn local_entry(path: &std::path::Path, meta: &std::fs::Metadata) -> Entry {
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
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        kind,
        link_to_dir: ft.is_symlink() && std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false),
        size: if ft.is_file() { meta.len() } else { 0 },
        modified: meta.modified().ok(),
        mode,
    }
}

/// Search the local filesystem with a parallel walker. Blocking: run it on a
/// blocking thread.
pub fn search_local(q: &Query, sink: &Sink) -> Result<Outcome, Text> {
    use grep_searcher::{BinaryDetection, SearcherBuilder};
    let started = Instant::now();
    let m = Matcher::new(q)?;
    let content_matcher = if q.content.is_empty() {
        None
    } else {
        let p = if q.content_regex { q.content.clone() } else { regex::escape(&q.content) };
        Some(
            grep_regex::RegexMatcherBuilder::new()
                .case_insensitive(!q.case_sensitive)
                .build(&p)
                .map_err(|e| txt!("search text: {error}", error = e.to_string()))?,
        )
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12);
    let walker = ignore::WalkBuilder::new(&q.root)
        .hidden(!q.hidden)
        .ignore(q.gitignore)
        .git_ignore(q.gitignore)
        .git_global(q.gitignore)
        .git_exclude(q.gitignore)
        .parents(q.gitignore)
        .follow_links(false)
        .threads(threads)
        .build_parallel();

    walker.run(|| {
        let m = &m;
        let content_matcher = content_matcher.clone();
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .build();
        Box::new(move |res| {
            use ignore::WalkState;
            if sink.stopped() {
                return WalkState::Quit;
            }
            let Ok(de) = res else { return WalkState::Continue };
            if de.depth() == 0 {
                return WalkState::Continue;
            }
            let name = de.file_name().to_string_lossy();
            if !m.name_matches(&name) {
                return WalkState::Continue;
            }
            let Ok(meta) = de.metadata() else { return WalkState::Continue };
            let entry = local_entry(de.path(), &meta);
            if !m.meta_matches(&entry) {
                return WalkState::Continue;
            }
            let mut line = None;
            if let Some(cm) = &content_matcher {
                if entry.kind != EntryKind::File {
                    return WalkState::Continue;
                }
                let mut found = None;
                let _ = searcher.search_path(
                    cm,
                    de.path(),
                    grep_searcher::sinks::Lossy(|lnum, text| {
                        found = Some((lnum, preview(text.as_bytes())));
                        Ok(false) // first match is enough
                    }),
                );
                match found {
                    Some(f) => line = Some(f),
                    None => return WalkState::Continue,
                }
            }
            let path = de.path().to_string_lossy().into_owned();
            let dir = de.path().parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let hit = Hit { rel: rel_path(&q.root, &path), dir, entry, line };
            if sink.push(hit) {
                WalkState::Continue
            } else {
                WalkState::Quit
            }
        })
    });
    Ok(sink.outcome(Method::Local, started))
}

// ------------------------------------------------------------- vfs walk

/// Files larger than this are not content-searched by the generic walker
/// (it has to download them).
pub const WALK_CONTENT_MAX: u64 = 32 * 1024 * 1024;

/// Generic breadth-first search over any backend (SFTP fallback).
pub async fn search_vfs(fs: &dyn Vfs, q: &Query, sink: &Sink<'_>) -> Result<Outcome, Text> {
    use tokio::io::AsyncReadExt;
    let started = Instant::now();
    let m = Matcher::new(q)?;
    let mut queue = std::collections::VecDeque::from([q.root.clone()]);
    while let Some(dir) = queue.pop_front() {
        if sink.stopped() {
            break;
        }
        let Ok(entries) = fs.list(&dir).await else { continue };
        for e in entries {
            if sink.stopped() {
                break;
            }
            if !q.hidden && e.name.starts_with('.') {
                continue;
            }
            if rolid_core::vfs::validate_name(&e.name).is_err() {
                continue; // never trust odd names from a server
            }
            let path = fs.join(&dir, &e.name);
            // Do not follow directory symlinks (loops).
            if e.kind == EntryKind::Dir {
                queue.push_back(path.clone());
            }
            if !m.name_matches(&e.name) || !m.meta_matches(&e) {
                continue;
            }
            let mut line = None;
            if m.content.is_some() {
                if e.kind != EntryKind::File || e.size > WALK_CONTENT_MAX {
                    continue;
                }
                let Ok(mut r) = fs.open_read(&path).await else { continue };
                let mut data = Vec::with_capacity(e.size as usize);
                if r.read_to_end(&mut data).await.is_err() || data[..data.len().min(8192)].contains(&0) {
                    continue; // unreadable or binary
                }
                match m.content_match(&data) {
                    Some(l) => line = Some(l),
                    None => continue,
                }
            }
            let hit = Hit { rel: rel_path(&q.root, &path), dir: dir.clone(), entry: e, line };
            if !sink.push(hit) {
                break;
            }
        }
    }
    Ok(sink.outcome(Method::Walk, started))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("").unwrap(), None);
        assert_eq!(parse_size("500").unwrap(), Some(500));
        assert_eq!(parse_size("10k").unwrap(), Some(10 * 1024));
        assert_eq!(parse_size("1.5M").unwrap(), Some(1_572_864));
        assert_eq!(parse_size("2 GB").unwrap(), Some(2 << 30));
        assert_eq!(parse_size("1,5 MiB").unwrap(), Some(1_572_864));
        assert!(parse_size("5x").is_err());
    }

    #[test]
    fn name_modes() {
        let m = |name: &str, mode, cs| {
            Matcher::new(&Query { name: name.into(), name_mode: mode, case_sensitive: cs, ..Default::default() }).unwrap()
        };
        let g = m("*.RS; *.toml", NameMode::Glob, false);
        assert!(g.name_matches("main.rs") && g.name_matches("Cargo.toml") && !g.name_matches("x.txt"));
        assert!(m("read", NameMode::Glob, false).name_matches("README.md"));
        assert!(!m("read", NameMode::Glob, true).name_matches("README.md"));
        assert!(m("me.M", NameMode::Contains, false).name_matches("README.md"));
        let r = m(r"^file\d+\.txt$", NameMode::Regex, false);
        assert!(r.name_matches("file10.txt") && !r.name_matches("file.txt"));
    }

    #[test]
    fn content_line() {
        let m = Matcher::new(&Query { content: "Hello".into(), ..Default::default() }).unwrap();
        assert_eq!(m.content_match(b"a\nb\n  say hello world \nc"), Some((3, "say hello world".into())));
        assert_eq!(m.content_match(b"nothing"), None);
    }

    fn tree() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rolid-search-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src/deep")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main() {\n    println!(\"Hello\");\n}\n").unwrap();
        std::fs::write(d.join("src/deep/lib.rs"), "pub fn x() {}\n").unwrap();
        std::fs::write(d.join("README.md"), "# Project\nhello world\n").unwrap();
        std::fs::write(d.join(".hidden/secret.rs"), "hello").unwrap();
        std::fs::write(d.join("data.bin"), [0u8, 1, 2, b'a', b'h', b'o', b'j']).unwrap();
        std::fs::write(d.join("big.log"), vec![b'x'; 50_000]).unwrap();
        d
    }

    fn run_local(q: Query) -> Vec<Hit> {
        let hits = Mutex::new(Vec::new());
        let emit = |h: Hit| hits.lock().unwrap().push(h);
        let cancel = AtomicBool::new(false);
        let sink = Sink::new(&emit, q.max_results, &cancel);
        search_local(&q, &sink).unwrap();
        let mut v = hits.into_inner().unwrap();
        v.sort_by(|a, b| a.rel.cmp(&b.rel));
        v
    }

    async fn run_vfs(q: Query) -> Vec<Hit> {
        let hits = Mutex::new(Vec::new());
        let emit = |h: Hit| hits.lock().unwrap().push(h);
        let cancel = AtomicBool::new(false);
        let sink = Sink::new(&emit, q.max_results, &cancel);
        search_vfs(&rolid_local::LocalFs, &q, &sink).await.unwrap();
        let mut v = hits.into_inner().unwrap();
        v.sort_by(|a, b| a.rel.cmp(&b.rel));
        v
    }

    fn rels(h: &[Hit]) -> Vec<String> {
        h.iter().map(|h| h.rel.replace('\\', "/")).collect()
    }

    #[tokio::test]
    async fn local_and_walk_agree() {
        let d = tree();
        let root = d.to_string_lossy().into_owned();
        let cases = [
            (Query { root: root.clone(), name: "*.rs".into(), ..Default::default() }, vec!["src/deep/lib.rs", "src/main.rs"]),
            (
                Query { root: root.clone(), name: "*.rs".into(), hidden: true, ..Default::default() },
                vec![".hidden/secret.rs", "src/deep/lib.rs", "src/main.rs"],
            ),
            // content: case-insensitive, binary file skipped
            (Query { root: root.clone(), content: "HELLO".into(), ..Default::default() }, vec!["README.md", "src/main.rs"]),
            (Query { root: root.clone(), kind: KindFilter::Dirs, ..Default::default() }, vec!["src", "src/deep"]),
            (Query { root: root.clone(), min_size: Some(10_000), ..Default::default() }, vec!["big.log"]),
        ];
        for (q, want) in cases {
            assert_eq!(rels(&run_local(q.clone())), want, "local {q:?}");
            assert_eq!(rels(&run_vfs(q.clone()).await), want, "walk {q:?}");
        }
        // line preview
        let h = run_local(Query { root: root.clone(), content: "println".into(), ..Default::default() });
        assert_eq!(h[0].line, Some((2, "println!(\"Hello\");".into())));
        // limit
        let h = run_local(Query { root: root.clone(), max_results: 2, ..Default::default() });
        assert_eq!(h.len(), 2);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
