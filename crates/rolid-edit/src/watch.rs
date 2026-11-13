//! Detecting that the editor saved the file.
//!
//! The *directory* is watched, not the file: many editors save atomically (write a
//! temporary file, then rename it over the original), which replaces the inode.
//! Every create / modify / rename event that names our file starts a debounce
//! timer; when it settles, the file's [`Fingerprint`] (size, mtime, content hash)
//! decides whether it really changed.

use std::ffi::{OsStr, OsString};
use std::hash::Hasher;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use notify::{EventKind, RecursiveMode, Watcher};

/// Default quiet period after the last event.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// What a saved file looks like; compared to tell real changes from noise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub hash: u64,
}

impl Fingerprint {
    /// Content differs (the mtime alone, e.g. "save" without edits, is not a change).
    pub fn content_differs(&self, other: &Fingerprint) -> bool {
        self.size != other.size || self.hash != other.hash
    }
}

/// Read the file and compute its fingerprint.
pub fn fingerprint(path: &Path) -> std::io::Result<Fingerprint> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let md = f.metadata()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        size += n as u64;
    }
    Ok(Fingerprint { size, mtime: md.modified().ok(), hash: h.finish() })
}

/// Waits until events stop arriving for `delay`.
#[derive(Debug, Clone)]
pub struct Debounce {
    delay: Duration,
    last: Option<Instant>,
}

impl Default for Debounce {
    fn default() -> Self {
        Self::new(DEBOUNCE)
    }
}

impl Debounce {
    pub fn new(delay: Duration) -> Self {
        Self { delay, last: None }
    }

    /// An event arrived at `now`.
    pub fn event(&mut self, now: Instant) {
        self.last = Some(now);
    }

    pub fn pending(&self) -> bool {
        self.last.is_some()
    }

    /// Time left until the quiet period ends (`None` = nothing pending).
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.last.map(|t| (t + self.delay).saturating_duration_since(now))
    }

    /// True once (and resets) when the quiet period after the last event has passed.
    pub fn fire(&mut self, now: Instant) -> bool {
        match self.last {
            Some(t) if now.saturating_duration_since(t) >= self.delay => {
                self.last = None;
                true
            }
            _ => false,
        }
    }
}

/// Does this event concern our file? Creations, modifications and renames that name
/// it (as source or target) count; pure access does not.
pub fn relevant(event: &notify::Event, file_name: &OsStr) -> bool {
    let kind_ok = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any);
    kind_ok && event.paths.iter().any(|p| p.file_name() == Some(file_name))
}

/// Watches one directory and calls `on_change` for events concerning one file.
pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn new(
        dir: &Path,
        file_name: OsString,
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<Self> {
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if relevant(&ev, &file_name) {
                    on_change();
                }
            }
        })?;
        w.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: w })
    }
}

/// Watches the entries of one directory (not deeper): any created, changed,
/// renamed or removed entry calls `on_change`; reading does not.
pub struct DirWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl DirWatcher {
    pub fn new(dir: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
            Ok(ev) if matches!(ev.kind, EventKind::Access(_)) => {}
            // An error (e.g. the queue overflowed) also means "look again".
            _ => on_change(),
        })?;
        w.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: w })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn debounce_waits_for_quiet() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(300));
        assert!(!d.fire(t0));
        d.event(t0);
        assert!(!d.fire(t0 + Duration::from_millis(200)));
        d.event(t0 + Duration::from_millis(250)); // another write: timer restarts
        assert!(!d.fire(t0 + Duration::from_millis(500)));
        assert_eq!(d.remaining(t0 + Duration::from_millis(500)), Some(Duration::from_millis(50)));
        assert!(d.fire(t0 + Duration::from_millis(550)));
        assert!(!d.fire(t0 + Duration::from_millis(900)), "fires once");
        assert!(!d.pending());
    }

    #[test]
    fn fingerprint_tells_content_changes() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.txt");
        std::fs::write(&p, "one").unwrap();
        let a = fingerprint(&p).unwrap();
        std::fs::write(&p, "one").unwrap();
        assert!(!a.content_differs(&fingerprint(&p).unwrap()), "same content");
        std::fs::write(&p, "onE").unwrap();
        assert!(a.content_differs(&fingerprint(&p).unwrap()), "same size, other content");
    }

    fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
        let mut e = notify::Event::new(kind);
        for p in paths {
            e = e.add_path(p.into());
        }
        e
    }

    #[test]
    fn relevant_events() {
        use notify::event::{AccessKind, CreateKind, ModifyKind, RenameMode};
        let name = OsStr::new("index.php");
        // Atomic save: temp file renamed over ours.
        assert!(relevant(
            &event(EventKind::Modify(ModifyKind::Name(RenameMode::Both)), &["/d/.goutputstream-X1", "/d/index.php"]),
            name
        ));
        assert!(relevant(&event(EventKind::Modify(ModifyKind::Name(RenameMode::To)), &["/d/index.php"]), name));
        assert!(relevant(&event(EventKind::Create(CreateKind::File), &["/d/index.php"]), name));
        // Only the editor's temp / backup files: not yet.
        assert!(!relevant(&event(EventKind::Create(CreateKind::File), &["/d/.index.php.swp"]), name));
        assert!(!relevant(&event(EventKind::Modify(ModifyKind::Any), &["/d/index.php~"]), name));
        // Reading the file is not a change.
        assert!(!relevant(&event(EventKind::Access(AccessKind::Any), &["/d/index.php"]), name));
    }

    /// Wait (max. 5 s) until `cond` holds.
    fn wait_for(cond: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn plain_and_atomic_saves_are_detected() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("config.yaml");
        std::fs::write(&file, "a: 1\n").unwrap();
        let before = fingerprint(&file).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let _w = FileWatcher::new(t.path(), "config.yaml".into(), move || {
            h.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Unrelated file in the same directory: ignored.
        std::fs::write(t.path().join("other.txt"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        // Plain save (same inode).
        std::fs::write(&file, "a: 2\n").unwrap();
        assert!(wait_for(|| hits.load(Ordering::SeqCst) > 0), "plain save");
        let plain = fingerprint(&file).unwrap();
        assert!(before.content_differs(&plain));

        // Atomic save: write a temp file, rename it over the original (new inode).
        let n = hits.load(Ordering::SeqCst);
        let tmp = t.path().join(".config.yaml.tmp123");
        std::fs::write(&tmp, "a: 3\nb: 4\n").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        assert!(wait_for(|| hits.load(Ordering::SeqCst) > n), "atomic save");
        let atomic = fingerprint(&file).unwrap();
        assert!(plain.content_differs(&atomic));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a: 3\nb: 4\n");

        // The watch survives the replaced inode: the next plain save is seen too.
        let n = hits.load(Ordering::SeqCst);
        std::fs::write(&file, "a: 5\n").unwrap();
        assert!(wait_for(|| hits.load(Ordering::SeqCst) > n), "save after atomic replace");
    }

    #[test]
    fn a_folder_watch_sees_new_changed_renamed_and_removed_entries() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("sub")).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let _w = DirWatcher::new(t.path(), move || {
            h.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let mut seen = 0;
        let mut step = |what: &str, f: &dyn Fn()| {
            f();
            assert!(wait_for(|| hits.load(Ordering::SeqCst) > seen), "{what}");
            std::thread::sleep(Duration::from_millis(50));
            seen = hits.load(Ordering::SeqCst);
        };
        let p = t.path().to_path_buf();
        step("new file", &|| std::fs::write(p.join("a.txt"), "x").unwrap());
        step("changed file", &|| std::fs::write(p.join("a.txt"), "xyz").unwrap());
        step("renamed", &|| std::fs::rename(p.join("a.txt"), p.join("b.txt")).unwrap());
        step("removed", &|| std::fs::remove_file(p.join("b.txt")).unwrap());
        // Deeper levels are not watched.
        std::thread::sleep(Duration::from_millis(200));
        let n = hits.load(Ordering::SeqCst);
        std::fs::write(t.path().join("sub").join("deep.txt"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(hits.load(Ordering::SeqCst), n, "not recursive");
    }
}
