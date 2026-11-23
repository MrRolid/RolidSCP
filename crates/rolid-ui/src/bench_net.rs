//! A bad line (roadmap 0.8.17): transfers to and from a server over a
//! network with latency, jitter, packet loss, a changing speed, repeated
//! outages and a frozen connection. `scripts/bench/badnet.sh` makes a private
//! network namespace (no root needed), starts a throwaway SSH server in it,
//! sets the line with `tc netem` and keeps changing it while this test copies
//! a big file and a folder of small files up and down through the program's
//! own code: one shared connection with the app's keepalive, automatic
//! reconnection and resumed transfers (as `rolid-ssh/tests/reconnect.rs`).
//! Every file is compared by SHA-256 afterwards.
//!
//! Ignored by a plain `cargo test`. Environment:
//!
//! - `ROLIDSCP_BIG_SSH`: the server (`port|user|client key|known_hosts`)
//! - `ROLIDSCP_NET_DIR`: the work folder (seen by the server too)
//! - `ROLIDSCP_NET_TAG`: the scenario's name in the reported values
//! - `ROLIDSCP_NET_MB`: size of the big file in MiB (default 64)
//! - `ROLIDSCP_NET_SMALL`: number of small files next to it (default 200)
//!
//! Prints `BENCH net.<tag>.<value>` lines; a wrong checksum or a transfer
//! that does not finish fails the test.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::copy::{CopyJob, Progress, MAX_AUTO_RETRIES};
use rolid_core::Vfs;
use rolid_local::LocalFs;
use rolid_ssh::link::drive_reconnect;
use rolid_ssh::{Attempt, ConnectParams, Link, RemoteFs, SshConnection};
use sha2::{Digest, Sha256};

use crate::bench::{ms, report};

/// Small files in the folder by default (many requests: latency counts).
const SMALL: usize = 200;

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

fn sha(p: &Path) -> String {
    use std::io::Read;
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// (relative name, SHA-256) of every file under `root`, sorted.
fn sums(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            } else {
                out.push((p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), sha(&p)));
            }
        }
    }
    out.sort();
    out
}

fn make_source(src: &Path, mb: usize, small: usize) {
    if src.join("data").exists() {
        return;
    }
    let d = src.join("data");
    std::fs::create_dir_all(d.join("small")).unwrap();
    std::fs::write(d.join("big.bin"), content(1, mb << 20)).unwrap();
    for i in 0..small {
        std::fs::write(d.join(format!("small/f{i:03}.txt")), content(100 + i as u64, 1000 + i * 37)).unwrap();
    }
}

/// What the app does every frame: notice a dead connection, reconnect.
fn supervise(link: Arc<Link>, params: ConnectParams, stop: Arc<AtomicBool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            link.check();
            if link.claim_driver() {
                let p = params.clone();
                drive_reconnect(link.clone(), move |_| {
                    let p = p.clone();
                    async move {
                        match SshConnection::connect(&p).await {
                            Ok(c) => Attempt::Connected(c),
                            Err(e) => Attempt::Retry(e.text()),
                        }
                    }
                })
                .await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
}

/// Copy `data` from `from_dir` to `to_dir` the way the transfer queue does:
/// on a lost connection wait for the reconnection and go on (resuming the
/// part files). Returns (time, waits for a reconnection, error).
async fn copy(link: &Arc<Link>, src: &dyn Vfs, from_dir: &Path, dst: &dyn Vfs, to_dir: &Path) -> (Duration, u32, Option<String>) {
    static NOOP: fn(&Progress) = |_| {};
    let cancel = Arc::new(AtomicBool::new(false));
    let mut job = CopyJob::new(src, dst, cancel.clone(), &NOOP);
    let waits = AtomicU64::new(0);
    let t = Instant::now();
    let r = job
        .copy_items_retrying(&[(from_dir.to_string_lossy().into_owned(), "data".into())], &to_dir.to_string_lossy(), MAX_AUTO_RETRIES * 4, |_| {
            waits.fetch_add(1, Ordering::Relaxed);
            // Why the transfer stopped (the log of the scenario keeps it).
            eprintln!("{:>7.1} s: interrupted, {:?}", t.elapsed().as_secs_f64(), link.state());
            let (link, cancel) = (link.clone(), cancel.clone());
            async move { link.wait_up(&cancel).await }
        })
        .await;
    (t.elapsed(), waits.load(Ordering::Relaxed) as u32, r.err().map(|e| e.to_string()))
}

#[test]
#[ignore]
fn bench_net() {
    let (Ok(spec), Ok(dir)) = (std::env::var("ROLIDSCP_BIG_SSH"), std::env::var("ROLIDSCP_NET_DIR")) else {
        eprintln!("SKIP: ROLIDSCP_BIG_SSH / ROLIDSCP_NET_DIR not set");
        return;
    };
    let tag = std::env::var("ROLIDSCP_NET_TAG").unwrap_or_else(|_| "line".into());
    let mb: usize = std::env::var("ROLIDSCP_NET_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let small: usize = std::env::var("ROLIDSCP_NET_SMALL").ok().and_then(|v| v.parse().ok()).unwrap_or(SMALL);
    let w = PathBuf::from(dir);
    let src = w.join(format!("src-{mb}-{small}"));
    make_source(&src, mb, small);
    let want = sums(&src.join("data"));
    let (up, down) = (w.join(format!("{tag}-up")), w.join(format!("{tag}-down")));
    for d in [&up, &down] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).unwrap();
    }
    let p: Vec<&str> = spec.split('|').collect();
    // The app's own connection settings (keepalive 15 s × 3).
    let params = ConnectParams {
        host: "127.0.0.1".into(),
        port: p[0].parse().unwrap(),
        user: p[1].into(),
        key_path: Some(p[2].into()),
        known_hosts: Some(p[3].into()),
        skip_agent: true,
        skip_default_keys: true,
        ..Default::default()
    };
    let rt = crate::bench::rt();
    rt.block_on(async {
        let t = Instant::now();
        // The first connection may meet an outage too: try for a while.
        let conn = loop {
            match SshConnection::connect(&params).await {
                Ok(c) => break c,
                Err(e) if t.elapsed() < Duration::from_secs(120) => {
                    eprintln!("connect: {}", e.text());
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => panic!("no connection: {}", e.text()),
            }
        };
        report(&format!("net.{tag}.connect_ms"), ms(t.elapsed()), "ms");
        let link = Link::new(conn);
        let stop = Arc::new(AtomicBool::new(false));
        let sup = supervise(link.clone(), params.clone(), stop.clone());
        let remote = RemoteFs::new(link.clone());
        let gen0 = link.generation();
        let mut failed = Vec::new();
        for (way, from, to) in [("up", &src, &up), ("down", &up, &down)] {
            let (a, b): (&dyn Vfs, &dyn Vfs) = if way == "up" { (&LocalFs, &remote) } else { (&remote, &LocalFs) };
            let (took, waits, err) = copy(&link, a, from, b, to).await;
            let got = sums(&to.join("data"));
            let ok = err.is_none() && got == want;
            report(&format!("net.{tag}.{way}_ms"), ms(took), "ms");
            report(&format!("net.{tag}.{way}_mb_s"), (mb as f64) / took.as_secs_f64().max(0.001), "MiB/s");
            report(&format!("net.{tag}.{way}_reconnect_waits"), waits as f64, "waits");
            report(&format!("net.{tag}.{way}_files_ok"), got.iter().filter(|g| want.contains(g)).count() as f64, "files");
            report(&format!("net.{tag}.{way}_checksums_ok"), f64::from(u8::from(ok)), "bool");
            if !ok {
                failed.push(format!("{way}: {:?}, {} of {} files right", err, got.iter().filter(|g| want.contains(g)).count(), want.len()));
            }
        }
        report(&format!("net.{tag}.reconnections"), (link.generation() - gen0) as f64, "times");
        stop.store(true, Ordering::Relaxed);
        link.stop();
        sup.abort();
        for d in [&up, &down] {
            let _ = std::fs::remove_dir_all(d);
        }
        assert!(failed.is_empty(), "{failed:?}");
    });
}
