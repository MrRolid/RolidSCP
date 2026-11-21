//! Big files (roadmap 0.8.16): one real file of over 100 GB copied to a
//! server and back through the program's own copy code, with what the window
//! shows measured once a second: the counters, the speed and the remaining
//! time (the same [`rolid_core::eta::Window`] as the transfer queue), memory
//! and open files. `scripts/bench/big.sh` creates the file, starts the
//! throwaway SSH server, runs the stages, kills a stage in the middle of the
//! transfer, compares checksums and deletes everything.
//!
//! Ignored by a plain `cargo test`. Settings come from the environment:
//!
//! - `ROLIDSCP_BIG_SSH`: `port|user|client key|known_hosts` of the server
//! - `ROLIDSCP_BIG_DIR`: `up` (local → server) or `down` (server → local)
//! - `ROLIDSCP_BIG_SRC`: the source file, `ROLIDSCP_BIG_DST`: the target folder
//! - `ROLIDSCP_BIG_NAME`: the target's name (default: the source's)
//! - `ROLIDSCP_BIG_STAGE`: `copy` (the whole file; the script may kill it),
//!   `cancel` (canceled at `ROLIDSCP_BIG_AT` percent, default 50, then
//!   continued by a new job with "Resume"), `resume` (continue a killed one)
//! - `ROLIDSCP_BIG_TAG`: prefix of the reported names

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::eta::{Eta, Window};
use rolid_core::Vfs;

use crate::bench::{open_fds, report, status_kb};

fn var(name: &str) -> Option<String> {
    let v = std::env::var(name).ok();
    if v.is_none() {
        eprintln!("SKIP: {name} not set");
    }
    v
}

async fn connect(spec: &str) -> (Arc<dyn Vfs>, rolid_ssh::SshConnection) {
    let p: Vec<&str> = spec.split('|').collect();
    let params = rolid_ssh::ConnectParams {
        host: "127.0.0.1".into(),
        port: p[0].parse().expect("port"),
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

/// What the progress callback saw.
#[derive(Default)]
struct Seen {
    /// Bytes done + bytes resumed (what the window shows).
    shown: AtomicU64,
    done: AtomicU64,
    resumed: AtomicU64,
    /// The shown count went down at least once.
    backwards: AtomicU64,
    started: Mutex<Option<Instant>>,
}

/// One sample a second: (seconds since start, bytes shown, remaining-time
/// estimate in seconds, RSS kB, open files).
type Sample = (f64, u64, Option<u64>, u64, usize);

struct Run {
    samples: Vec<Sample>,
    result: Result<(), String>,
    elapsed: Duration,
    resumed: u64,
    done: u64,
    backwards: u64,
}

#[allow(clippy::too_many_arguments)]
fn run_job(src: &dyn Vfs, dst: &dyn Vfs, dir: &str, name: &str, dst_dir: &str, target: &str, size: u64, conflict: Conflict, cancel_at: Option<u64>) -> Run {
    let rt = crate::bench::rt();
    let seen = Arc::new(Seen::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let (s2, c2) = (seen.clone(), cancel.clone());
    let cb = move |p: &Progress| {
        let shown = p.bytes_done + p.bytes_resumed;
        if s2.shown.swap(shown, Ordering::Relaxed) > shown {
            s2.backwards.fetch_add(1, Ordering::Relaxed);
        }
        s2.done.store(p.bytes_done, Ordering::Relaxed);
        s2.resumed.store(p.bytes_resumed, Ordering::Relaxed);
        if let Some(at) = cancel_at {
            if shown >= at {
                c2.store(true, Ordering::Relaxed);
            }
        }
    };
    // The sampler: what the transfer queue shows, once a second.
    let stop = Arc::new(AtomicBool::new(false));
    let (s3, stop2) = (seen.clone(), stop.clone());
    let sampler = std::thread::spawn(move || {
        let mut win = Window::default();
        let mut out: Vec<Sample> = Vec::new();
        let t0 = Instant::now();
        while !stop2.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            let start = *s3.started.lock().unwrap();
            let Some(start) = start else { continue };
            let (done, resumed) = (s3.done.load(Ordering::Relaxed), s3.resumed.load(Ordering::Relaxed));
            win.sample(start.elapsed(), done);
            let eta = match win.eta(size.saturating_sub(resumed).saturating_sub(done)) {
                Eta::Secs(s) => Some(s),
                _ => None,
            };
            out.push((t0.elapsed().as_secs_f64(), done + resumed, eta, status_kb("VmRSS"), open_fds()));
        }
        out
    });
    *seen.started.lock().unwrap() = Some(Instant::now());
    let t = Instant::now();
    let result = rt.block_on(async {
        let mut job = CopyJob::new(src, dst, cancel.clone(), &cb).with_conflict(conflict).with_target_name(Some(target.to_string()));
        job.copy_items_retrying(&[(dir.to_string(), name.to_string())], dst_dir, 0, |_| async { false }).await
    });
    let elapsed = t.elapsed();
    stop.store(true, Ordering::Relaxed);
    let samples = sampler.join().unwrap();
    Run {
        samples,
        result: result.map_err(|e| rolid_i18n::t(&e.text())),
        elapsed,
        resumed: seen.resumed.load(Ordering::Relaxed),
        done: seen.done.load(Ordering::Relaxed),
        backwards: seen.backwards.load(Ordering::Relaxed),
    }
}

/// Speed, the remaining-time estimate against the real remaining time,
/// memory and open files of a run that went to the end.
fn report_run(tag: &str, size: u64, r: &Run) {
    report(&format!("{tag}.seconds"), r.elapsed.as_secs_f64(), "s");
    report(&format!("{tag}.bytes_done"), r.done as f64, "B");
    report(&format!("{tag}.bytes_resumed"), r.resumed as f64, "B");
    report(&format!("{tag}.rate"), r.done as f64 / r.elapsed.as_secs_f64() / 1e6, "MB/s");
    report(&format!("{tag}.counter_backwards"), r.backwards as f64, "times");
    let end = r.samples.last().map_or(0.0, |s| s.0);
    // The estimate when 10, 25, 50, 75 % of what was left at the start was done.
    let first = r.samples.first().map_or(0, |s| s.1);
    for pct in [10u64, 25, 50, 75] {
        let at = first + (size - first) * pct / 100;
        if let Some(s) = r.samples.iter().find(|s| s.1 >= at && s.2.is_some()) {
            let real = (end - s.0).max(1.0);
            let est = s.2.unwrap() as f64;
            report(&format!("{tag}.eta_at_{pct}pct.estimate"), est, "s");
            report(&format!("{tag}.eta_at_{pct}pct.real"), real, "s");
            report(&format!("{tag}.eta_at_{pct}pct.error"), (est - real).abs() / real * 100.0, "%");
        }
    }
    // Memory once the transfer runs (after the first 10 %).
    let steady: Vec<&Sample> = r.samples.iter().filter(|s| s.1 >= first + (size - first) / 10).collect();
    if let (Some(lo), Some(hi)) = (steady.iter().map(|s| s.3).min(), steady.iter().map(|s| s.3).max()) {
        report(&format!("{tag}.rss_min"), lo as f64 / 1024.0, "MB");
        report(&format!("{tag}.rss_max"), hi as f64 / 1024.0, "MB");
        report(&format!("{tag}.rss_spread"), (hi - lo) as f64 / 1024.0, "MB");
    }
    if let Some(f) = r.samples.iter().map(|s| s.4).max() {
        report(&format!("{tag}.fds_max"), f as f64, "fds");
    }
    // The speed over each minute (a USB disk under long writes varies).
    let mut minute = Vec::new();
    for w in r.samples.windows(2) {
        if (w[1].0 / 60.0).floor() > (w[0].0 / 60.0).floor() || minute.is_empty() {
            minute.push(w[1]);
        }
    }
    let rates: Vec<f64> = minute.windows(2).map(|w| (w[1].1 - w[0].1) as f64 / (w[1].0 - w[0].0) / 1e6).collect();
    if let (Some(lo), Some(hi)) = (rates.iter().cloned().reduce(f64::min), rates.iter().cloned().reduce(f64::max)) {
        report(&format!("{tag}.rate_minute_min"), lo, "MB/s");
        report(&format!("{tag}.rate_minute_max"), hi, "MB/s");
    }
}

#[test]
#[ignore]
fn bench_big() {
    let (Some(ssh), Some(way), Some(src), Some(dst_dir), Some(stage)) =
        (var("ROLIDSCP_BIG_SSH"), var("ROLIDSCP_BIG_DIR"), var("ROLIDSCP_BIG_SRC"), var("ROLIDSCP_BIG_DST"), var("ROLIDSCP_BIG_STAGE"))
    else {
        return;
    };
    let tag = std::env::var("ROLIDSCP_BIG_TAG").unwrap_or_else(|_| format!("big.{way}.{stage}"));
    let rt = crate::bench::rt();
    let (sftp, _conn) = rt.block_on(connect(&ssh));
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let (from, to) = if way == "up" { (local.clone(), sftp.clone()) } else { (sftp.clone(), local.clone()) };
    let (dir, name) = src.rsplit_once('/').expect("absolute source path");
    let target = std::env::var("ROLIDSCP_BIG_NAME").unwrap_or_else(|_| name.to_string());
    let size = rt.block_on(from.stat(&src)).expect("source").size;
    report(&format!("{tag}.size"), size as f64, "B");
    let fd0 = open_fds();
    match stage.as_str() {
        "copy" => {
            let r = run_job(from.as_ref(), to.as_ref(), dir, name, &dst_dir, &target, size, Conflict::Overwrite, None);
            r.result.as_ref().expect("copy");
            report_run(&tag, size, &r);
        }
        "resume" => {
            let r = run_job(from.as_ref(), to.as_ref(), dir, name, &dst_dir, &target, size, Conflict::Resume, None);
            r.result.as_ref().expect("resume");
            report_run(&tag, size, &r);
        }
        "cancel" => {
            let pct: u64 = std::env::var("ROLIDSCP_BIG_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
            let at = size / 100 * pct;
            let r = run_job(from.as_ref(), to.as_ref(), dir, name, &dst_dir, &target, size, Conflict::Overwrite, Some(at));
            report(&format!("{tag}.first.canceled"), r.result.is_err() as u8 as f64, "bool");
            report(&format!("{tag}.first.bytes_done"), r.done as f64, "B");
            report(&format!("{tag}.first.seconds"), r.elapsed.as_secs_f64(), "s");
            let part = format!("{dst_dir}/{target}.filepart");
            let part_size = rt.block_on(to.stat(&part)).map(|e| e.size).unwrap_or(0);
            report(&format!("{tag}.first.filepart"), part_size as f64, "B");
            let r2 = run_job(from.as_ref(), to.as_ref(), dir, name, &dst_dir, &target, size, Conflict::Resume, None);
            r2.result.as_ref().expect("resume");
            report(&format!("{tag}.resumed_from_filepart"), (r2.resumed == part_size || r2.resumed + rolid_core::copy::RESUME_MARGIN == part_size) as u8 as f64, "bool");
            report_run(&format!("{tag}.second"), size, &r2);
        }
        other => panic!("unknown stage {other}"),
    }
    report(&format!("{tag}.fds_after"), open_fds().saturating_sub(fd0) as f64, "fds extra");
    let st = rt.block_on(to.stat(&format!("{dst_dir}/{target}"))).expect("target");
    assert_eq!(st.size, size, "target size");
}
