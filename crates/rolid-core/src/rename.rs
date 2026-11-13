//! Renaming an item in its folder (F2) and moving by rename (F6 on the same
//! disk or server).
//!
//! A conflict is told by the exact name in the folder listing, so a change of
//! letter case only ("readme" → "README") is not mistaken for an existing
//! target on a case-insensitive file system; such a rename goes through a
//! temporary name, because some of those file systems ignore a direct one.

use crate::vfs::{Vfs, VfsError, VfsResult};
use rolid_text::{txt, txtn, Text};

/// Longest name most file systems take (ext4, btrfs, XFS, NTFS in UTF-16 units).
pub const MAX_NAME_BYTES: usize = 255;

/// Check a new name typed by the user; the error is a message for them.
pub fn check_new_name(name: &str) -> Result<(), Text> {
    if name.is_empty() {
        return Err(txt!("The name must not be empty."));
    }
    if name.trim().is_empty() {
        return Err(txt!("The name must not consist of spaces only."));
    }
    if name == "." || name == ".." {
        return Err(txt!("The name cannot be “{name}” (a dot and two dots mean folders).", name = name));
    }
    if name.contains('/') {
        return Err(txt!("The name must not contain a slash “/” (the folder separator)."));
    }
    if name.contains('\0') {
        return Err(txt!("The name must not contain the NUL character."));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(txtn!(
            "The name is too long: {n} byte, at most {max} are allowed (accented characters take 2 bytes).",
            "The name is too long: {n} bytes, at most {max} are allowed (accented characters take 2 bytes).",
            name.len(),
            max = MAX_NAME_BYTES
        ));
    }
    Ok(())
}

/// Range of the name without its extension (selected when editing starts):
/// "report.txt" → 0..6, ".bashrc" → 0..7 (a leading dot is no extension),
/// "archive.tar.gz" → 0..11. In characters, not bytes.
pub fn stem_chars(name: &str) -> usize {
    let n = name.chars().count();
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].chars().count(),
        _ => n,
    }
}

/// How a rename ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameOutcome {
    Done,
    /// The new name is the old one: nothing to do.
    Unchanged,
    /// Something with the new name exists (and `overwrite` was not given);
    /// `dir` = it is a folder.
    Exists { dir: bool },
}

/// The two names differ only in letter case.
pub fn case_only(a: &str, b: &str) -> bool {
    a != b && a.to_lowercase() == b.to_lowercase()
}

/// Rename `from` to `to` in folder `dir`. An existing `to` is replaced only
/// with `overwrite`, and only a file by a file (never a folder).
pub async fn rename_entry(fs: &dyn Vfs, dir: &str, from: &str, to: &str, overwrite: bool) -> VfsResult<RenameOutcome> {
    if from == to {
        return Ok(RenameOutcome::Unchanged);
    }
    check_new_name(to).map_err(VfsError::Backend)?;
    let src = fs.join(dir, from);
    let dst = fs.join(dir, to);
    let listing = fs.list(dir).await?;
    let Some(entry) = listing.iter().find(|e| e.name == from) else {
        return Err(VfsError::Backend(txt!("“{name}” is no longer in the folder (did someone else delete or rename it?)", name = from)));
    };
    let src_dir = entry.is_dir_like();
    let exact = listing.iter().find(|e| e.name == to).map(|e| e.is_dir_like());
    let only_case = case_only(from, to);
    // Another item under a case-insensitive alias of the name also blocks it.
    let existing = match exact {
        Some(d) => Some(d),
        None if !only_case => fs.stat(&dst).await.ok().map(|e| e.is_dir_like()),
        None => None,
    };
    if let Some(dst_dir) = existing {
        if !overwrite {
            return Ok(RenameOutcome::Exists { dir: dst_dir });
        }
        if dst_dir || src_dir {
            return Err(VfsError::Backend(txt!("“{name}” is a folder or a folder is being renamed: only a file can overwrite a file.", name = to)));
        }
        replace(fs, &src, &dst).await?;
        return Ok(RenameOutcome::Done);
    }
    if only_case {
        let tmp = temp_name(&listing, from);
        let tmp = fs.join(dir, &tmp);
        fs.rename(&src, &tmp).await?;
        if let Err(e) = fs.rename(&tmp, &dst).await {
            let _ = fs.rename(&tmp, &src).await;
            return Err(e);
        }
        return Ok(RenameOutcome::Done);
    }
    fs.rename(&src, &dst).await?;
    Ok(RenameOutcome::Done)
}

/// A free temporary name next to `name`.
fn temp_name(listing: &[crate::Entry], name: &str) -> String {
    let base = format!(".{name}.rolid-prejmenovani");
    let mut n = 0u32;
    loop {
        let cand = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        if !listing.iter().any(|e| e.name.to_lowercase() == cand.to_lowercase()) {
            return cand;
        }
        n += 1;
    }
}

/// Put file `src` in place of file `dst`: atomically where the backend can
/// (local rename(2), SFTP posix-rename@openssh.com), else by moving `dst`
/// aside first and back if the rename fails.
pub async fn replace(fs: &dyn Vfs, src: &str, dst: &str) -> VfsResult<()> {
    if fs.rename_replace(src, dst).await? {
        return Ok(());
    }
    let aside = format!("{dst}.rolid-zaloha");
    fs.rename(dst, &aside).await?;
    match fs.rename(src, dst).await {
        Ok(()) => {
            let _ = fs.remove_file(&aside).await;
            Ok(())
        }
        Err(e) => {
            let _ = fs.rename(&aside, dst).await;
            Err(e)
        }
    }
}

/// What a move by rename did with one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStep {
    /// Moved (or already in place).
    Moved,
    /// Not moved: the target exists (the copy applies the conflict choice) or
    /// the folders are on different disks. The item goes through copy + delete.
    NeedsCopy,
}

/// Move `src_dir/name` to `dst_dir/dst_name` on the same backend by rename.
pub async fn move_by_rename(fs: &dyn Vfs, src_dir: &str, name: &str, dst_dir: &str, dst_name: &str) -> VfsResult<MoveStep> {
    let src = fs.join(src_dir, name);
    let dst = fs.join(dst_dir, dst_name);
    if src == dst {
        return Ok(MoveStep::Moved);
    }
    if inside(&src, &dst) {
        return Err(VfsError::Backend(txt!("Folder “{name}” cannot be moved into itself.", name = name)));
    }
    if fs.stat(&dst).await.is_ok() {
        return Ok(MoveStep::NeedsCopy);
    }
    match fs.rename(&src, &dst).await {
        Ok(()) => Ok(MoveStep::Moved),
        Err(VfsError::Io(e)) if crosses_devices(&e) => Ok(MoveStep::NeedsCopy),
        Err(e) => Err(e),
    }
}

/// `path` is `dir` itself or lies inside it (POSIX paths).
pub fn inside(dir: &str, path: &str) -> bool {
    let d = dir.trim_end_matches('/');
    path == d || path.strip_prefix(d).is_some_and(|rest| rest.starts_with('/'))
}

/// `path` moved from `old` to `new`: its new path (when it is `old` or inside it).
pub fn moved_path(path: &str, old: &str, new: &str) -> Option<String> {
    if !inside(old, path) {
        return None;
    }
    let rest = &path[old.trim_end_matches('/').len()..];
    Some(format!("{}{rest}", new.trim_end_matches('/')))
}

/// The target typed in the move dialog (POSIX paths): the folder to move into
/// and, for a single item, its new name when it changes. A relative path is
/// taken from `dst_dir`; for a single item the path ends with its (new) name,
/// a trailing "/" means "into this folder, same name".
pub fn split_target(dst_dir: &str, raw: &str, single: Option<&str>) -> Result<(String, Option<String>), Text> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(txt!("Enter where to move to."));
    }
    let abs = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("{}/{raw}", dst_dir.trim_end_matches('/'))
    };
    let dir_only = |p: &str| {
        let t = p.trim_end_matches('/');
        if t.is_empty() { "/".to_string() } else { t.to_string() }
    };
    match single {
        Some(old) if !raw.ends_with('/') => {
            let (dir, name) = abs.rsplit_once('/').unwrap_or(("", abs.as_str()));
            check_new_name(name)?;
            let dir = dir_only(dir);
            Ok((dir, (name != old).then(|| name.to_string())))
        }
        _ => Ok((dir_only(&abs), None)),
    }
}

/// rename(2) failed because the two paths are on different file systems.
fn crosses_devices(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_checks() {
        for (n, part) in [
            ("", "empty"),
            ("   ", "spaces"),
            (".", "“.”"),
            ("..", "“..”"),
            ("a/b", "slash"),
            ("a\0b", "NUL"),
        ] {
            let e = check_new_name(n).unwrap_err().to_string();
            assert!(e.contains(part), "{n:?}: {e}");
        }
        let long = "ž".repeat(128); // 256 bytes
        assert!(check_new_name(&long).unwrap_err().to_string().contains("256 bytes"));
        assert!(check_new_name(&"a".repeat(255)).is_ok());
        for ok in ["zpráva.txt", ".bashrc", "..hidden", "a b", "UPPER"] {
            assert!(check_new_name(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn selection_without_the_extension() {
        assert_eq!(stem_chars("zpráva.txt"), 6);
        assert_eq!(stem_chars(".bashrc"), 7);
        assert_eq!(stem_chars("archive.tar.gz"), 11);
        assert_eq!(stem_chars("no_extension"), 12);
        assert_eq!(stem_chars("čeština.md"), 7);
    }

    #[test]
    fn paths_inside_and_moved() {
        assert!(inside("/a/b", "/a/b"));
        assert!(inside("/a/b", "/a/b/c"));
        assert!(inside("/a/b/", "/a/b/c"));
        assert!(!inside("/a/b", "/a/bc"));
        assert_eq!(moved_path("/a/b/c/d", "/a/b", "/x/y").as_deref(), Some("/x/y/c/d"));
        assert_eq!(moved_path("/a/b", "/a/b", "/a/B").as_deref(), Some("/a/B"));
        assert_eq!(moved_path("/a/bc", "/a/b", "/x"), None);
        assert!(case_only("readme", "README"));
        assert!(!case_only("readme", "readme"));
        assert!(!case_only("readme", "readme2"));
    }

    #[test]
    fn move_targets() {
        let t = |raw, single| split_target("/srv/dest", raw, single);
        assert_eq!(t("/srv/dest/a.txt", Some("a.txt")), Ok(("/srv/dest".into(), None)));
        assert_eq!(t("/srv/dest/b.txt", Some("a.txt")), Ok(("/srv/dest".into(), Some("b.txt".into()))));
        assert_eq!(t("elsewhere/b.txt", Some("a.txt")), Ok(("/srv/dest/elsewhere".into(), Some("b.txt".into()))));
        assert_eq!(t("/tmp/", Some("a.txt")), Ok(("/tmp".into(), None)));
        assert_eq!(t("/a.txt", Some("a.txt")), Ok(("/".into(), None)));
        assert_eq!(t("/srv/dest", None), Ok(("/srv/dest".into(), None)));
        assert_eq!(t("/srv/dest/", None), Ok(("/srv/dest".into(), None)));
        assert_eq!(t("/", None), Ok(("/".into(), None)));
        assert!(t("  ", None).is_err());
        assert!(t("/srv/..", Some("a")).is_err());
    }
}
