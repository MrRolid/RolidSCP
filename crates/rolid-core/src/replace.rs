//! Putting a finished file in place of an existing one, and repairing a
//! replacement that was cut off (crash, lost connection).
//!
//! [`safe_replace`] renames `part` over `target`:
//!
//! - atomically where the backend can ([`Vfs::rename_replace`]: local rename(2),
//!   SFTP `posix-rename@openssh.com`) – there is no moment without the file;
//! - otherwise (plain SFTPv3 `rename` does not overwrite) the target is first
//!   renamed aside (`name.rolidscp-old`), the new file renamed into place and the
//!   old one deleted; if the second rename fails, the original is renamed back.
//!
//! Either way the real name never holds a truncated file. In the second case a
//! crash or a lost connection between the two renames leaves the name missing
//! for a moment, so the step is written to a journal on this computer first
//! (one small file per replacement, removed when it is done). [`repair`] runs
//! when a connection to the server is made again: it finishes or undoes every
//! journaled step for that server.
//!
//! The names of leftovers changed in 0.8.14 (`.rolid-zaloha` → `.rolidscp-old`,
//! `.rolid-prejmenovani` → `.rolid-renaming`); the repair knows both.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::vfs::{Vfs, VfsResult};

/// The original, moved aside while the new file is renamed into place.
pub const ASIDE_SUFFIX: &str = ".rolidscp-old";
/// Older name of the same leftover (rename with overwrite, 0.8.13 and before).
pub const LEGACY_ASIDE_SUFFIXES: &[&str] = &[".rolid-zaloha"];
/// Temporary name of an item whose name changes only in letter case
/// (`.name.rolid-renaming`).
pub const RENAME_TMP_SUFFIX: &str = ".rolid-renaming";
/// Older name of the same temporary name (0.8.13 and before).
pub const LEGACY_RENAME_TMP_SUFFIXES: &[&str] = &[".rolid-prejmenovani"];

const JOURNAL_EXT: &str = "replace";
const JOURNAL_HEADER: &str = "rolidscp-replace 1";

static JOURNAL_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the journal of unfinished replacements is kept (set once at start-up;
/// without it nothing is journaled).
pub fn set_journal_dir(dir: PathBuf) {
    let _ = JOURNAL_DIR.set(dir);
}

fn journal_dir() -> Option<&'static Path> {
    JOURNAL_DIR.get().map(PathBuf::as_path)
}

/// One step that leaves a name missing for a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `target` was (or is being) moved to `aside` and `part` renamed to `target`.
    Replace { target: String, aside: String, part: String },
    /// `src` was (or is being) renamed to `tmp` and then to `dst`.
    Rename { src: String, tmp: String, dst: String },
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

fn encode(key: &str, step: &Step) -> String {
    let (kind, a, b, c) = match step {
        Step::Replace { target, aside, part } => ("replace", target, aside, part),
        Step::Rename { src, tmp, dst } => ("rename", src, tmp, dst),
    };
    format!("{JOURNAL_HEADER}\n{kind}\n{}\n{}\n{}\n{}\n", hex(key), hex(a), hex(b), hex(c))
}

fn decode(text: &str) -> Option<(String, Step)> {
    let mut l = text.lines();
    if l.next()? != JOURNAL_HEADER {
        return None;
    }
    let kind = l.next()?;
    let key = unhex(l.next()?)?;
    let (a, b, c) = (unhex(l.next()?)?, unhex(l.next()?)?, unhex(l.next()?)?);
    let step = match kind {
        "replace" => Step::Replace { target: a, aside: b, part: c },
        "rename" => Step::Rename { src: a, tmp: b, dst: c },
        _ => return None,
    };
    Some((key, step))
}

/// A journal entry on disk; removed by [`JournalEntry::done`].
struct JournalEntry(PathBuf);

impl JournalEntry {
    fn done(self) {
        // A failure only leaves the entry for the next repair, which finds
        // nothing to do and removes it.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Write the step to the journal (only for a server and with a journal folder).
fn begin(dir: Option<&Path>, fs: &dyn Vfs, step: &Step) -> Option<JournalEntry> {
    let dir = dir?;
    let key = fs.journal_key()?;
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    #[cfg(unix)]
    {
        // Server names and paths: for the user only.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = format!("{}-{nanos}-{}.{JOURNAL_EXT}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    // Written completely before it counts (renamed into place).
    let ok = std::fs::write(&tmp, encode(&key, step)).and_then(|_| std::fs::rename(&tmp, &path)).is_ok();
    ok.then_some(JournalEntry(path))
}

/// Replace `target` with `part` (see the module docs), journaled in the
/// folder set by [`set_journal_dir`].
pub async fn safe_replace(fs: &dyn Vfs, part: &str, target: &str) -> VfsResult<()> {
    safe_replace_in(fs, part, target, journal_dir()).await
}

/// [`safe_replace`] with the journal in `dir` (`None` = no journal).
pub async fn safe_replace_in(fs: &dyn Vfs, part: &str, target: &str, dir: Option<&Path>) -> VfsResult<()> {
    let aside = format!("{target}{ASIDE_SUFFIX}");
    // Atomic replace if possible. If the server offers it but refuses, the plain
    // path below gives the real reason (permissions…) or works where it does not.
    if let Ok(true) = fs.rename_replace(part, target).await {
        remove_stale_asides(fs, target).await;
        return Ok(());
    }
    if fs.stat(target).await.is_err() {
        // A leftover from a cut-off replacement may still hold the original.
        restore_aside(fs, target).await;
        if fs.stat(target).await.is_err() {
            return fs.rename(part, target).await;
        }
    }
    remove_stale_asides(fs, target).await;
    let entry = begin(dir, fs, &Step::Replace { target: target.into(), aside: aside.clone(), part: part.into() });
    if let Err(e) = fs.rename(target, &aside).await {
        // Nothing changed.
        if let Some(j) = entry {
            j.done();
        }
        return Err(e);
    }
    match fs.rename(part, target).await {
        Ok(()) => {
            let _ = fs.remove_file(&aside).await;
            if let Some(j) = entry {
                j.done();
            }
            Ok(())
        }
        Err(e) => {
            // Put the original back. If even that fails (the connection is gone),
            // the journal entry stays and the repair puts it back later.
            if fs.rename(&aside, target).await.is_ok() {
                if let Some(j) = entry {
                    j.done();
                }
            }
            Err(e)
        }
    }
}

/// Every name a leftover original of `target` can have (current and older).
fn aside_names(target: &str) -> Vec<String> {
    std::iter::once(ASIDE_SUFFIX).chain(LEGACY_ASIDE_SUFFIXES.iter().copied()).map(|s| format!("{target}{s}")).collect()
}

/// `target` exists: originals left aside by an earlier, cut-off replacement are
/// out of date and go.
async fn remove_stale_asides(fs: &dyn Vfs, target: &str) {
    for a in aside_names(target) {
        if fs.stat(&a).await.is_ok() {
            let _ = fs.remove_file(&a).await;
        }
    }
}

/// `target` is missing: put back an original left aside by a cut-off
/// replacement (current or older name). True when one was put back.
async fn restore_aside(fs: &dyn Vfs, target: &str) -> bool {
    for a in aside_names(target) {
        if fs.stat(&a).await.is_ok() && fs.rename(&a, target).await.is_ok() {
            return true;
        }
    }
    false
}

/// What [`repair`] did (paths on the server).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repair {
    /// The original was put back (the new version had not arrived).
    pub restored: Vec<String>,
    /// The new version was put in place (it was complete, only the rename was missing).
    pub completed: Vec<String>,
    /// Steps that could not be checked now (the connection failed); kept for next time.
    pub pending: usize,
}

impl Repair {
    pub fn is_empty(&self) -> bool {
        self.restored.is_empty() && self.completed.is_empty()
    }
}

/// Finish or undo the journaled steps of the server of `fs` (journal folder
/// from [`set_journal_dir`]).
pub async fn repair(fs: &dyn Vfs) -> Repair {
    match journal_dir() {
        Some(d) => repair_in(fs, d).await,
        None => Repair::default(),
    }
}

/// [`repair`] with the journal in `dir`.
pub async fn repair_in(fs: &dyn Vfs, dir: &Path) -> Repair {
    let mut out = Repair::default();
    let Some(key) = fs.journal_key() else { return out };
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == JOURNAL_EXT)).collect();
    files.sort();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Some((k, step)) = decode(&text) else {
            let _ = std::fs::remove_file(&f); // unreadable: of no use
            continue;
        };
        if k != key {
            continue;
        }
        match repair_step(fs, &step, &mut out).await {
            Ok(()) => {
                let _ = std::fs::remove_file(&f);
            }
            Err(e) if e.is_connection_lost() => out.pending += 1,
            Err(_) => {
                // Permissions or similar: nothing more can be done automatically.
                let _ = std::fs::remove_file(&f);
            }
        }
    }
    out
}

async fn exists(fs: &dyn Vfs, p: &str) -> VfsResult<bool> {
    match fs.stat(p).await {
        Ok(_) => Ok(true),
        Err(e) if e.is_connection_lost() => Err(e),
        Err(_) => Ok(false),
    }
}

async fn repair_step(fs: &dyn Vfs, step: &Step, out: &mut Repair) -> VfsResult<()> {
    match step {
        Step::Replace { target, aside, part } => {
            if exists(fs, target).await? {
                // The replacement went through; only the old copy may be left.
                if exists(fs, aside).await? {
                    fs.remove_file(aside).await?;
                }
            } else if exists(fs, aside).await? {
                // Cut off between the two renames: the original goes back. The
                // new version stays in its part file (an F4 edit uploads again).
                fs.rename(aside, target).await?;
                out.restored.push(target.clone());
            } else if exists(fs, part).await? {
                fs.rename(part, target).await?;
                out.completed.push(target.clone());
            }
        }
        Step::Rename { src, tmp, dst } => {
            if exists(fs, tmp).await? && !exists(fs, dst).await? {
                fs.rename(tmp, dst).await?;
                out.completed.push(dst.clone());
            }
            let _ = src;
        }
    }
    Ok(())
}

/// Journal a rename through a temporary name; [`RenameGuard::done`] when it is finished.
pub(crate) struct RenameGuard(Option<JournalEntry>);

impl RenameGuard {
    pub(crate) fn begin(fs: &dyn Vfs, src: &str, tmp: &str, dst: &str) -> RenameGuard {
        RenameGuard(begin(journal_dir(), fs, &Step::Rename { src: src.into(), tmp: tmp.into(), dst: dst.into() }))
    }
    pub(crate) fn done(self) {
        if let Some(j) = self.0 {
            j.done();
        }
    }
}

/// The original name of a leftover temporary name `name` (`.x.rolid-renaming`,
/// `.x.rolid-renaming-2`, older `.x.rolid-prejmenovani`), if it is one.
pub fn renaming_original(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('.')?;
    for suffix in std::iter::once(RENAME_TMP_SUFFIX).chain(LEGACY_RENAME_TMP_SUFFIXES.iter().copied()) {
        if let Some(i) = rest.rfind(suffix) {
            let tail = &rest[i + suffix.len()..];
            if tail.is_empty() || tail.strip_prefix('-').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
                return Some(&rest[..i]).filter(|o| !o.is_empty());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_text_round_trip() {
        let s = Step::Replace { target: "/a b/ž\nx".into(), aside: "/a.rolidscp-old".into(), part: "/a.filepart".into() };
        assert_eq!(decode(&encode("u@h:22", &s)), Some(("u@h:22".to_string(), s)));
        let r = Step::Rename { src: "/d/readme".into(), tmp: "/d/.readme.rolid-renaming".into(), dst: "/d/README".into() };
        assert_eq!(decode(&encode("k", &r)), Some(("k".to_string(), r)));
        assert_eq!(decode("garbage"), None);
    }

    #[test]
    fn leftover_names_old_and_new() {
        assert_eq!(renaming_original(".readme.rolid-renaming"), Some("readme"));
        assert_eq!(renaming_original(".readme.rolid-renaming-3"), Some("readme"));
        assert_eq!(renaming_original(".readme.rolid-prejmenovani"), Some("readme"));
        assert_eq!(renaming_original(".readme.rolid-prejmenovani-1"), Some("readme"));
        assert_eq!(renaming_original("readme.rolid-renaming"), None);
        assert_eq!(renaming_original(".readme.rolid-renaming-x"), None);
        assert_eq!(aside_names("/d/f"), ["/d/f.rolidscp-old", "/d/f.rolid-zaloha"]);
    }
}
