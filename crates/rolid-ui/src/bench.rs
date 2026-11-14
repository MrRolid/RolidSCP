//! Load tests with many files (roadmap 0.8.15). They measure the code the
//! program itself runs – listing, the panel's sorting and filter, folder sizes,
//! search, copying and deleting – and print one line per value:
//!
//! `BENCH <tab> name <tab> value <tab> unit`
//!
//! They are ignored by a plain `cargo test`; `scripts/bench/measure.sh` creates
//! the file trees, runs them with `--ignored` in a release build and writes the
//! table. Each test reads its tree from an environment variable and is skipped
//! (with a note) when it is not set:
//!
//! - `ROLIDSCP_BENCH_FLAT`: one folder with many files (`gen-tree.py flat`)
//! - `ROLIDSCP_BENCH_TREE`: many files in folders (`gen-tree.py tree`)
//! - `ROLIDSCP_BENCH_DEEP`: deep folders (`gen-tree.py deep`)
//! - `ROLIDSCP_BENCH_ODD`: unusual names and special files (`gen-tree.py odd`)
//! - `ROLIDSCP_BENCH_WORK`: an empty folder on the same disk for copies
//!
//! SFTP runs against a throwaway OpenSSH server on 127.0.0.1 (the same one the
//! SSH integration tests use), which sees the same disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::Vfs;

use crate::pane::{Pane, SortKey};

#[path = "../../rolid-ssh/tests/sshd/mod.rs"]
#[allow(clippy::all, unused)]
mod sshd;

fn env_dir(var: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            eprintln!("SKIP: {var} not set");
            None
        }
    }
}

/// One measured value (stdout and, with ROLIDSCP_BENCH_OUT, that file).
fn report(name: &str, value: f64, unit: &str) {
    let line = format!("BENCH\t{name}\t{value:.3}\t{unit}");
    println!("{line}");
    if let Some(out) = std::env::var_os("ROLIDSCP_BENCH_OUT") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(out) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// A value from /proc/self/status in kB (VmRSS, VmHWM).
fn status_kb(key: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(|v| v.trim().trim_end_matches(" kB").parse().unwrap_or(0)))
        .unwrap_or(0)
}

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0)
}

/// The program's own background runtime (with its thread stack size).
fn rt() -> tokio::runtime::Runtime {
    crate::runtime()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Time `f` once.
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

/// A throwaway OpenSSH server and an SFTP session to it.
async fn sftp(name: &str) -> Option<(sshd::Sshd, Arc<dyn Vfs>, rolid_ssh::SshConnection)> {
    let srv = sshd::Sshd::start(name)?;
    let conn = rolid_ssh::SshConnection::connect(&srv.params(srv.port)).await.expect("connect");
    let fs: Arc<dyn Vfs> = Arc::new(conn.sftp().await.expect("sftp"));
    Some((srv, fs, conn))
}

/// Listing one big folder locally: time, memory per item, the panel's sorting.
#[test]
#[ignore]
fn bench_list_local() {
    let Some(dir) = env_dir("ROLIDSCP_BENCH_FLAT") else { return };
    let rt = rt();
    let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let rss0 = status_kb("VmRSS");
    let (entries, t_list) = timed(|| rt.block_on(fs.list(&s(&dir))).unwrap());
    let n = entries.len();
    report("local.list.items", n as f64, "items");
    report("local.list.read_ms", ms(t_list), "ms");
    let mut pane = Pane::new(fs.clone(), s(dir.parent().unwrap()));
    let (_, t_set) = timed(|| pane.set_listing(s(&dir), entries, None));
    report("local.list.install_ms", ms(t_set), "ms");
    report("local.list.total_ms", ms(t_list + t_set), "ms");
    let rss1 = status_kb("VmRSS");
    report("local.list.rss_extra_mb", (rss1.saturating_sub(rss0)) as f64 / 1024.0, "MB");
    report("local.list.bytes_per_item", (rss1.saturating_sub(rss0)) as f64 * 1024.0 / n.max(1) as f64, "B");
    pane_ops("local", &mut pane);
}

/// What the panel does with a big listing on the UI thread: sorting, the
/// quick filter, the footer (every frame), marking, a background refresh.
fn pane_ops(prefix: &str, pane: &mut Pane) {
    for (key, name) in [(SortKey::Size, "size"), (SortKey::Modified, "date"), (SortKey::Ext, "ext"), (SortKey::Name, "name")] {
        let (_, t) = timed(|| pane.set_sort(key));
        report(&format!("{prefix}.sort.{name}_ms"), ms(t), "ms");
    }
    let (_, t) = timed(|| pane.set_sort(SortKey::Name)); // descending
    report(&format!("{prefix}.sort.name_desc_ms"), ms(t), "ms");
    pane.set_sort(SortKey::Name);
    let mut worst = Duration::ZERO;
    for q in ["f", "f1", "f12", "f123", "f1234", "f12345"] {
        pane.filter = q.into();
        let (_, t) = timed(|| pane.rebuild_view());
        report(&format!("{prefix}.filter.{q}_ms"), ms(t), "ms");
        worst = worst.max(t);
    }
    report(&format!("{prefix}.filter.worst_ms"), ms(worst), "ms");
    pane.filter.clear();
    let (_, t) = timed(|| pane.rebuild_view());
    report(&format!("{prefix}.filter.clear_ms"), ms(t), "ms");
    // The footer as the window makes it: the first frame of the folder, the
    // frame after Ctrl+A, the frame after one more mark (Insert).
    let (_, t) = timed(|| pane.footer_text());
    report(&format!("{prefix}.footer_ms"), ms(t), "ms");
    let (_, t) = timed(|| pane.mark_all());
    report(&format!("{prefix}.mark_all_ms"), ms(t), "ms");
    let (_, t) = timed(|| pane.footer_text());
    report(&format!("{prefix}.footer_marked_ms"), ms(t), "ms");
    let (_, t) = timed(|| {
        pane.toggle_mark_current();
        pane.footer_text()
    });
    report(&format!("{prefix}.footer_one_more_ms"), ms(t), "ms");
    // Everything counted again (the footer when folder sizes are computed).
    let (_, t) = timed(|| crate::pane::selection_summary(&pane.entries, &pane.marked, &pane.dir_sizes));
    report(&format!("{prefix}.footer_recount_ms"), ms(t), "ms");
    let (_, t) = timed(|| pane.targets());
    report(&format!("{prefix}.targets_ms"), ms(t), "ms");
    pane.marked.clear();
    // A background refresh: nothing changed, then one file more. The new
    // listing is fingerprinted and sorted off the UI thread (as the program
    // does it); what is timed is what the window waits for.
    let sort = Some((pane.sort, pane.sort_asc));
    let (same, t) = timed(|| crate::pane::Prepared::new(pane.entries.clone(), sort));
    report(&format!("{prefix}.refresh_prepare_ms"), ms(t), "ms");
    let (_, t) = timed(|| pane.update_prepared(same));
    report(&format!("{prefix}.refresh_same_ms"), ms(t), "ms");
    let mut more = pane.entries.clone();
    let mut extra = more[more.len() / 2].clone();
    extra.name = "zz-new-file".into();
    more.push(extra);
    let more = crate::pane::Prepared::new(more, sort);
    let (_, t) = timed(|| pane.update_prepared(more));
    report(&format!("{prefix}.refresh_changed_ms"), ms(t), "ms");
    for (rows, name) in [(1isize, "down"), (30, "page")] {
        let (_, t) = timed(|| pane.move_cursor(rows));
        report(&format!("{prefix}.cursor_{name}_ms"), ms(t), "ms");
    }
    let (_, t) = timed(|| pane.cursor_end());
    report(&format!("{prefix}.cursor_end_ms"), ms(t), "ms");
}

/// The same folder over SFTP.
#[test]
#[ignore]
fn bench_list_sftp() {
    let Some(dir) = env_dir("ROLIDSCP_BENCH_FLAT") else { return };
    let rt = rt();
    let Some((_srv, fs, _conn)) = rt.block_on(sftp("bench-list")) else { return };
    let rss0 = status_kb("VmRSS");
    let (entries, t_list) = timed(|| rt.block_on(fs.list(&s(&dir))).unwrap());
    let n = entries.len();
    report("sftp.list.items", n as f64, "items");
    report("sftp.list.read_ms", ms(t_list), "ms");
    let mut pane = Pane::new(fs.clone(), s(dir.parent().unwrap()));
    let (_, t_set) = timed(|| pane.set_listing(s(&dir), entries, None));
    report("sftp.list.install_ms", ms(t_set), "ms");
    report("sftp.list.total_ms", ms(t_list + t_set), "ms");
    report("sftp.list.rss_extra_mb", (status_kb("VmRSS").saturating_sub(rss0)) as f64 / 1024.0, "MB");
    // Leaving the folder while it loads: how soon does the listing stop?
    let path = s(&dir);
    let fs2 = fs.clone();
    let task = rt.spawn(async move { fs2.list(&path).await.map(|v| v.len()) });
    std::thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    task.abort();
    let _ = rt.block_on(task);
    report("sftp.list.abort_ms", ms(t.elapsed()), "ms");
    // The session still works afterwards.
    let (_, t) = timed(|| rt.block_on(fs.stat(&s(&dir))).unwrap());
    report("sftp.list.stat_after_abort_ms", ms(t), "ms");
}

/// Folder sizes of a tree: locally, over SFTP (walking) and with `du` on the server.
#[test]
#[ignore]
fn bench_dirsize() {
    let Some(dir) = env_dir("ROLIDSCP_BENCH_TREE") else { return };
    let stop = AtomicBool::new(false);
    let (r, t) = timed(|| crate::dirsize::local(&dir, &stop, &mut |_| {}).unwrap());
    report("size.local.bytes", r.bytes as f64, "B");
    report("size.local_ms", ms(t), "ms");
    let rt = rt();
    let Some((_srv, fs, conn)) = rt.block_on(sftp("bench-size")) else { return };
    let (r, t) = timed(|| rt.block_on(crate::dirsize::walk(fs.as_ref(), &s(&dir), &stop, &mut |_| {})).unwrap());
    report("size.sftp_walk.bytes", r.bytes as f64, "B");
    report("size.sftp_walk_ms", ms(t), "ms");
    let (r, t) = timed(|| rt.block_on(crate::dirsize::remote_du(&conn, &s(&dir), &stop)));
    match r {
        Ok(Some(r)) => report("size.server_du.bytes", r.bytes as f64, "B"),
        other => println!("size.server_du: {other:?}"),
    }
    report("size.server_du_ms", ms(t), "ms");
}

/// Search by name in a tree: time to the first hit and to the end.
#[test]
#[ignore]
fn bench_search() {
    let Some(dir) = env_dir("ROLIDSCP_BENCH_TREE") else { return };
    // Matches one file in every 100 000 ("f?00000*" style: f0100000, f0200000…) plus f0000000.
    let q = rolid_search::Query { root: s(&dir), name: "f??00000".into(), max_results: 1_000_000, ..Default::default() };
    let run = |label: &str, f: &dyn Fn(&rolid_search::Sink) -> Result<rolid_search::Outcome, rolid_i18n::rolid_text::Text>| {
        let start = Instant::now();
        let first = std::sync::Mutex::new(None::<Duration>);
        let emit = |_h: rolid_search::Hit| {
            first.lock().unwrap().get_or_insert(start.elapsed());
        };
        let cancel = AtomicBool::new(false);
        let sink = rolid_search::Sink::new(&emit, q.max_results, &cancel);
        let out = f(&sink).unwrap();
        report(&format!("search.{label}.hits"), out.hits as f64, "hits");
        report(&format!("search.{label}.first_ms"), first.lock().unwrap().map_or(f64::NAN, ms), "ms");
        report(&format!("search.{label}.total_ms"), ms(start.elapsed()), "ms");
    };
    run("local", &|sink| rolid_search::search_local(&q, sink));
    let rt = rt();
    let Some((_srv, fs, conn)) = rt.block_on(sftp("bench-search")) else { return };
    run("server_find", &|sink| rt.block_on(rolid_search::remote::search_remote(&conn, fs.as_ref(), &q, sink)));
    run("sftp_walk", &|sink| rt.block_on(rolid_search::search_vfs(fs.as_ref(), &q, sink)));
}

#[allow(clippy::too_many_arguments)]
fn copy_job(rt: &tokio::runtime::Runtime, src: &dyn Vfs, dst: &dyn Vfs, items: &[(String, String)], dst_dir: &str, conflict: Conflict, cancel: Arc<AtomicBool>) -> (Result<(), String>, u64, Duration) {
    let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let d2 = done.clone();
    let cb = move |p: &Progress| d2.store(p.files_done, Ordering::Relaxed);
    let t = Instant::now();
    let r = rt.block_on(async {
        let mut job = CopyJob::new(src, dst, cancel, &cb).with_conflict(conflict);
        job.copy_items_retrying(items, dst_dir, 0, |_| async { false }).await
    });
    (r.map_err(|e| rolid_i18n::t(&e.text())), done.load(Ordering::Relaxed), t.elapsed())
}

/// Compare two trees file by file: (missing in `b`, different, extra in `b`).
fn compare_trees(a: &Path, b: &Path) -> (u64, u64, u64) {
    fn files(root: &Path) -> std::collections::BTreeSet<PathBuf> {
        let mut out = std::collections::BTreeSet::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if e.file_type().unwrap().is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.strip_prefix(root).unwrap().to_path_buf());
                }
            }
        }
        out
    }
    let (fa, fb) = (files(a), files(b));
    let missing = fa.difference(&fb).count() as u64;
    let extra = fb.difference(&fa).count() as u64;
    let different = fa.intersection(&fb).filter(|r| std::fs::read(a.join(r)).ok() != std::fs::read(b.join(r)).ok()).count() as u64;
    (missing, different, extra)
}

fn report_compare(prefix: &str, a: &Path, b: &Path) {
    let (m, d, e) = compare_trees(a, b);
    report(&format!("{prefix}.missing"), m as f64, "files");
    report(&format!("{prefix}.different"), d as f64, "files");
    report(&format!("{prefix}.extra"), e as f64, "files");
}

/// Copy a tree to the server and back, cancel a copy and finish it with
/// "Skip existing", delete the copies – locally and on the server. Every copy
/// is compared with the source byte by byte.
#[test]
#[ignore]
fn bench_copy_delete() {
    let (Some(tree), Some(work)) = (env_dir("ROLIDSCP_BENCH_TREE"), env_dir("ROLIDSCP_BENCH_WORK")) else { return };
    let rt = rt();
    let Some((_srv, sftp, _conn)) = rt.block_on(sftp("bench-copy")) else { return };
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let (up, back, again) = (work.join("up"), work.join("back"), work.join("again"));
    for d in [&up, &back, &again] {
        std::fs::create_dir_all(d).unwrap();
    }
    let name = tree.file_name().unwrap().to_string_lossy().into_owned();
    let items = vec![(s(tree.parent().unwrap()), name.clone())];
    let fd0 = open_fds();
    let del = |fs: &dyn Vfs, path: String| {
        let t = Instant::now();
        if let Err(e) = rt.block_on(crate::app::remove_recursive(fs, &path)) {
            panic!("deleting {path}: {}", rolid_i18n::t(&e.text()));
        }
        t.elapsed()
    };

    // Local → server → local.
    let (r, files, t) = copy_job(&rt, local.as_ref(), sftp.as_ref(), &items, &s(&up), Conflict::Skip, Arc::new(AtomicBool::new(false)));
    r.unwrap();
    report("copy.up.files", files as f64, "files");
    report("copy.up_ms", ms(t), "ms");
    let items_back = vec![(s(&up), name.clone())];
    let (r, files, t) = copy_job(&rt, sftp.as_ref(), local.as_ref(), &items_back, &s(&back), Conflict::Skip, Arc::new(AtomicBool::new(false)));
    r.unwrap();
    report("copy.down.files", files as f64, "files");
    report("copy.down_ms", ms(t), "ms");
    report_compare("copy.roundtrip", &tree, &back.join(&name));
    report("delete.local_ms", ms(del(local.as_ref(), s(&back.join(&name)))), "ms");
    report("delete.sftp_ms", ms(del(sftp.as_ref(), s(&up.join(&name)))), "ms");

    // Cancel after 3 s, then finish with "Skip existing".
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let canceled_at = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let ca = canceled_at.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        *ca.lock().unwrap() = Some(Instant::now());
        c2.store(true, Ordering::Relaxed);
    });
    let (r, files_before, _) = copy_job(&rt, local.as_ref(), sftp.as_ref(), &items, &s(&again), Conflict::Skip, cancel);
    let stop_ms = canceled_at.lock().unwrap().map_or(f64::NAN, |t| ms(t.elapsed()));
    stopper.join().unwrap();
    report("copy.cancel_stop_ms", stop_ms, "ms");
    report("copy.cancel.canceled", r.is_err() as u8 as f64, "bool");
    report("copy.cancel.files_before", files_before as f64, "files");
    let (r, files_after, t) = copy_job(&rt, local.as_ref(), sftp.as_ref(), &items, &s(&again), Conflict::Skip, Arc::new(AtomicBool::new(false)));
    r.unwrap();
    report("copy.resume.files", files_after as f64, "files");
    report("copy.resume_ms", ms(t), "ms");
    report_compare("copy.resume", &tree, &again.join(&name));
    report("copy.fds_extra", open_fds().saturating_sub(fd0) as f64, "fds");
    del(sftp.as_ref(), s(&again.join(&name)));
}

/// Memory after repeated listings and copies: does it come back?
#[test]
#[ignore]
fn bench_leak() {
    let Some(dir) = env_dir("ROLIDSCP_BENCH_FLAT") else { return };
    let rt = rt();
    let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let once = || {
        let entries = rt.block_on(fs.list(&s(&dir))).unwrap();
        let mut pane = Pane::new(fs.clone(), "/".into());
        pane.set_listing(s(&dir), entries, None);
        pane.set_sort(SortKey::Size);
        pane.filter = "f1".into();
        pane.rebuild_view();
        drop(pane);
    };
    // The allocator keeps freed memory for later: the second listing lands in
    // another arena of the allocator and RSS settles there (measured: 83 MB
    // after one listing, 143 MB after the second and the same after 30). A
    // leak grows with every round, so the base is taken after two rounds.
    once();
    report("leak.rss_after_1_mb", status_kb("VmRSS") as f64 / 1024.0, "MB");
    once();
    let (rss1, fd1) = (status_kb("VmRSS"), open_fds());
    for _ in 0..10 {
        once();
    }
    let (rss10, fd10) = (status_kb("VmRSS"), open_fds());
    report("leak.rss_after_2_mb", rss1 as f64 / 1024.0, "MB");
    report("leak.rss_after_12_mb", rss10 as f64 / 1024.0, "MB");
    report("leak.rss_growth_pct", (rss10 as f64 - rss1 as f64) * 100.0 / rss1.max(1) as f64, "%");
    report("leak.fd_growth", fd10 as f64 - fd1 as f64, "fds");
}

/// Deep folders and a path close to PATH_MAX: copy, size, search, delete.
#[test]
#[ignore]
fn bench_deep() {
    let (Some(deep), Some(work)) = (env_dir("ROLIDSCP_BENCH_DEEP"), env_dir("ROLIDSCP_BENCH_WORK")) else { return };
    let local: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
    let dst = work.join("deep-copy");
    std::fs::create_dir_all(&dst).unwrap();
    let items: Vec<(String, String)> = ["levels", "long", "loop"].iter().map(|n| (s(&deep), n.to_string())).collect();
    let rt = rt();
    let (r, files, t) = copy_job(&rt, local.as_ref(), local.as_ref(), &items, &s(&dst), Conflict::Skip, Arc::new(AtomicBool::new(false)));
    report("deep.copy.files", files as f64, "files");
    report("deep.copy_ms", ms(t), "ms");
    println!("deep.copy.result: {:?}", r);
    let stop = AtomicBool::new(false);
    let (sz, t) = timed(|| crate::dirsize::local(&deep, &stop, &mut |_| {}));
    report("deep.size.bytes", sz.map_or(f64::NAN, |d| d.bytes as f64), "B");
    report("deep.size_ms", ms(t), "ms");
    let q = rolid_search::Query { root: s(&deep), name: "bottom.txt".into(), ..Default::default() };
    let cancel = AtomicBool::new(false);
    let emit = |h: rolid_search::Hit| println!("deep.search.hit: {} bytes of path", h.dir.len());
    let sink = rolid_search::Sink::new(&emit, 10, &cancel);
    let (out, t) = timed(|| rolid_search::search_local(&q, &sink).unwrap());
    report("deep.search.hits", out.hits as f64, "hits");
    report("deep.search_ms", ms(t), "ms");
    let (r, t) = timed(|| rt.block_on(crate::app::remove_recursive(local.as_ref(), &s(&dst))));
    report("deep.delete_ms", ms(t), "ms");
    println!("deep.delete.result: {:?}", r.as_ref().map_err(|e| rolid_i18n::t(&e.text())));
}

