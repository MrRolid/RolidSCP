//! Moving local files to the desktop's trash (FreeDesktop.org Trash
//! specification 1.0), where file managers (Files, Dolphin, Thunar, Caja…)
//! show them and can put them back.
//!
//! - A file on the disk of the home folder goes to `$XDG_DATA_HOME/Trash`
//!   (`~/.local/share/Trash`).
//! - A file on another disk goes to that disk's trash: `$topdir/.Trash/$uid`
//!   when the administrator made `$topdir/.Trash` (a real folder with the
//!   sticky bit), else `$topdir/.Trash-$uid`. Files are never copied across
//!   disks: when no trash can be used there, nothing is moved and the error
//!   says so ([`TrashError::NoTrash`]).
//! - For each file, `info/NAME.trashinfo` is created first (exclusively: two
//!   programs never take the same name), then the file is renamed into
//!   `files/NAME`. When the rename fails, the info file is removed again.

use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Why an item was not moved to the trash.
#[derive(Debug)]
pub enum TrashError {
    /// No trash can be used on the disk of the item (read-only, no rights,
    /// a `.Trash` that is not safe).
    NoTrash { disk: PathBuf, why: String },
    Io(std::io::Error),
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrashError::NoTrash { disk, why } => write!(f, "no trash on the disk {}: {why}", disk.display()),
            TrashError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for TrashError {
    fn from(e: std::io::Error) -> Self {
        TrashError::Io(e)
    }
}

/// Where an item went.
#[derive(Debug, Clone)]
pub struct Trashed {
    /// The item now (in `…/Trash/files`).
    pub file: PathBuf,
    /// Its `.trashinfo`.
    pub info: PathBuf,
}

/// The trash of the home folder: `$XDG_DATA_HOME/Trash`, by default
/// `~/.local/share/Trash` (a relative `XDG_DATA_HOME` is ignored, as the XDG
/// specification says).
pub fn home_trash() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    data.or_else(|| std::env::home_dir().map(|h| h.join(".local/share"))).map(|d| d.join("Trash"))
}

/// Move `path` (a file, a folder with its contents, a link: never followed)
/// to the trash.
pub fn trash(path: &Path) -> Result<Trashed, TrashError> {
    let home = home_trash().ok_or_else(|| std::io::Error::other("no home folder"))?;
    trash_with(path, &home)
}

/// [`trash`] with the trash of the home folder given.
pub fn trash_with(path: &Path, home_trash: &Path) -> Result<Trashed, TrashError> {
    let path = absolute(path)?;
    let meta = std::fs::symlink_metadata(&path)?;
    let parent = path.parent().ok_or_else(|| std::io::Error::other("the root folder cannot be moved to the trash"))?;
    let dev = std::fs::metadata(parent)?.dev();
    if meta.is_dir() && meta.dev() != dev {
        return Err(std::io::Error::other("a mount point cannot be moved to the trash").into());
    }
    if device_of(home_trash)? == dev {
        let dir = ensure_trash(home_trash)?;
        return put(&path, &dir, &path);
    }
    let top = mount_point(parent, dev)?;
    let dir = disk_trash(&top).map_err(|why| TrashError::NoTrash { disk: top.clone(), why })?;
    // On another disk the original place is written relative to the disk.
    let rel = path.strip_prefix(&top).map(Path::to_path_buf).unwrap_or_else(|_| path.clone());
    put(&path, &dir, &rel)
}

/// `path` made absolute without following its last part (it may be a link).
fn absolute(path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| std::io::Error::other("no file name"))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok(std::fs::canonicalize(parent)?.join(name))
}

/// The device of `p`, or of its nearest existing folder above.
fn device_of(p: &Path) -> std::io::Result<u64> {
    let mut cur = Some(p);
    while let Some(c) = cur {
        match std::fs::metadata(c) {
            Ok(m) => return Ok(m.dev()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => cur = c.parent(),
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no folder"))
}

/// The top folder of the disk `dir` is on.
fn mount_point(dir: &Path, dev: u64) -> std::io::Result<PathBuf> {
    let mut top = std::fs::canonicalize(dir)?;
    while let Some(up) = top.parent() {
        if std::fs::metadata(up)?.dev() != dev {
            break;
        }
        top = up.to_path_buf();
    }
    Ok(top)
}

/// `files` and `info` of a trash folder, made when missing (0700).
fn ensure_trash(dir: &Path) -> std::io::Result<PathBuf> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true).mode(0o700);
    b.create(dir.join("files"))?;
    b.create(dir.join("info"))?;
    Ok(dir.to_path_buf())
}

/// The trash of the disk with top folder `top` (see the module text).
fn disk_trash(top: &Path) -> Result<PathBuf, String> {
    let uid = rustix::process::getuid().as_raw();
    let admin = top.join(".Trash");
    if let Ok(m) = std::fs::symlink_metadata(&admin) {
        // Only a real folder with the sticky bit; otherwise it is skipped (the
        // specification: "the implementation also MUST report this").
        if m.is_dir() && m.permissions().mode() & 0o1000 != 0 {
            let mine = admin.join(uid.to_string());
            if ensure_owned(&mine, uid).is_ok() {
                return Ok(mine);
            }
        }
    }
    let own = top.join(format!(".Trash-{uid}"));
    ensure_owned(&own, uid).map_err(|e| e.to_string())
}

/// A trash folder of user `uid`: made when missing, and when it exists, a
/// real folder of that user (not a link someone put there).
fn ensure_owned(dir: &Path, uid: u32) -> std::io::Result<PathBuf> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let m = std::fs::symlink_metadata(dir)?;
    if !m.is_dir() || m.uid() != uid {
        return Err(std::io::Error::other(format!("{} is not a folder of this user", dir.display())));
    }
    ensure_trash(dir)
}

/// `%`-encode a path for the `Path=` key (RFC 2396: everything but the
/// unreserved characters and `/`).
pub fn encode_path(p: &Path) -> String {
    let mut s = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Create the info file under a free name, then move the item.
fn put(path: &Path, trash: &Path, original: &Path) -> Result<Trashed, TrashError> {
    let name = path.file_name().unwrap_or(OsStr::new("item"));
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    let text = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", encode_path(original));
    for n in 1..10_000u32 {
        let mut cand = name.to_os_string();
        if n > 1 {
            cand.push(format!(".{n}"));
        }
        let file = trash.join("files").join(&cand);
        let mut info_name = cand.clone();
        info_name.push(".trashinfo");
        let info = trash.join("info").join(info_name);
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&info) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        if std::fs::symlink_metadata(&file).is_ok() {
            // A file without its info (left by another program): keep both.
            drop(f);
            let _ = std::fs::remove_file(&info);
            continue;
        }
        let written = f.write_all(text.as_bytes()).and_then(|_| f.sync_all());
        drop(f);
        let moved = written.and_then(|_| std::fs::rename(path, &file));
        if let Err(e) = moved {
            let _ = std::fs::remove_file(&info);
            return Err(e.into());
        }
        return Ok(Trashed { file, info });
    }
    Err(std::io::Error::other("no free name in the trash").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        // Under target/: the same disk as the trash below.
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-trash").join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn info(t: &Trashed) -> String {
        std::fs::read_to_string(&t.info).unwrap()
    }

    #[test]
    fn a_file_and_a_folder_go_to_the_home_trash_with_their_place() {
        let d = tmp("home");
        let trash = d.join("data/Trash");
        let f = d.join("a file ž.txt");
        std::fs::write(&f, b"x").unwrap();
        let t = trash_with(&f, &trash).unwrap();
        assert!(!f.exists());
        assert_eq!(std::fs::read(&t.file).unwrap(), b"x");
        assert_eq!(t.file, trash.join("files/a file ž.txt"));
        let i = info(&t);
        assert!(i.starts_with("[Trash Info]\nPath=/"), "{i}");
        assert!(i.contains("/a%20file%20%C5%BE.txt\n"), "{i}");
        assert!(i.contains("\nDeletionDate=20"), "{i}");
        assert_eq!(std::fs::metadata(trash.join("files")).unwrap().permissions().mode() & 0o777, 0o700);
        // A folder with its contents; a link is moved, not what it points to.
        let dir = d.join("dir");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/in.txt"), b"y").unwrap();
        std::os::unix::fs::symlink(&dir, d.join("link")).unwrap();
        let t = trash_with(&d.join("link"), &trash).unwrap();
        assert!(std::fs::symlink_metadata(&t.file).unwrap().file_type().is_symlink());
        assert!(dir.join("sub/in.txt").exists());
        let t = trash_with(&dir, &trash).unwrap();
        assert_eq!(std::fs::read(t.file.join("sub/in.txt")).unwrap(), b"y");
        assert!(!dir.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_same_name_twice_gets_a_number() {
        let d = tmp("names");
        let trash = d.join("Trash");
        for n in 0..3 {
            std::fs::write(d.join("same.txt"), format!("{n}")).unwrap();
            trash_with(&d.join("same.txt"), &trash).unwrap();
        }
        // A file without its info (left by another program) is not overwritten.
        std::fs::write(trash.join("files/same.txt.4"), b"other").unwrap();
        std::fs::write(d.join("same.txt"), b"3").unwrap();
        let t = trash_with(&d.join("same.txt"), &trash).unwrap();
        assert_eq!(t.file, trash.join("files/same.txt.5"));
        for (name, text) in [("same.txt", "0"), ("same.txt.2", "1"), ("same.txt.3", "2"), ("same.txt.4", "other"), ("same.txt.5", "3")] {
            assert_eq!(std::fs::read_to_string(trash.join("files").join(name)).unwrap(), text);
        }
        assert!(!trash.join("info/same.txt.4.trashinfo").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn nothing_is_left_when_the_move_fails() {
        let d = tmp("fail");
        let trash = d.join("Trash");
        assert!(trash_with(&d.join("missing"), &trash).is_err());
        // A folder that cannot be left: no info file stays behind.
        let ro = d.join("ro");
        std::fs::create_dir_all(&ro).unwrap();
        std::fs::write(ro.join("f"), b"x").unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = trash_with(&ro.join("f"), &trash);
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        if rustix::process::geteuid().as_raw() != 0 {
            assert!(r.is_err());
            assert!(ro.join("f").exists());
            assert_eq!(std::fs::read_dir(trash.join("info")).unwrap().count(), 0);
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn paths_are_encoded_for_the_info_file() {
        assert_eq!(encode_path(Path::new("/home/a b/50%/ž#?.txt")), "/home/a%20b/50%25/%C5%BE%23%3F.txt");
        assert_eq!(encode_path(Path::new("rel/x-y_z.(1)~")), "rel/x-y_z.(1)~");
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(encode_path(Path::new(OsStr::from_bytes(b"/bad\xff"))), "/bad%FF");
    }

    /// On a disk other than the home trash's (here: the temporary folder,
    /// when it is another disk, as a tmpfs is): the disk's own trash, with the
    /// place written relative to the disk.
    #[test]
    fn another_disk_has_its_own_trash() {
        let d = tmp("other");
        let home_trash = d.join("Trash");
        let other = std::env::temp_dir().join(format!("rolid-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(&other).unwrap();
        if device_of(&other).unwrap() == device_of(&d).unwrap() {
            eprintln!("SKIP: the temporary folder is on the same disk");
            let _ = std::fs::remove_dir_all(&other);
            return;
        }
        let top = mount_point(&other, device_of(&other).unwrap()).unwrap();
        let uid = rustix::process::getuid().as_raw();
        let own = top.join(format!(".Trash-{uid}"));
        let made = !own.exists();
        let f = other.join("far.txt");
        std::fs::write(&f, b"far").unwrap();
        let t = trash_with(&f, &home_trash).unwrap();
        assert!(!home_trash.exists());
        assert!(t.file.starts_with(&top), "{} {}", t.file.display(), top.display());
        let i = info(&t);
        let rel = f.strip_prefix(&top).unwrap();
        assert!(i.contains(&format!("\nPath={}\n", encode_path(rel))), "{i}");
        // Clean up only what this test made.
        std::fs::remove_file(&t.file).unwrap();
        std::fs::remove_file(&t.info).unwrap();
        if made && t.file.starts_with(&own) {
            std::fs::remove_dir_all(&own).unwrap();
        }
        std::fs::remove_dir_all(&other).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}
