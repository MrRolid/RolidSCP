//! Changing permissions (chmod) locally and on servers: the dialog model
//! (checkboxes with a third state "unchanged" + the octal field, kept in
//! sync), the recursive plan (separate modes for files and folders, symlinks
//! never followed) and running the plan.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::copy::Failure;
use crate::vfs::{Entry, EntryKind, Vfs, VfsError, VfsResult};
use rolid_text::{txt, Text};

/// One permission bit in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    Off,
    On,
    /// Several items differ: leave the bit as it is on each.
    Keep,
}

/// The 12 permission bits (bit `i` = `1 << i`: other x/w/r, group x/w/r,
/// owner x/w/r, sticky, setgid, setuid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermEdit {
    pub bits: [Tri; 12],
}

pub const SETUID: usize = 11;
pub const SETGID: usize = 10;
pub const STICKY: usize = 9;

impl PermEdit {
    /// Current permissions of the selected items: a bit set on all = On, on
    /// none = Off, on some = Keep.
    pub fn from_modes(modes: &[u32]) -> PermEdit {
        let mut bits = [Tri::Off; 12];
        for (i, b) in bits.iter_mut().enumerate() {
            let n = modes.iter().filter(|m| *m & (1 << i) != 0).count();
            *b = if n == 0 {
                Tri::Off
            } else if n == modes.len() {
                Tri::On
            } else {
                Tri::Keep
            };
        }
        PermEdit { bits }
    }

    /// A definite mode (no Keep bits).
    pub fn from_mode(mode: u32) -> PermEdit {
        PermEdit::from_modes(&[mode])
    }

    /// Bit index for a checkbox: `who` 0 = owner, 1 = group, 2 = others;
    /// `what` 0 = r, 1 = w, 2 = x.
    pub fn index(who: usize, what: usize) -> usize {
        (2 - who) * 3 + (2 - what)
    }

    /// A click on a checkbox: Keep → On → Off → On…
    pub fn toggle(&mut self, i: usize) {
        self.bits[i] = match self.bits[i] {
            Tri::On => Tri::Off,
            Tri::Off | Tri::Keep => Tri::On,
        };
    }

    /// Four octal digits (special bits first); a digit whose bits are not all
    /// definite is `?`.
    pub fn octal(&self) -> String {
        (0..4)
            .rev()
            .map(|d| {
                let group = &self.bits[d * 3..d * 3 + 3];
                if group.contains(&Tri::Keep) {
                    '?'
                } else {
                    let v: u32 = group.iter().enumerate().map(|(k, t)| u32::from(*t == Tri::On) << k).sum();
                    char::from_digit(v, 8).unwrap()
                }
            })
            .collect()
    }

    /// Set from the octal field: 3 digits (owner, group, others – special bits
    /// stay as they are) or 4 digits (special bits first). `?` keeps a digit's
    /// bits unchanged.
    pub fn set_octal(&mut self, text: &str) -> Result<(), Text> {
        let t = text.trim();
        let digits: Vec<char> = t.chars().collect();
        if !(3..=4).contains(&digits.len()) || !digits.iter().all(|c| matches!(c, '0'..='7' | '?')) {
            return Err(txt!("enter 3 or 4 octal digits (0–7), ? = unchanged"));
        }
        let mut new = *self;
        for (pos, c) in digits.iter().rev().enumerate() {
            for k in 0..3 {
                new.bits[pos * 3 + k] = match c.to_digit(8) {
                    Some(v) if v & (1 << k) != 0 => Tri::On,
                    Some(_) => Tri::Off,
                    None => Tri::Keep,
                };
            }
        }
        *self = new;
        Ok(())
    }

    /// The new mode of an item that has `old` now.
    pub fn apply(&self, old: u32) -> u32 {
        let mut m = old & 0o7777;
        for (i, b) in self.bits.iter().enumerate() {
            match b {
                Tri::On => m |= 1 << i,
                Tri::Off => m &= !(1 << i),
                Tri::Keep => {}
            }
        }
        m
    }
}

/// Inside folders: what gets the separate modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Files,
    Dirs,
    Both,
}

/// "Apply to folder contents too".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recursion {
    pub scope: Scope,
    /// Mode for files inside (default 644).
    pub file_mode: u32,
    /// Mode for folders inside (default 755).
    pub dir_mode: u32,
}

impl Default for Recursion {
    fn default() -> Self {
        Recursion { scope: Scope::Both, file_mode: 0o644, dir_mode: 0o755 }
    }
}

/// One planned change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub path: String,
    pub is_dir: bool,
    pub old: u32,
    pub new: u32,
}

/// The whole change, in the order it is applied: a folder's contents before
/// the folder itself (removing rights on a folder never locks out its contents).
#[derive(Debug, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub files: u64,
    pub dirs: u64,
    /// Could not be planned (unreadable folders, symlinks selected…).
    pub failures: Vec<Failure>,
}

impl Plan {
    /// Steps that really change something.
    pub fn changes(&self) -> usize {
        self.steps.iter().filter(|s| s.old != s.new).count()
    }
}

fn perm(e: &Entry) -> u32 {
    e.mode.unwrap_or(0) & 0o7777
}

/// Plan the change for `items` (`(path, entry)`); with `rec`, also for the
/// folders' contents. Symlinks are never followed nor changed (changing a
/// link's permissions would change its target).
pub async fn plan(
    fs: &dyn Vfs,
    items: &[(String, Entry)],
    edit: &PermEdit,
    rec: Option<Recursion>,
    cancel: &AtomicBool,
) -> VfsResult<Plan> {
    let mut plan = Plan::default();
    for (path, e) in items {
        if e.kind == EntryKind::Symlink {
            plan.failures.push(Failure {
                path: path.clone(),
                error: txt!("symbolic link – the permissions of a link are not changed (that would change its target)"),
            });
            continue;
        }
        let is_dir = e.kind == EntryKind::Dir;
        if let (true, Some(r)) = (is_dir, rec) {
            walk(fs, path, r, cancel, &mut plan).await?;
        }
        add(&mut plan, path.clone(), is_dir, perm(e), edit.apply(perm(e)));
    }
    Ok(plan)
}

fn add(plan: &mut Plan, path: String, is_dir: bool, old: u32, new: u32) {
    if is_dir {
        plan.dirs += 1;
    } else {
        plan.files += 1;
    }
    plan.steps.push(Step { path, is_dir, old, new });
}

fn walk<'a>(
    fs: &'a dyn Vfs,
    dir: &'a str,
    r: Recursion,
    cancel: &'a AtomicBool,
    plan: &'a mut Plan,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = VfsResult<()>> + Send + 'a>> {
    Box::pin(async move {
        if cancel.load(Ordering::Relaxed) {
            return Err(VfsError::canceled());
        }
        let entries = match fs.list(dir).await {
            Ok(v) => v,
            Err(e) if e.is_connection_lost() => return Err(e),
            Err(e) => {
                plan.failures.push(Failure { path: dir.to_string(), error: txt!("cannot read the contents: {error}", error = e.text()) });
                return Ok(());
            }
        };
        for e in entries {
            let p = fs.join(dir, &e.name);
            match e.kind {
                EntryKind::Dir => {
                    walk(fs, &p, r, cancel, plan).await?;
                    if r.scope != Scope::Files {
                        let old = perm(&e);
                        // Special bits (setgid on shared folders…) stay.
                        add(plan, p, true, old, (old & 0o7000) | (r.dir_mode & 0o777));
                    }
                }
                EntryKind::File if r.scope != Scope::Dirs => {
                    let old = perm(&e);
                    add(plan, p, false, old, (old & 0o7000) | (r.file_mode & 0o777));
                }
                // Symlinks (not followed, not changed) and special files.
                _ => {}
            }
        }
        Ok(())
    })
}

/// Run the plan. `progress(done)` after every step. A failed item is recorded
/// and the rest goes on; a lost connection or cancel stops.
pub async fn run(
    fs: &dyn Vfs,
    plan: &Plan,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> (VfsResult<()>, Vec<Failure>) {
    let mut failures = Vec::new();
    for (i, s) in plan.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return (Err(VfsError::canceled()), failures);
        }
        if s.old != s.new {
            match fs.set_attrs(&s.path, Some(s.new), None).await {
                Ok(()) => {}
                Err(e) if e.is_connection_lost() => return (Err(e), failures),
                Err(e) => failures.push(Failure { path: s.path.clone(), error: e.text() }),
            }
        }
        progress(i as u64 + 1);
    }
    (Ok(()), failures)
}

/// `rwxr-sr-t`-style text of a mode.
pub fn symbolic(mode: u32) -> String {
    let mut s = String::new();
    for who in 0..3 {
        let shift = (2 - who) * 3;
        let r = mode >> shift & 4 != 0;
        let w = mode >> shift & 2 != 0;
        let x = mode >> shift & 1 != 0;
        let special = match who {
            0 => mode & 0o4000 != 0,
            1 => mode & 0o2000 != 0,
            _ => mode & 0o1000 != 0,
        };
        s.push(if r { 'r' } else { '-' });
        s.push(if w { 'w' } else { '-' });
        let (on, off) = match (who, special) {
            (2, true) => ('t', 'T'),
            (_, true) => ('s', 'S'),
            _ => ('x', '-'),
        };
        s.push(if x { on } else { off });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkboxes_and_octal_field_stay_in_sync() {
        let mut e = PermEdit::from_mode(0o644);
        assert_eq!(e.octal(), "0644");
        e.toggle(PermEdit::index(0, 2)); // owner x
        e.toggle(PermEdit::index(1, 2)); // group x
        assert_eq!(e.octal(), "0754");
        e.set_octal("2770").unwrap();
        assert_eq!(e.bits[SETGID], Tri::On);
        assert_eq!(e.bits[PermEdit::index(2, 0)], Tri::Off);
        assert_eq!(e.apply(0o100644), 0o2770, "file type bits are dropped");
        assert!(e.set_octal("89").is_err());
        assert!(e.set_octal("07777").is_err());
        assert_eq!(e.octal(), "2770", "a wrong entry changes nothing");
        assert_eq!(symbolic(0o2755), "rwxr-sr-x");
        assert_eq!(symbolic(0o1777), "rwxrwxrwt");
        assert_eq!(symbolic(0o4644), "rwSr--r--");
    }

    #[test]
    fn three_digits_keep_the_special_bits() {
        let mut e = PermEdit::from_mode(0o2775); // setgid folder
        e.set_octal("750").unwrap();
        assert_eq!(e.apply(0o2775), 0o2750, "setgid stays (only the octal field with 4 digits changes it)");
        e.set_octal("0750").unwrap();
        assert_eq!(e.apply(0o2775), 0o0750);
    }

    #[test]
    fn different_items_get_a_third_state() {
        let e = PermEdit::from_modes(&[0o644, 0o664, 0o600]);
        assert_eq!(e.bits[PermEdit::index(0, 0)], Tri::On, "owner r on all");
        assert_eq!(e.bits[PermEdit::index(1, 1)], Tri::Keep, "group w on one");
        assert_eq!(e.bits[PermEdit::index(2, 2)], Tri::Off);
        assert_eq!(e.octal(), "06??");
        // Only definite bits change; "unchanged" bits stay per item.
        let mut e2 = e;
        e2.toggle(PermEdit::index(2, 0)); // others r: Keep -> On
        assert_eq!(e2.apply(0o600), 0o604);
        assert_eq!(e2.apply(0o664), 0o664);
        assert_eq!(e.apply(0o664), 0o664);
        assert_eq!(e.apply(0o600), 0o600);
        let mut e3 = e;
        e3.set_octal("7?0").unwrap();
        assert_eq!(e3.apply(0o664), 0o760);
        assert_eq!(e3.apply(0o600), 0o700);
        assert_eq!(e3.octal(), "07?0");
    }
}
