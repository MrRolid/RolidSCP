//! Disk space (roadmap 0.8.16): a full disk in the middle of a transfer, on
//! both sides, and sparse files.
//!
//! `bench_full`: `scripts/bench/space.sh` mounts a small tmpfs in a user namespace of its
//! own (no root needed) and starts a throwaway SSH server; the tmpfs is
//! reached through `/proc/PID/root`, by this test and by the server alike.
//!
//! For an upload (the server's disk fills up) and a download (this
//! computer's disk fills up): a filler leaves less room than the file needs,
//! an older file of the same name is in the target. The copy must fail with a
//! message that names the full disk, leave the older file as it was and keep
//! the written part; with the filler deleted, "Try again" (a new job with the
//! old one's state, as the transfer queue does it) must finish the file, the
//! same as its source.
//!
//! `bench_sparse`: a file of `ROLIDSCP_SPARSE_GIB` GiB (default 2) that is
//! almost all holes, with data at its start, middle and end, copied to the
//! server and back and on this computer; each copy is compared with the
//! source, and the space it takes on the disk is reported.
//!
//! Settings from the environment:
//! - `ROLIDSCP_SPARSE_DIR`: a folder on a real disk (not tmpfs)
//! - `ROLIDSCP_FULL_DIR`: a folder on the small tmpfs
//! - `ROLIDSCP_FULL_SRC`: a folder on a real disk for the source files
//! - `ROLIDSCP_FULL_ROOM`: free bytes the filler leaves (default 40 MiB)
//! - `ROLIDSCP_BIG_SSH`: the server (see `bench_big`)

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::Vfs;

use crate::bench::{report, rt, s};

const MIB: u64 = 1024 * 1024;
const SIZE: u64 = 100 * MIB;

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

/// Fill `dir` so that about `room` bytes stay free.
fn fill(dir: &Path, room: u64) {
    let free = rt().block_on(rolid_local::LocalFs.space(&s(dir))).expect("space").expect("space known").avail;
    let n = free.saturating_sub(room);
    let f = std::fs::File::create(dir.join("filler")).unwrap();
    f.set_len(0).unwrap();
    use std::io::Write;
    let mut w = std::io::BufWriter::new(f);
    let block = vec![0x5au8; MIB as usize];
    let mut left = n;
    while left > 0 {
        let k = left.min(MIB) as usize;
        w.write_all(&block[..k]).unwrap();
        left -= k as u64;
    }
    w.flush().unwrap();
}

fn source(dir: &Path) -> std::path::PathBuf {
    let p = dir.join("big.bin");
    if !p.exists() {
        // Not all zeros (tmpfs does not keep holes on writes, but let the
        // content be checked meaningfully).
        let mut data = Vec::with_capacity(SIZE as usize);
        let mut x = 0x9e37_79b9u32;
        while data.len() < SIZE as usize {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            data.extend_from_slice(&x.to_le_bytes());
        }
        std::fs::write(&p, &data).unwrap();
    }
    p
}

fn digest(p: &Path) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(p).unwrap()).unwrap();
    h.finalize()
}

#[test]
#[ignore]
fn bench_full() {
    let (Some(full), Some(srcdir)) = (crate::bench::env_dir("ROLIDSCP_FULL_DIR"), crate::bench::env_dir("ROLIDSCP_FULL_SRC")) else {
        return;
    };
    let room: u64 = std::env::var("ROLIDSCP_FULL_ROOM").ok().and_then(|v| v.parse().ok()).unwrap_or(40 * MIB);
    let rt = rt();
    let (sftp, _conn) = rt.block_on(connect());
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let src = source(&srcdir);
    let want = digest(&src);
    for way in ["up", "down"] {
        let (from, to) = if way == "up" { (local.clone(), sftp.clone()) } else { (sftp.clone(), local.clone()) };
        let target = full.join(format!("{way}.bin"));
        std::fs::write(&target, b"the older file").unwrap();
        fill(&full, room);
        let items = [(s(&srcdir), "big.bin".to_string())];
        // Copied under the name of the older file: SRC/big.bin -> FULL/<way>.bin.
        let done = AtomicU64::new(0);
        let cb = |p: &Progress| done.store(p.bytes_done, Ordering::Relaxed);
        let t = Instant::now();
        let mut job = CopyJob::new(from.as_ref(), to.as_ref(), Arc::new(AtomicBool::new(false)), &cb)
            .with_conflict(Conflict::Overwrite)
            .with_target_name(Some(format!("{way}.bin")));
        let r = rt.block_on(job.copy_items_retrying(&items, &s(&full), 0, |_| async { false }));
        let failed_after = t.elapsed();
        let state = job.state().clone();
        drop(job);
        let text = match &r {
            Err(e) => rolid_i18n::t(&e.text()),
            Ok(()) => String::new(),
        };
        println!("FULLMSG\t{way}\t{}", text.escape_debug());
        // Also into the results file (stdout lines can be cut up by other output).
        if let Some(out) = std::env::var_os("ROLIDSCP_BENCH_OUT") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).create(true).open(out).unwrap();
            writeln!(f, "MESSAGE\tfull.{way}\t{}", text.escape_debug()).unwrap();
        }
        report(&format!("full.{way}.failed"), r.is_err() as u8 as f64, "bool");
        report(&format!("full.{way}.message_names_full_disk"), text.starts_with("The target disk is full") as u8 as f64, "bool");
        report(&format!("full.{way}.failed_after_ms"), failed_after.as_secs_f64() * 1000.0, "ms");
        report(&format!("full.{way}.written_before"), done.load(Ordering::Relaxed) as f64, "B");
        report(&format!("full.{way}.older_file_kept"), (std::fs::read(&target).ok().as_deref() == Some(b"the older file")) as u8 as f64, "bool");
        let part = full.join(format!("{way}.bin.filepart"));
        report(&format!("full.{way}.part_kept"), std::fs::metadata(&part).map_or(0, |m| m.len()) as f64, "B");
        // Room again: "Try again".
        std::fs::remove_file(full.join("filler")).unwrap();
        let (done2, resumed2) = (AtomicU64::new(0), AtomicU64::new(0));
        let cb2 = |p: &Progress| {
            done2.store(p.bytes_done, Ordering::Relaxed);
            resumed2.store(p.bytes_resumed, Ordering::Relaxed);
        };
        let mut job = CopyJob::new(from.as_ref(), to.as_ref(), Arc::new(AtomicBool::new(false)), &cb2)
            .with_conflict(Conflict::Overwrite)
            .with_target_name(Some(format!("{way}.bin")))
            .with_state(state);
        let r2 = rt.block_on(job.copy_items_retrying(&items, &s(&full), 0, |_| async { false }));
        drop(job);
        report(&format!("full.{way}.try_again_ok"), r2.is_ok() as u8 as f64, "bool");
        report(&format!("full.{way}.try_again_resumed"), resumed2.load(Ordering::Relaxed) as f64, "B");
        report(&format!("full.{way}.try_again_transferred"), done2.load(Ordering::Relaxed) as f64, "B");
        report(&format!("full.{way}.same_as_source"), (std::fs::metadata(&target).is_ok() && digest(&target) == want) as u8 as f64, "bool");
        let leftovers = std::fs::read_dir(&full).unwrap().filter(|e| e.as_ref().unwrap().file_name() != format!("{way}.bin").as_str()).count();
        report(&format!("full.{way}.leftovers"), leftovers as f64, "files");
        std::fs::remove_file(&target).unwrap();
    }
}

/// The source of `bench_sparse`: holes with 1 MiB of data at the start, in the
/// middle and at the end.
fn sparse_source(p: &Path, size: u64) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::File::create(p).unwrap();
    f.set_len(size).unwrap();
    let data: Vec<u8> = (0..MIB as usize).map(|i| (i * 31 % 251) as u8 + 1).collect();
    for at in [0, size / 2, size - MIB] {
        f.seek(SeekFrom::Start(at)).unwrap();
        f.write_all(&data).unwrap();
    }
    f.sync_all().unwrap();
}

/// Bytes the file takes on the disk.
fn allocated(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.blocks() * 512)
}

#[test]
#[ignore]
fn bench_sparse() {
    let Some(dir) = crate::bench::env_dir("ROLIDSCP_SPARSE_DIR") else { return };
    let gib: u64 = std::env::var("ROLIDSCP_SPARSE_GIB").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let size = gib * 1024 * MIB;
    let rt = rt();
    let (sftp, _conn) = rt.block_on(connect());
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let src = a.join("sparse.bin");
    sparse_source(&src, size);
    report("sparse.size", size as f64, "B");
    report("sparse.source_on_disk", allocated(&src) as f64, "B");
    let want = digest(&src);
    // (name, from, to, source folder): up and down go through the server.
    for (way, from, to) in [("local", &local, &local), ("up", &local, &sftp), ("down", &sftp, &local)] {
        let (from_dir, to_dir) = if way == "down" { (&b, &a) } else { (&a, &b) };
        let name = if way == "down" { "up.bin" } else { "sparse.bin" };
        let target_name = format!("{way}.bin");
        static NOOP: fn(&Progress) = |_| {};
        let t = Instant::now();
        let mut job = CopyJob::new(from.as_ref(), to.as_ref(), Arc::new(AtomicBool::new(false)), &NOOP).with_target_name(Some(target_name.clone()));
        let r = rt.block_on(job.copy_items_retrying(&[(s(from_dir), name.to_string())], &s(to_dir), 0, |_| async { false }));
        drop(job);
        let secs = t.elapsed().as_secs_f64();
        let target = to_dir.join(&target_name);
        report(&format!("sparse.{way}.ok"), r.is_ok() as u8 as f64, "bool");
        report(&format!("sparse.{way}.seconds"), secs, "s");
        report(&format!("sparse.{way}.same_as_source"), (digest(&target) == want) as u8 as f64, "bool");
        report(&format!("sparse.{way}.size"), std::fs::metadata(&target).map_or(0, |m| m.len()) as f64, "B");
        report(&format!("sparse.{way}.on_disk"), allocated(&target) as f64, "B");
        // "down" reads the upload: delete it only afterwards.
        if way != "up" {
            std::fs::remove_file(&target).unwrap();
        }
    }
    let _ = std::fs::remove_file(b.join("up.bin"));
    std::fs::remove_file(&src).unwrap();
}
