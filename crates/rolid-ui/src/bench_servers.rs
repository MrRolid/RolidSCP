//! Servers of different kinds (roadmap 0.8.18): OpenSSH of several versions,
//! Dropbear, an account with SFTP only (no shell), a server with BusyBox tools
//! and a chroot SFTP account. `scripts/compat/servers.sh` starts each of them
//! (no root needed) and runs this test against it; it goes through the
//! program's own code for everything the panels do there:
//!
//! connect, list, a folder up and back (a big file, small files, a subfolder,
//! a link) compared by SHA-256, F4 (upload over a file, keeping its mode),
//! permissions, rename (also over an existing file), delete, an archive packed
//! here, uploaded, downloaded and extracted, free space, and the server tools
//! the program uses when there is a shell (find, grep, du, df, checksum
//! tools), each with what the program does when the tool is missing.
//!
//! Ignored by a plain `cargo test`. Environment:
//!
//! - `ROLIDSCP_COMPAT_SSH`: `host|port|user|client key|known_hosts`
//! - `ROLIDSCP_COMPAT_TAG`: the server's name in the report
//! - `ROLIDSCP_COMPAT_LOCAL`: a local work folder
//! - `ROLIDSCP_COMPAT_REMOTE`: the work folder on the server (default: a new
//!   folder in the home folder)
//!
//! Prints `COMPAT <tag> <step> <ok|fail|info> <detail>` lines; a failed
//! step that the program needs fails the test.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rolid_core::copy::{CopyJob, Progress};
use rolid_core::{EntryKind, Vfs};
use rolid_local::LocalFs;
use rolid_ssh::{ConnectParams, Link, RemoteFs, SshConnection};
use sha2::{Digest, Sha256};

fn content(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut v = Vec::with_capacity(len + 8);
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(len);
    v
}

fn sha_bytes(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}

/// (relative path, SHA-256 or "dir") of everything under `root`; a link to a
/// file counts as that file (a copy takes the content, as the program does).
fn sums(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let t = e.file_type().unwrap();
            if t.is_dir() {
                out.push((rel, "dir".into()));
                stack.push(p);
            } else {
                out.push((rel, sha_bytes(&std::fs::read(&p).unwrap())));
            }
        }
    }
    out.sort();
    out
}

// Marked although the whole module is test code: the check for untranslated
// text skips `#[cfg(test)]` items, and this one holds a non-English name.
#[cfg(test)]
fn make_tree(d: &Path) {
    std::fs::create_dir_all(d.join("sub/deeper")).unwrap();
    std::fs::write(d.join("big.bin"), content(1, 6 << 20)).unwrap();
    for i in 0..40 {
        std::fs::write(d.join(format!("small-{i:02}.txt")), content(10 + i, 500 + i as usize * 97)).unwrap();
    }
    std::fs::write(d.join("sub/deeper/note.txt"), b"a note\n").unwrap();
    std::fs::write(d.join("sub/with space and ü.txt"), content(3, 4096)).unwrap();
    std::os::unix::fs::symlink("small-01.txt", d.join("link-to-small")).unwrap();
}

struct Report {
    tag: String,
    failed: Mutex<Vec<String>>,
}

impl Report {
    fn line(&self, step: &str, verdict: &str, detail: &str) {
        let detail = detail.replace(['\n', '\t'], " ");
        println!("COMPAT\t{}\t{step}\t{verdict}\t{detail}", self.tag);
    }
    fn ok(&self, step: &str, detail: &str) {
        self.line(step, "ok", detail);
    }
    fn info(&self, step: &str, detail: &str) {
        self.line(step, "info", detail);
    }
    fn fail(&self, step: &str, detail: &str) {
        self.line(step, "fail", detail);
        self.failed.lock().unwrap().push(format!("{step}: {detail}"));
    }
    fn check(&self, step: &str, good: bool, detail: &str) {
        if good {
            self.ok(step, detail)
        } else {
            self.fail(step, detail)
        }
    }
}

async fn copy(src: &dyn Vfs, from_dir: &str, name: &str, dst: &dyn Vfs, to_dir: &str) -> Result<(), String> {
    static NOOP: fn(&Progress) = |_| {};
    let mut job = CopyJob::new(src, dst, Arc::new(AtomicBool::new(false)), &NOOP);
    job.copy_items_retrying(&[(from_dir.to_string(), name.to_string())], to_dir, 0, |_| async { false })
        .await
        .map_err(|e| e.to_string())
}

#[test]
#[ignore]
fn compat_server() {
    let (Ok(spec), Ok(local)) = (std::env::var("ROLIDSCP_COMPAT_SSH"), std::env::var("ROLIDSCP_COMPAT_LOCAL")) else {
        eprintln!("SKIP: ROLIDSCP_COMPAT_SSH / ROLIDSCP_COMPAT_LOCAL not set");
        return;
    };
    let rep = Report { tag: std::env::var("ROLIDSCP_COMPAT_TAG").unwrap_or_else(|_| "server".into()), failed: Mutex::default() };
    let p: Vec<&str> = spec.split('|').collect();
    let params = ConnectParams {
        host: p[0].into(),
        port: p[1].parse().unwrap(),
        user: p[2].into(),
        key_path: Some(p[3].into()),
        known_hosts: Some(p[4].into()),
        skip_agent: true,
        skip_default_keys: true,
        ..Default::default()
    };
    let local = PathBuf::from(local);
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(&local).unwrap();
    let rt = crate::bench::rt();
    rt.block_on(async {
        let t = Instant::now();
        let conn = match SshConnection::connect(&params).await {
            Ok(c) => c,
            Err(e) => {
                rep.fail("connect", &e.to_string());
                panic!("no connection: {e}");
            }
        };
        rep.ok("connect", &format!("{} ms", t.elapsed().as_millis()));
        let link = Link::new(conn.clone());
        let fs = RemoteFs::new(link.clone());
        let fs: &dyn Vfs = &fs;

        // Where to work.
        let home = fs.home().await;
        rep.check("home", home.is_ok(), &format!("{home:?}"));
        let base = match std::env::var("ROLIDSCP_COMPAT_REMOTE") {
            Ok(b) => b,
            Err(_) => fs.join(home.as_deref().unwrap_or("/tmp"), "rolidscp-compat"),
        };
        // Only a folder this test makes is ever deleted.
        if fs.stat(&base).await.is_ok() {
            rep.fail("mkdir", &format!("{base} exists already: not touched"));
            panic!("{base} exists");
        }
        let made = fs.mkdir(&base).await;
        rep.check("mkdir", made.is_ok(), &format!("{base}: {made:?}"));
        let root_list = fs.list("/").await;
        rep.check("list /", root_list.is_ok(), &match &root_list {
            Ok(l) => format!("{} entries", l.len()),
            Err(e) => e.to_string(),
        });

        // A folder up and back.
        let src = local.join("src");
        make_tree(&src.join("data"));
        let want = sums(&src.join("data"));
        let t = Instant::now();
        let up = copy(&LocalFs, &src.to_string_lossy(), "data", fs, &base).await;
        rep.check("upload", up.is_ok(), &format!("{} ms {up:?}", t.elapsed().as_millis()));
        let listed = fs.list(&fs.join(&base, "data")).await;
        match &listed {
            Ok(l) => {
                let link = l.iter().find(|e| e.name == "link-to-small");
                rep.check("list", l.len() == want.iter().filter(|(p, _)| !p.contains('/')).count(), &format!("{} entries", l.len()));
                // The program copies the content of a link to a file.
                rep.check("link copied as its file", link.is_some_and(|e| e.kind == EntryKind::File), &format!("{:?}", link.map(|e| e.kind)));
            }
            Err(e) => rep.fail("list", &e.to_string()),
        }
        let down = local.join("down");
        std::fs::create_dir_all(&down).unwrap();
        let t = Instant::now();
        let back = copy(fs, &base, "data", &LocalFs, &down.to_string_lossy()).await;
        rep.check("download", back.is_ok(), &format!("{} ms {back:?}", t.elapsed().as_millis()));
        let got = sums(&down.join("data"));
        let same = got == want;
        let diff: Vec<String> = want.iter().filter(|w| !got.contains(w)).map(|w| w.0.clone()).take(5).collect();
        rep.check("checksums", same, &format!("{} of {} items the same; differ: {diff:?}", got.iter().filter(|g| want.contains(g)).count(), want.len()));
        let big = fs.stat(&fs.join(&base, "data/big.bin")).await;
        let mtime_src = std::fs::metadata(src.join("data/big.bin")).unwrap().modified().ok();
        rep.check(
            "times kept",
            big.as_ref().ok().and_then(|b| b.modified).zip(mtime_src).is_some_and(|(a, b)| {
                let d = if a > b { a.duration_since(b) } else { b.duration_since(a) };
                d.unwrap_or_default().as_secs() <= 1
            }),
            &format!("{:?}", big.as_ref().ok().and_then(|b| b.modified)),
        );

        // F4: an edited file goes up over the original, keeping its mode.
        let edited = local.join("edited.txt");
        std::fs::write(&edited, content(77, 300_000)).unwrap();
        let target = fs.join(&base, "data/small-05.txt");
        let _ = fs.set_attrs(&target, Some(0o640), None).await;
        static NOOP: fn(u64) = |_| {};
        let ctl = rolid_edit::transfer::Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &NOOP };
        let f4 = rolid_edit::transfer::upload_replace(&LocalFs, &edited.to_string_lossy(), fs, &target, Some(0o640), &ctl).await;
        let after = fs.stat(&target).await;
        let back = local.join("f4-back.txt");
        let dl = rolid_edit::transfer::download(fs, &target, &LocalFs, &back.to_string_lossy(), &ctl).await;
        let same = dl.is_ok() && std::fs::read(&back).ok() == std::fs::read(&edited).ok();
        rep.check("F4 upload", f4.is_ok() && same, &format!("{f4:?}, content the same: {same}"));
        let mode = after.as_ref().ok().and_then(|e| e.mode).map(|m| m & 0o777);
        rep.check("F4 keeps mode", mode == Some(0o640), &format!("{mode:?}"));
        let names: Vec<String> = fs.list(&fs.join(&base, "data")).await.map(|l| l.into_iter().map(|e| e.name).collect()).unwrap_or_default();
        let leftovers: Vec<&String> = names.iter().filter(|n| n.contains(".filepart") || n.contains(".rolidscp")).collect();
        rep.check("F4 leaves nothing", leftovers.is_empty(), &format!("{leftovers:?}"));

        // Permissions.
        let f = fs.join(&base, "data/small-06.txt");
        let set = fs.set_attrs(&f, Some(0o600), None).await;
        let m = fs.stat(&f).await.ok().and_then(|e| e.mode).map(|m| m & 0o777);
        rep.check("chmod", set.is_ok() && m == Some(0o600), &format!("{set:?} → {m:?}"));
        let d = fs.join(&base, "data/sub");
        let set = fs.set_attrs(&d, Some(0o750), None).await;
        let m = fs.stat(&d).await.ok().and_then(|e| e.mode).map(|m| m & 0o777);
        rep.check("chmod folder", set.is_ok() && m == Some(0o750), &format!("{set:?} → {m:?}"));

        // Rename, and replace an existing file in one step.
        let (a, b) = (fs.join(&base, "data/small-07.txt"), fs.join(&base, "data/renamed.txt"));
        let r = fs.rename(&a, &b).await;
        rep.check("rename", r.is_ok() && fs.stat(&b).await.is_ok() && fs.stat(&a).await.is_err(), &format!("{r:?}"));
        let over = fs.rename_replace(&b, &fs.join(&base, "data/small-08.txt")).await;
        rep.info("rename over a file", &match over {
            Ok(true) => "in one step (posix-rename)".to_string(),
            Ok(false) => "the server cannot: the program renames the old file aside first".to_string(),
            Err(e) => format!("error: {e}"),
        });
        let _ = copy(&LocalFs, &src.join("data").to_string_lossy(), "small-10.txt", fs, &fs.join(&base, "data")).await;
        let safe = rolid_core::replace::safe_replace(fs, &fs.join(&base, "data/small-10.txt"), &fs.join(&base, "data/small-09.txt")).await;
        rep.check("replace", safe.is_ok(), &format!("{safe:?}"));

        // An archive made here goes up, comes back and is extracted.
        let cancel = AtomicBool::new(false);
        let on = |_: rolid_archive::Tick<'_>| {};
        let actl = rolid_archive::Ctl { cancel: &cancel, on: &on };
        let zip = local.join("arch/data.zip");
        std::fs::create_dir_all(zip.parent().unwrap()).unwrap();
        let packed = rolid_archive::pack(&[src.join("data")], &zip, rolid_archive::Format::Zip, rolid_archive::Level::Fast, None, &actl);
        let up = copy(&LocalFs, &local.join("arch").to_string_lossy(), "data.zip", fs, &base).await;
        let back_dir = local.join("arch-back");
        std::fs::create_dir_all(back_dir.join("x")).unwrap();
        let down = copy(fs, &base, "data.zip", &LocalFs, &back_dir.to_string_lossy()).await;
        let ex = rolid_archive::extract(
            &back_dir.join("data.zip"),
            rolid_archive::Format::Zip,
            &back_dir.join("x"),
            &rolid_archive::ReadOpts::new(),
            rolid_archive::Limits::default(),
            rolid_archive::Exist::Overwrite,
            None,
            &actl,
        );
        let same = ex.is_ok() && sums(&back_dir.join("x/data")) == want;
        rep.check("archive", packed.is_ok() && up.is_ok() && down.is_ok() && same, &format!("pack {:?} up {up:?} down {down:?} extract {:?} same {same}", packed.err(), ex.err()));

        // Free space.
        let space = fs.space(&base).await;
        rep.info("free space", &match space {
            Ok(Some(s)) => format!("{} of {} MB free (statvfs)", s.avail >> 20, s.total >> 20),
            Ok(None) => "not offered: the program shows no free space".into(),
            Err(e) => format!("error: {e}"),
        });

        // Server tools (a shell is needed; without one the program walks over SFTP).
        let caps = rolid_search::remote::probe(&conn, &cancel).await;
        rep.info("find/grep", &format!("GNU find: {}, GNU grep: {}", caps.gnu_find, caps.gnu_grep));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let h2 = hits.clone();
        let emit = move |h: rolid_search::Hit| h2.lock().unwrap().push(h.rel);
        let sink = rolid_search::Sink::new(&emit, 1000, &cancel);
        let q = rolid_search::Query { root: fs.join(&base, "data"), name: "note*".into(), ..Default::default() };
        let found = rolid_search::remote::search_remote(&conn, fs, &q, &sink).await;
        let n = hits.lock().unwrap().len();
        rep.check("search", found.is_ok() && n == 1, &format!("{:?}, {n} found", found.as_ref().map(|o| o.method)));
        let q = rolid_search::Query { root: fs.join(&base, "data"), name: "*".into(), content: "a note".into(), ..Default::default() };
        hits.lock().unwrap().clear();
        let sink = rolid_search::Sink::new(&emit, 1000, &cancel);
        let found = rolid_search::remote::search_remote(&conn, fs, &q, &sink).await;
        let n = hits.lock().unwrap().len();
        rep.check("search text", found.is_ok() && n == 1, &format!("{:?}, {n} found", found.as_ref().map(|o| o.method)));
        let dir = fs.join(&base, "data");
        let du = crate::dirsize::remote_du(&conn, &dir, &cancel).await;
        let walked = crate::dirsize::walk(fs, &dir, &cancel, &mut |_| {}).await;
        let want_bytes = walked.as_ref().map_or(0, |w| w.bytes);
        rep.info("folder size", &match &du {
            Ok(Some(s)) => format!("du: {} B (SFTP walk: {} B)", s.bytes, want_bytes),
            Ok(None) | Err(_) => format!("no usable du ({:?}): SFTP walk {} B", du.as_ref().err(), want_bytes),
        });
        rep.check("folder size", walked.is_some(), "the SFTP walk always works");
        let fstype = crate::app::remote_fs_type(&link, &base).await;
        rep.info("file system", &format!("{fstype:?}"));
        for algo in [crate::checksum::Algo::Sha256] {
            let probe = crate::checksum::probe_command(algo);
            let mut out = Vec::new();
            let tool = match &probe {
                Some(cmd) => conn
                    .exec_probe(cmd, &cancel, &mut |d| {
                        out.extend_from_slice(d);
                        out.len() < 256
                    })
                    .await
                    .is_ok_and(|_| {
                        let o = String::from_utf8_lossy(&out);
                        o.contains("GNU") || o.contains("B3")
                    }),
                None => false,
            };
            rep.info("checksum", &format!("{algo:?}: {}", if tool { "server tool" } else { "files read over SFTP" }));
        }

        // Delete everything.
        let t = Instant::now();
        let del = crate::app::remove_recursive(fs, &base).await;
        let gone = fs.stat(&base).await.is_err();
        rep.check("delete", del.is_ok() && gone, &format!("{} ms {del:?}", t.elapsed().as_millis()));
        link.stop();
    });
    let _ = std::fs::remove_dir_all(&local);
    let failed = rep.failed.lock().unwrap().clone();
    assert!(failed.is_empty(), "{failed:#?}");
}
