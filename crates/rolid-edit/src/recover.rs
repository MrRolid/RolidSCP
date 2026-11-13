//! Leftovers after a crash.
//!
//! Every running instance keeps `<base>/instance-<pid>` (its PID and process start
//! time). Every remote edit keeps `<base>/<dir>.meta` next to its private directory:
//! which instance owns it, where the file came from and a hash of the content that
//! is on the server (after download / the last upload).
//!
//! At start-up [`scan`] looks at edits whose owner is no longer running (the PID
//! does not exist, or belongs to another program or a later process):
//! - content equal to the hash (nothing unsaved) → deleted;
//! - content differs → returned as an [`Orphan`] for the user to decide
//!   (upload / open the folder / discard). Nothing is deleted then.
//!
//! Edits of running instances are never touched.
//!
//! Files dragged out of a server or an archive to another application are
//! downloaded into `<base>/drag-<random>` (0700) with `<base>/drag-<random>.owner`
//! (the owning instance). [`scan`] deletes those of instances that no longer run.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use rolid_text::{txt, Text};

pub const META_EXT: &str = "meta";
pub const DRAG_PREFIX: &str = "drag-";
pub const OWNER_EXT: &str = "owner";
const INSTANCE_PREFIX: &str = "instance-";

/// Identity of a process: PID and start time (to tell a reused PID apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcId {
    pub pid: u32,
    /// Start time in clock ticks since boot (Linux `/proc/<pid>/stat`), if known.
    pub start: Option<u64>,
}

impl ProcId {
    pub fn current() -> ProcId {
        let pid = std::process::id();
        ProcId { pid, start: proc_start(pid) }
    }
}

/// Start time of a process (Linux: field 22 of `/proc/<pid>/stat`).
fn proc_start(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) is in parentheses and may contain spaces.
    let rest = &s[s.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn proc_name(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
}

/// Is the process `id` still running *this* program? Without `/proc` (not Linux)
/// we cannot tell, so we assume it is (and never touch its files).
pub fn is_running(id: &ProcId) -> bool {
    if !Path::new("/proc/self/stat").exists() {
        return true;
    }
    if !Path::new(&format!("/proc/{}", id.pid)).exists() {
        return false;
    }
    let same_program = match (proc_name(id.pid), proc_name(std::process::id())) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let same_process = match (id.start, proc_start(id.pid)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    same_program && same_process
}

/// What is known about one edit (`<dir>.meta`, TOML).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditMeta {
    pub owner: ProcId,
    /// Saved server profile, if the file came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    pub remote_path: String,
    /// File name inside the edit directory.
    pub file: String,
    /// The remote file when it was downloaded / last uploaded.
    pub remote_size: Option<u64>,
    pub remote_mtime: Option<u64>,
    pub mode: Option<u32>,
    pub production: bool,
    /// SHA-256 (hex) of the content that is on the server.
    pub hash: String,
}

/// SHA-256 of a file, hex.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `<base>/<dir>.meta` for an edit directory.
pub fn meta_path(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(META_EXT);
    dir.with_file_name(name)
}

/// Write the metadata of the edit in `dir` (atomically, 0600).
pub fn write_meta(dir: &Path, meta: &EditMeta) -> std::io::Result<()> {
    let text = toml::to_string(meta).map_err(std::io::Error::other)?;
    write_private(&meta_path(dir), text.as_bytes())
}

pub fn read_meta(dir: &Path) -> std::io::Result<EditMeta> {
    let text = std::fs::read_to_string(meta_path(dir))?;
    toml::from_str(&text).map_err(std::io::Error::other)
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(data)?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// While alive, `<base>/instance-<pid>` says this instance is running.
pub struct Instance {
    path: PathBuf,
    pub id: ProcId,
}

impl Instance {
    /// Register the running instance (the base directory must exist).
    pub fn register(base: &Path) -> std::io::Result<Instance> {
        let id = ProcId::current();
        let path = base.join(format!("{INSTANCE_PREFIX}{}", id.pid));
        let text = toml::to_string(&id).map_err(std::io::Error::other)?;
        write_private(&path, text.as_bytes())?;
        Ok(Instance { path, id })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// An edit left behind by a crashed instance, with changes not on the server.
#[derive(Debug, Clone)]
pub struct Orphan {
    pub dir: PathBuf,
    pub file: PathBuf,
    pub meta: EditMeta,
}

impl Orphan {
    /// Delete the directory and its metadata.
    pub fn discard(&self) -> std::io::Result<()> {
        remove_edit(&self.dir)
    }
}

/// Delete an edit directory together with its `.meta`.
pub fn remove_edit(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    match std::fs::remove_file(meta_path(dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// A private folder for files dragged out to another application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragDir {
    pub dir: PathBuf,
}

impl DragDir {
    fn owner_path(dir: &Path) -> PathBuf {
        let mut p = dir.as_os_str().to_os_string();
        p.push(format!(".{OWNER_EXT}"));
        PathBuf::from(p)
    }

    /// Delete the folder with everything in it, and its owner file.
    pub fn remove(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        match std::fs::remove_file(Self::owner_path(&self.dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Create `<base>/drag-<random>` (0700) owned by `me`. The owner file is
/// written first, so a crash never leaves a folder nobody would clean.
pub fn create_drag_dir(base: &Path, me: &ProcId) -> Result<DragDir, Text> {
    crate::temp::ensure_base(base).map_err(|e| txt!("{path}: {error}", path = base, error = rolid_core::VfsError::Io(e).text()))?;
    let owner = toml::to_string(me).map_err(rolid_core::vfs::raw_text)?;
    for _ in 0..16 {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).map_err(rolid_core::vfs::raw_text)?;
        let hex: String = r.iter().map(|b| format!("{b:02x}")).collect();
        let dir = base.join(format!("{DRAG_PREFIX}{hex}"));
        let owner_path = DragDir::owner_path(&dir);
        if dir.exists() || owner_path.exists() {
            continue;
        }
        write_private(&owner_path, owner.as_bytes()).map_err(|e| txt!("{path}: {error}", path = &owner_path, error = e.to_string()))?;
        match crate::temp::mkdir_private(&dir) {
            Ok(()) => return Ok(DragDir { dir }),
            Err(e) => {
                let _ = std::fs::remove_file(&owner_path);
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(txt!("{path}: {error}", path = &dir, error = e.to_string()));
            }
        }
    }
    Err(txt!("cannot create a temporary folder"))
}

/// Result of [`scan`].
#[derive(Debug, Default)]
pub struct Scan {
    /// Unchanged leftovers that were deleted.
    pub removed: usize,
    /// Leftovers with unsaved changes; now owned by `me` (so a parallel start
    /// does not offer them too).
    pub orphans: Vec<Orphan>,
}

/// Process leftovers of instances that no longer run (see the module docs).
/// `alive` decides whether an owner still runs ([`is_running`] in the app).
pub fn scan(base: &Path, me: &ProcId, alive: impl Fn(&ProcId) -> bool) -> Scan {
    let mut out = Scan::default();
    let Ok(rd) = std::fs::read_dir(base) else { return out };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        // Stale instance markers.
        if let Some(pid) = name.strip_prefix(INSTANCE_PREFIX) {
            let id = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| toml::from_str::<ProcId>(&t).ok())
                .unwrap_or(ProcId { pid: pid.parse().unwrap_or(0), start: None });
            if id != *me && !alive(&id) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        // Folders of dragged-out files: deleted once their instance is gone.
        if let Some(stem) = name.strip_suffix(&format!(".{OWNER_EXT}")).filter(|s| s.starts_with(DRAG_PREFIX)) {
            let owner = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str::<ProcId>(&t).ok());
            if owner.is_some_and(|o| o != *me && !alive(&o)) && (DragDir { dir: base.join(stem) }).remove().is_ok() {
                out.removed += 1;
            }
            continue;
        }
        let Some(stem) = name.strip_suffix(&format!(".{META_EXT}")) else { continue };
        let dir = base.join(stem);
        let Ok(meta) = read_meta(&dir) else { continue }; // unknown format: leave it
        if meta.owner == *me || alive(&meta.owner) {
            continue; // a running instance's edit: never touch
        }
        if crate::temp::local_name(&meta.file).is_err() {
            continue;
        }
        let file = dir.join(&meta.file);
        let unchanged = match hash_file(&file) {
            Ok(h) => h == meta.hash,
            // No file (or no directory): nothing that could be lost.
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if unchanged {
            if remove_edit(&dir).is_ok() {
                out.removed += 1;
            }
            continue;
        }
        let mut meta = meta;
        meta.owner = *me;
        let _ = write_meta(&dir, &meta);
        out.orphans.push(Orphan { dir, file, meta });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(owner: ProcId, file: &str, hash: String) -> EditMeta {
        EditMeta {
            owner,
            profile: Some("p1".into()),
            host: "example.org".into(),
            port: 22,
            user: "u".into(),
            key_path: None,
            remote_path: format!("/srv/{file}"),
            file: file.into(),
            remote_size: Some(5),
            remote_mtime: Some(1_700_000_000),
            mode: Some(0o640),
            production: false,
            hash,
        }
    }

    /// An edit dir as the app leaves it: file + meta.
    fn edit(base: &Path, name: &str, owner: ProcId, server: &str, local: &str) -> PathBuf {
        let d = crate::temp::create(base, name).unwrap();
        std::fs::write(&d.file, server).unwrap();
        let h = hash_file(&d.file).unwrap();
        write_meta(&d.dir, &meta(owner, name, h)).unwrap();
        std::fs::write(&d.file, local).unwrap();
        d.dir
    }

    #[test]
    fn drag_folders_are_private_and_cleaned_after_a_crash() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("edit");
        let me = ProcId { pid: 100, start: Some(1) };
        let dead = ProcId { pid: 200, start: Some(5) };
        let live = ProcId { pid: 300, start: Some(7) };
        let alive = |p: &ProcId| *p == live;
        let mine = create_drag_dir(&base, &me).unwrap();
        let crashed = create_drag_dir(&base, &dead).unwrap();
        let running = create_drag_dir(&base, &live).unwrap();
        assert_ne!(mine, crashed);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&mine.dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        }
        std::fs::create_dir(crashed.dir.join("folder")).unwrap();
        std::fs::write(crashed.dir.join("folder/a.txt"), "x").unwrap();
        std::fs::write(mine.dir.join("b.txt"), "y").unwrap();
        let s = scan(&base, &me, alive);
        assert_eq!(s.removed, 1);
        assert!(s.orphans.is_empty());
        assert!(!crashed.dir.exists() && !DragDir::owner_path(&crashed.dir).exists(), "a crashed instance's folder goes");
        assert!(mine.dir.join("b.txt").exists(), "own folder untouched");
        assert!(running.dir.exists(), "a running instance's folder untouched");
        // After the drop (or at exit) the app removes its own folder.
        mine.remove().unwrap();
        assert!(!mine.dir.exists() && !DragDir::owner_path(&mine.dir).exists());
        mine.remove().unwrap(); // twice is fine
    }

    #[test]
    fn crashed_leftovers_are_cleaned_or_offered() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("edit");
        let me = ProcId { pid: 100, start: Some(1) };
        let dead = ProcId { pid: 200, start: Some(5) };
        let live = ProcId { pid: 300, start: Some(7) };
        let alive = |p: &ProcId| *p == live;

        let clean = edit(&base, "a.txt", dead, "server", "server");
        let dirty = edit(&base, "b.txt", dead, "server", "changed locally");
        let running = edit(&base, "c.txt", live, "server", "changed by a running instance");
        let mine = edit(&base, "d.txt", me, "server", "my own unsaved change");
        // Instance markers.
        std::fs::write(base.join("instance-200"), toml::to_string(&dead).unwrap()).unwrap();
        std::fs::write(base.join("instance-300"), toml::to_string(&live).unwrap()).unwrap();
        // A 0.5.0 leftover without metadata: unknown owner, left alone.
        let old = crate::temp::create(&base, "old.txt").unwrap();
        std::fs::write(&old.file, "x").unwrap();

        let s = scan(&base, &me, alive);
        assert_eq!(s.removed, 1);
        assert!(!clean.exists() && !meta_path(&clean).exists(), "unchanged copy deleted");
        assert_eq!(s.orphans.len(), 1);
        let o = &s.orphans[0];
        assert_eq!(o.dir, dirty);
        assert_eq!(o.meta.remote_path, "/srv/b.txt");
        assert_eq!(std::fs::read_to_string(&o.file).unwrap(), "changed locally", "nothing deleted");
        assert_eq!(read_meta(&dirty).unwrap().owner, me, "taken over");
        assert!(running.exists() && meta_path(&running).exists(), "running instance untouched");
        assert!(mine.exists());
        assert!(old.dir.exists(), "no metadata: untouched");
        assert!(!base.join("instance-200").exists(), "stale marker removed");
        assert!(base.join("instance-300").exists());

        // A second scan by the same instance: the orphan is its own now.
        assert!(scan(&base, &me, alive).orphans.is_empty());
        o.discard().unwrap();
        assert!(!dirty.exists() && !meta_path(&dirty).exists());
    }

    #[test]
    fn reused_pid_of_another_program_is_not_alive() {
        let me = ProcId::current();
        if me.start.is_none() {
            return; // no /proc
        }
        assert!(is_running(&me));
        assert!(!is_running(&ProcId { pid: me.pid, start: me.start.map(|s| s + 1) }), "same PID, later start");
        assert!(!is_running(&ProcId { pid: 1, start: None }) || proc_name(1) == proc_name(me.pid), "PID 1 is another program");
        assert!(!is_running(&ProcId { pid: u32::MAX - 1, start: None }), "no such PID");
    }

    #[test]
    fn instance_marker_lives_while_registered() {
        let t = tempfile::tempdir().unwrap();
        let i = Instance::register(t.path()).unwrap();
        let p = t.path().join(format!("instance-{}", i.id.pid));
        assert!(p.exists());
        drop(i);
        assert!(!p.exists());
    }
}
