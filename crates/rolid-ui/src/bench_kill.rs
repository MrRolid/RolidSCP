//! The process killed (SIGKILL) in the middle of an operation (roadmap 0.8.16):
//! afterwards no data may be lost or damaged. `scripts/bench/kill.sh` runs the
//! stage `do` of an operation, kills it after a random time, runs the stage
//! `check` and repeats. Each check also finishes the interrupted work the way
//! the program does it (copy again with "Resume", the repair of replacements)
//! and compares everything byte by byte.
//!
//! Ignored by a plain `cargo test`. Environment:
//!
//! - `ROLIDSCP_BIG_SSH`: the server (see `bench_big`)
//! - `ROLIDSCP_KILL_DIR`: the work folder (seen by the server too)
//! - `ROLIDSCP_KILL_OP`: `copy_up`, `copy_down`, `move_up`, `move_down`, `f4`,
//!   `pack`, `unpack`, `settings`, `vault`, `trash`
//! - `ROLIDSCP_KILL_STAGE`: `prepare` (once), `do`, `check`
//!
//! `check` prints `KILLCHECK <op> ok <what was found>` or panics.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::Vfs;
use secrecy::{ExposeSecret, SecretString};

const FILES: usize = 24;

fn content(name: &str, len: usize) -> Vec<u8> {
    let mut x: u64 = name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)) | 1;
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(len);
    v
}

/// The source tree: `data/` with files `fNN` of (NN+1) × 300 KiB in two folders.
fn rel(i: usize) -> String {
    format!("{}/f{i:02}", if i.is_multiple_of(2) { "a" } else { "b" })
}

fn expected(i: usize) -> Vec<u8> {
    content(&rel(i), (i + 1) * 300 * 1024)
}

fn write_tree(root: &Path) {
    for i in 0..FILES {
        let p = root.join(rel(i));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, expected(i)).unwrap();
    }
}

/// Every regular file under `root` (relative names).
fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

fn is_part(name: &str) -> bool {
    [rolid_core::copy::PARTIAL_SUFFIX, rolid_core::copy::PARTIAL_INFO_SUFFIX, rolid_core::replace::ASIDE_SUFFIX].iter().any(|s| name.ends_with(s))
}

/// Files under their real names in `root` are complete; returns (complete, part files).
fn real_files_complete(root: &Path) -> (usize, usize) {
    let (mut ok, mut parts) = (0, 0);
    for f in files(root) {
        if is_part(&f) {
            parts += 1;
            continue;
        }
        let i = (0..FILES).find(|i| rel(*i) == f).unwrap_or_else(|| panic!("unexpected file {f}"));
        assert!(std::fs::read(root.join(&f)).unwrap() == expected(i), "{f} under its real name is damaged");
        ok += 1;
    }
    (ok, parts)
}

fn all_complete(root: &Path) {
    let got = files(root);
    let want: Vec<String> = {
        let mut v: Vec<String> = (0..FILES).map(rel).collect();
        v.sort();
        v
    };
    assert_eq!(got, want, "after finishing: exactly the source files, no leftovers");
    real_files_complete(root);
}

async fn connect() -> (Arc<dyn Vfs>, rolid_ssh::SshConnection) {
    let spec = std::env::var("ROLIDSCP_BIG_SSH").expect("ROLIDSCP_BIG_SSH");
    let p: Vec<&str> = spec.split('|').collect();
    let params = rolid_ssh::ConnectParams {
        host: "127.0.0.1".into(),
        port: p[0].parse().unwrap(),
        user: p[1].into(),
        key_path: Some(p[2].into()),
        known_hosts: Some(p[3].into()),
        skip_agent: true,
        skip_default_keys: true,
        ..Default::default()
    };
    let conn = rolid_ssh::SshConnection::connect(&params).await.expect("connect");
    let fs: Arc<dyn Vfs> = Arc::new(conn.sftp().await.expect("sftp"));
    (fs, conn)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

async fn copy(src: &dyn Vfs, dst: &dyn Vfs, from_dir: &Path, to_dir: &Path, conflict: Conflict, moving: bool) {
    static NOOP: fn(&Progress) = |_| {};
    let mut job = CopyJob::new(src, dst, Arc::new(AtomicBool::new(false)), &NOOP).with_conflict(conflict);
    if moving {
        job = job.moving().verify_moved(true);
    }
    job.copy_items_retrying(&[(s(from_dir), "data".into())], &s(to_dir), 0, |_| async { false }).await.expect("copy");
    if moving {
        let c = job.delete_moved_sources().await;
        assert!(c.mismatch.is_empty() && c.changed.is_empty(), "{c:?}");
    }
}

const SETTINGS_A: &str = "a";
const SETTINGS_B: &str = "b";

fn settings_text(which: &str) -> String {
    // A few hundred lines, like a workspace with many tabs.
    (0..400).map(|i| format!("[[tab]]\nversion = \"{which}\"\nn = {i}\npath = \"/home/user/{which}/{i}\"\n")).collect()
}

fn master() -> SecretString {
    SecretString::from("kill test master password".to_string())
}

#[test]
#[ignore]
fn bench_kill() {
    let (Ok(dir), Ok(op), Ok(stage)) = (std::env::var("ROLIDSCP_KILL_DIR"), std::env::var("ROLIDSCP_KILL_OP"), std::env::var("ROLIDSCP_KILL_STAGE")) else {
        eprintln!("SKIP: ROLIDSCP_KILL_DIR / _OP / _STAGE not set");
        return;
    };
    let w = PathBuf::from(dir);
    let rt = crate::bench::rt();
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let journal = w.join("journal");
    rolid_core::replace::set_journal_dir(journal.clone());
    let src = w.join("src");
    let (up, mv_src, mv_dst) = (w.join(format!("{op}-to")), w.join(format!("{op}-from")), w.join(format!("{op}-to")));
    let needs_ssh = matches!(op.as_str(), "copy_up" | "copy_down" | "move_up" | "move_down" | "f4");
    let (sftp, _conn) = if needs_ssh {
        let (f, c) = rt.block_on(connect());
        (Some(f), Some(c))
    } else {
        (None, None)
    };
    // Up: local → server; down: server → local (the same disk underneath).
    let pair = |way_up: bool| -> (Arc<dyn Vfs>, Arc<dyn Vfs>) {
        let r = sftp.clone().unwrap();
        if way_up {
            (local.clone(), r)
        } else {
            (r, local.clone())
        }
    };
    match (op.as_str(), stage.as_str()) {
        (_, "prepare") => {
            write_tree(&src.join("data"));
            std::fs::create_dir_all(&journal).unwrap();
            let ctl_cancel = AtomicBool::new(false);
            let on = |_: rolid_archive::Tick<'_>| {};
            let ctl = rolid_archive::Ctl { cancel: &ctl_cancel, on: &on };
            rolid_archive::pack(&[src.join("data")], &src.join("data.zip"), rolid_archive::Format::Zip, rolid_archive::Level::Fast, None, &ctl).unwrap();
            rolid_config::write_atomic(&w.join("settings.toml"), settings_text(SETTINGS_A).as_bytes()).unwrap();
            let mut v = rolid_config::vault::Vault::create(&w.join("vault.bin"), &master(), rolid_config::vault::KdfParams::MIN).unwrap();
            v.set("p", SecretString::from(SETTINGS_A.to_string()));
            v.save().unwrap();
            std::fs::write(w.join("f4-old"), content("old", 8 << 20)).unwrap();
            std::fs::write(w.join("f4-new"), content("new", 8 << 20)).unwrap();
            std::fs::copy(w.join("f4-old"), w.join("f4-target")).unwrap();
        }
        ("copy_up" | "copy_down", "do") => {
            let (a, b) = pair(op == "copy_up");
            std::fs::create_dir_all(&up).unwrap();
            rt.block_on(copy(a.as_ref(), b.as_ref(), &src, &up, Conflict::Overwrite, false));
        }
        ("copy_up" | "copy_down", "check") => {
            let (ok, parts) = real_files_complete(&up.join("data"));
            let (a, b) = pair(op == "copy_up");
            // Killed before it made the target folder (in the program it is
            // the other panel's folder: always there).
            std::fs::create_dir_all(&up).unwrap();
            rt.block_on(copy(a.as_ref(), b.as_ref(), &src, &up, Conflict::Resume, false));
            all_complete(&up.join("data"));
            std::fs::remove_dir_all(&up).unwrap();
            println!("KILLCHECK {op} ok complete={ok} part_files={parts}");
        }
        ("move_up" | "move_down", "do") => {
            let (a, b) = pair(op == "move_up");
            std::fs::create_dir_all(&mv_dst).unwrap();
            rt.block_on(copy(a.as_ref(), b.as_ref(), &mv_src, &mv_dst, Conflict::Overwrite, true));
        }
        ("move_up" | "move_down", "check") => {
            // Every file is complete in the source or in the target (or both).
            let (in_src, in_dst) = (files(&mv_src.join("data")), files(&mv_dst.join("data")));
            real_files_complete(&mv_src.join("data"));
            let (ok, parts) = real_files_complete(&mv_dst.join("data"));
            for i in 0..FILES {
                assert!(in_src.contains(&rel(i)) || in_dst.contains(&rel(i)), "{} lost: neither in the source nor in the target", rel(i));
            }
            let left = (0..FILES).filter(|i| in_src.contains(&rel(*i))).count();
            let (a, b) = pair(op == "move_up");
            std::fs::create_dir_all(&mv_dst).unwrap();
            if mv_src.join("data").exists() {
                rt.block_on(copy(a.as_ref(), b.as_ref(), &mv_src, &mv_dst, Conflict::Resume, true));
            }
            all_complete(&mv_dst.join("data"));
            assert!(files(&mv_src.join("data")).is_empty(), "the source is empty after finishing the move");
            let _ = std::fs::remove_dir_all(&mv_src);
            std::fs::remove_dir_all(&mv_dst).unwrap();
            println!("KILLCHECK {op} ok in_target={ok} still_in_source={left} part_files={parts}");
        }
        ("f4", "do") => {
            let r = sftp.clone().unwrap();
            static NOOP: fn(u64) = |_| {};
            let ctl = rolid_edit::transfer::Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &NOOP };
            rt.block_on(rolid_edit::transfer::upload_replace(local.as_ref(), &s(&w.join("f4-new")), r.as_ref(), &s(&w.join("f4-target")), Some(0o644), &ctl)).unwrap();
        }
        ("f4", "check") => {
            let r = sftp.clone().unwrap();
            let rep = rt.block_on(rolid_core::replace::repair(r.as_ref()));
            let got = std::fs::read(w.join("f4-target")).expect("the edited file is there under its name");
            let which = if got == content("old", 8 << 20) {
                "old"
            } else if got == content("new", 8 << 20) {
                "new"
            } else {
                panic!("the edited file is neither the old nor the new version ({} B)", got.len());
            };
            for f in files(&w) {
                assert!(!f.starts_with("f4-target") || f == "f4-target" || f.ends_with(rolid_core::copy::PARTIAL_SUFFIX), "leftover {f}");
            }
            let _ = std::fs::remove_file(w.join("f4-target.filepart"));
            std::fs::copy(w.join("f4-old"), w.join("f4-target")).unwrap();
            println!("KILLCHECK f4 ok version={which} repaired={}", rep.restored.len() + rep.completed.len());
        }
        ("pack", "do") => {
            std::fs::create_dir_all(w.join("pack")).unwrap();
            let c = AtomicBool::new(false);
            // The record of the part file, as the program keeps it.
            let j = rolid_archive::journal::PartJournal::create(&w.join("parts")).unwrap();
            let on = |t: rolid_archive::Tick<'_>| j.note(&t);
            let ctl = rolid_archive::Ctl { cancel: &c, on: &on };
            rolid_archive::pack(&[src.join("data")], &w.join("pack/out.zip"), rolid_archive::Format::Zip, rolid_archive::Level::Normal, None, &ctl).unwrap();
        }
        ("pack", "check") => {
            // What the program does at its next start.
            let cleaned = rolid_archive::journal::clean(&w.join("parts")).len();
            let out = w.join("pack/out.zip");
            let state = if out.exists() {
                let c = AtomicBool::new(false);
                let on = |_: rolid_archive::Tick<'_>| {};
                let ctl = rolid_archive::Ctl { cancel: &c, on: &on };
                let t = rolid_archive::test(&out, rolid_archive::Format::Zip, &rolid_archive::ReadOpts::new(), &ctl).expect("a finished archive reads");
                assert!(t.bad.is_empty(), "{:?}", t.bad);
                assert_eq!(t.items as usize, FILES + 3, "every file and folder is in it");
                "complete"
            } else {
                "not_there"
            };
            // Anything else in the folder: a temporary file the killed process left.
            let left = files(&w.join("pack")).iter().filter(|f| f.as_str() != "out.zip").count();
            std::fs::remove_dir_all(w.join("pack")).unwrap();
            assert_eq!(left, 0, "the part file of the killed packing is cleaned up");
            println!("KILLCHECK pack ok archive={state} part_file_cleaned={cleaned} temporary_files_left={left}");
        }
        ("unpack", "do") => {
            std::fs::create_dir_all(w.join("unpack")).unwrap();
            let c = AtomicBool::new(false);
            let j = rolid_archive::journal::PartJournal::create(&w.join("parts")).unwrap();
            let on = |t: rolid_archive::Tick<'_>| j.note(&t);
            let ctl = rolid_archive::Ctl { cancel: &c, on: &on };
            rolid_archive::extract(&src.join("data.zip"), rolid_archive::Format::Zip, &w.join("unpack"), &rolid_archive::ReadOpts::new(), rolid_archive::Limits::default(), rolid_archive::Exist::Overwrite, None, &ctl).unwrap();
        }
        ("unpack", "check") => {
            let cleaned = rolid_archive::journal::clean(&w.join("parts")).len();
            let (ok, parts) = real_files_complete(&w.join("unpack/data"));
            std::fs::remove_dir_all(w.join("unpack")).unwrap();
            assert_eq!(parts, 0, "the part file of the killed extraction is cleaned up");
            println!("KILLCHECK unpack ok complete={ok} part_file_cleaned={cleaned} part_files={parts}");
        }
        ("trash", "do") => {
            // Every file on its own (as F8 on marked files), then the folders.
            let from = w.join("trash-from/data");
            for i in 0..FILES {
                rolid_local::trash::trash_with(&from.join(rel(i)), &w.join("trash-bin")).unwrap();
            }
            for d in ["a", "b"] {
                rolid_local::trash::trash_with(&from.join(d), &w.join("trash-bin")).unwrap();
            }
        }
        ("trash", "check") => {
            // Every file is complete in its place or in the trash with an info
            // file naming its place; nothing in the trash lacks its info.
            let from = w.join("trash-from/data");
            let bin = w.join("trash-bin");
            let left = real_files_complete(&from).0;
            let in_bin = files(&bin.join("files"));
            let mut found = 0;
            for f in &in_bin {
                let top = f.split('/').next().unwrap();
                let info = std::fs::read_to_string(bin.join("info").join(format!("{top}.trashinfo")))
                    .unwrap_or_else(|_| panic!("{f} is in the trash without its info file"));
                let place = info.lines().find_map(|l| l.strip_prefix("Path=")).expect("the info file names the place").to_string();
                let rest = f.strip_prefix(top).unwrap();
                // A file trashed alone (rest empty) or inside its trashed folder;
                // the rest is plain ASCII, the same encoded.
                let i = (0..FILES)
                    .find(|i| rolid_local::trash::encode_path(&from.join(rel(*i))) == format!("{place}{rest}"))
                    .unwrap_or_else(|| panic!("{f} in the trash is none of the files ({place})"));
                assert!(std::fs::read(bin.join("files").join(f)).unwrap() == expected(i), "{f} in the trash is damaged");
                found += 1;
            }
            assert_eq!(left + found, FILES, "every file is in its place or in the trash");
            // Info files of a move the kill stopped (allowed by the specification;
            // file managers skip them).
            let orphans = files(&bin.join("info"))
                .iter()
                .filter(|n| !bin.join("files").join(n.trim_end_matches(".trashinfo")).exists())
                .count();
            println!("KILLCHECK trash ok in_place={left} in_trash={found} info_without_file={orphans}");
        }
        ("settings", "do") => {
            let (a, b) = (settings_text(SETTINGS_A), settings_text(SETTINGS_B));
            let mut even = false;
            loop {
                even = !even;
                let t = if even { &b } else { &a };
                rolid_config::write_atomic(&w.join("settings.toml"), t.as_bytes()).unwrap();
            }
        }
        ("settings", "check") => {
            let got = std::fs::read_to_string(w.join("settings.toml")).expect("the settings file is there");
            let which = if got == settings_text(SETTINGS_A) {
                "a"
            } else if got == settings_text(SETTINGS_B) {
                "b"
            } else {
                panic!("the settings file is damaged ({} B)", got.len());
            };
            println!("KILLCHECK settings ok version={which}");
        }
        ("vault", "do") => {
            let mut v = rolid_config::vault::Vault::open(&w.join("vault.bin"), &master()).unwrap();
            let mut even = false;
            loop {
                even = !even;
                v.set("p", SecretString::from(if even { SETTINGS_B } else { SETTINGS_A }.to_string()));
                v.save().unwrap();
            }
        }
        ("vault", "check") => {
            let v = rolid_config::vault::Vault::open(&w.join("vault.bin"), &master()).expect("the vault opens");
            let p = v.get("p").expect("the password is there").expose_secret().to_string();
            assert!(p == SETTINGS_A || p == SETTINGS_B, "{p}");
            println!("KILLCHECK vault ok version={p}");
        }
        other => panic!("unknown {other:?}"),
    }
}
