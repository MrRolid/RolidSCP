//! The part file being written by packing or extracting, recorded so that a
//! `.filepart` left by a killed program is cleaned up at its next start (a
//! cancel or an error removes it at once).
//!
//! Each job has a small file `<pid>-<n>.parts` in the state folder holding
//! the path of its current part file (ended by a NUL byte; the path itself
//! cannot contain one). The record is rewritten in place with one write, so
//! a kill leaves it either old or new, never torn; it needs no flush, the
//! page cache survives the process. A finished job removes its file.
//! [`clean`] removes the part files of jobs whose process is gone.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::Tick;

const EXT: &str = "parts";
const PART: &str = ".filepart";

/// The record of one packing or extracting job.
pub struct PartJournal {
    file: Mutex<File>,
    path: PathBuf,
}

impl PartJournal {
    /// A new record in `dir` (made if missing).
    pub fn create(dir: &Path) -> io::Result<PartJournal> {
        static N: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(dir)?;
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{}-{n}.{EXT}", std::process::id()));
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(&path)?;
        Ok(PartJournal { file: Mutex::new(file), path })
    }

    /// Follow the progress of a job: [`Tick::Part`] updates the record.
    pub fn note(&self, t: &Tick<'_>) {
        if let Tick::Part(p) = t {
            self.set(*p);
        }
    }

    /// The part file being written now (`None`: none).
    pub fn set(&self, part: Option<&Path>) {
        let mut rec = part.map(|p| p.as_os_str().as_bytes().to_vec()).unwrap_or_default();
        rec.push(0);
        let f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if f.write_all_at(&rec, 0).is_ok() {
            let _ = f.set_len(rec.len() as u64);
        }
    }
}

impl Drop for PartJournal {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Is process `pid` running? (Another copy of the program keeps its jobs.)
fn alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// Remove the part files left by jobs of processes that are gone, and their
/// records; returns the removed part files. Only a regular file whose name
/// ends with `.filepart` is ever removed.
pub fn clean(dir: &Path) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return removed };
    for e in rd.flatten() {
        let rec = e.path();
        if rec.extension().and_then(|x| x.to_str()) != Some(EXT) {
            continue;
        }
        let pid = rec.file_stem().and_then(|s| s.to_str()).and_then(|s| s.split('-').next()).and_then(|p| p.parse::<u32>().ok());
        let Some(pid) = pid else { continue };
        if alive(pid) {
            continue;
        }
        if let Ok(bytes) = fs::read(&rec) {
            let path = bytes.split(|b| *b == 0).next().unwrap_or_default();
            if !path.is_empty() {
                let part = PathBuf::from(std::ffi::OsStr::from_bytes(path));
                let named = part.file_name().is_some_and(|n| n.as_bytes().ends_with(PART.as_bytes()));
                let file = fs::symlink_metadata(&part).is_ok_and(|m| m.file_type().is_file());
                if named && file && fs::remove_file(&part).is_ok() {
                    removed.push(part);
                }
            }
        }
        let _ = fs::remove_file(&rec);
    }
    removed
}
