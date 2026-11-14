//! Private temporary directories for edited remote files.
//!
//! `<base>/<random>/<file name>`: the base and every per-file directory are 0700,
//! the file keeps its name (and extension, for syntax highlighting in the editor).

use std::path::{Path, PathBuf};

use rolid_core::VfsError;
use rolid_text::{txt, Text};

/// Base directory: `$ROLIDSCP_EDIT_DIR`, else `~/.local/state/rolidscp/edit`
/// (on disk, not in memory: a large file does not take RAM, and an unsaved
/// edit survives a crash of the whole system).
pub fn base_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ROLIDSCP_EDIT_DIR") {
        return PathBuf::from(d);
    }
    dirs::state_dir().or_else(dirs::cache_dir).unwrap_or_else(std::env::temp_dir).join("rolidscp").join("edit")
}

/// Where versions up to 0.8.13 kept the copies (`$XDG_RUNTIME_DIR/rolidscp-edit`,
/// a tmpfs): still scanned for leftovers of a crash. `None` with
/// `$ROLIDSCP_EDIT_DIR` set, or when it does not exist.
pub fn legacy_base_dir() -> Option<PathBuf> {
    if std::env::var_os("ROLIDSCP_EDIT_DIR").is_some() {
        return None;
    }
    let d = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("rolidscp-edit");
    (d != base_dir() && d.is_dir()).then_some(d)
}

/// The edit copy lives under one of our temporary folders (new or old).
pub fn is_temp(path: &Path) -> bool {
    path.starts_with(base_dir()) || legacy_base_dir().is_some_and(|d| path.starts_with(d))
}

/// The local file name for a remote one: the same name, or an error if it could
/// escape the directory (same check as for downloads).
pub fn local_name(remote_name: &str) -> Result<String, Text> {
    rolid_core::vfs::validate_name(remote_name).map_err(|e| e.text())?;
    if remote_name.chars().any(char::is_control) {
        return Err(txt!("Invalid file name: {name}", name = format!("{remote_name:?}")));
    }
    Ok(remote_name.to_owned())
}

#[derive(Debug, Clone)]
pub struct EditDir {
    /// Private directory of this edit.
    pub dir: PathBuf,
    /// The file inside it.
    pub file: PathBuf,
}

impl EditDir {
    /// Delete the directory with everything in it (and its `.meta`).
    pub fn remove(&self) -> std::io::Result<()> {
        crate::recover::remove_edit(&self.dir)
    }
}

#[cfg(unix)]
pub(crate) fn mkdir_private(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(p)
}

#[cfg(not(unix))]
pub(crate) fn mkdir_private(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir(p)
}

/// Make sure `base` exists, is a real directory (not a symlink) and is private.
/// An error of our own carries a [`VfsError`] (its `text()` for the user).
pub fn ensure_base(base: &Path) -> std::io::Result<()> {
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match mkdir_private(base) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let md = std::fs::symlink_metadata(base)?;
    if !md.is_dir() {
        return Err(std::io::Error::other(VfsError::Backend(txt!("{path} is not a folder", path = base))));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if md.uid() != own_uid() {
            return Err(std::io::Error::other(VfsError::Backend(txt!(
                "{path} belongs to another user",
                path = base
            ))));
        }
        if md.mode() & 0o077 != 0 {
            std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn own_uid() -> u32 {
    // Our uid without libc: the owner of /proc/self (Linux).
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or_else(|_| {
        // Not Linux: fall back to the owner of a fresh temp file.
        let p = std::env::temp_dir().join(format!("rolidscp-uid-{}", std::process::id()));
        let uid = std::fs::write(&p, b"").and_then(|_| std::fs::metadata(&p)).map(|m| m.uid()).unwrap_or(0);
        let _ = std::fs::remove_file(&p);
        uid
    })
}

/// Create a new private directory under `base` for `remote_name`.
pub fn create(base: &Path, remote_name: &str) -> Result<EditDir, Text> {
    let name = local_name(remote_name)?;
    ensure_base(base).map_err(|e| txt!("{path}: {error}", path = base, error = VfsError::Io(e).text()))?;
    for _ in 0..16 {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).map_err(rolid_core::vfs::raw_text)?;
        let hex: String = r.iter().map(|b| format!("{b:02x}")).collect();
        let dir = base.join(hex);
        match mkdir_private(&dir) {
            Ok(()) => {
                let file = dir.join(&name);
                return Ok(EditDir { dir, file });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(txt!("{path}: {error}", path = &dir, error = e.to_string())),
        }
    }
    Err(txt!("cannot create a temporary folder"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_could_escape_are_rejected() {
        for bad in ["", ".", "..", "../x", "a/b", "..\\x", "x\0", "new\nline"] {
            assert!(local_name(bad).is_err(), "{bad:?}");
        }
        // A backslash is an ordinary character in names on Linux and macOS.
        assert_eq!(local_name("a\\b").is_ok(), !cfg!(windows));
        assert_eq!(local_name("index.php").unwrap(), "index.php");
        assert_eq!(local_name("..hidden.conf").unwrap(), "..hidden.conf");
        assert_eq!(local_name("Příliš žluťoučký.txt").unwrap(), "Příliš žluťoučký.txt");
    }

    #[test]
    fn private_directories() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("edit");
        let a = create(&base, "nginx.conf").unwrap();
        let b = create(&base, "nginx.conf").unwrap();
        assert_ne!(a.dir, b.dir, "one directory per edited file");
        assert_eq!(a.file.file_name().unwrap(), "nginx.conf");
        assert_eq!(a.file.parent().unwrap(), a.dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for d in [&base, &a.dir] {
                assert_eq!(std::fs::metadata(d).unwrap().permissions().mode() & 0o777, 0o700);
            }
        }
        std::fs::write(&a.file, b"x").unwrap();
        a.remove().unwrap();
        assert!(!a.dir.exists());
        assert!(create(&base, "../etc/passwd").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_base_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(create(&link, "a.txt").is_err());
    }
}
