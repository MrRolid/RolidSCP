//! A transfer that hears nothing from the server (roadmap 0.8.18, the flapping
//! line): the connection counts as dead after the stall limit, long before the
//! keepalive would notice, and the transfer continues on a new connection.
//! A server that is only slow (it answers pings) is not given up. An upload
//! knows how far the server confirmed it.

mod sshd;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::copy::{CopyJob, Progress};
use rolid_core::Vfs;
use rolid_local::LocalFs;
use rolid_ssh::link::drive_reconnect;
use rolid_ssh::{Attempt, ConnectParams, Link, RemoteFs, SshConnection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The stall limit of these tests (15 s in the app).
const STALL: Duration = Duration::from_secs(2);

fn supervise(link: Arc<Link>, params: ConnectParams) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
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

struct Env {
    srv: sshd::Sshd,
    proxy: sshd::Proxy,
    link: Arc<Link>,
    fs: Arc<RemoteFs>,
    _sup: tokio::task::JoinHandle<()>,
}

async fn env(name: &str) -> Option<Env> {
    let srv = sshd::Sshd::start(name)?;
    let proxy = sshd::Proxy::start(srv.port).await;
    srv.add_known_port(proxy.port);
    let mut params = srv.params(proxy.port);
    // The keepalive alone would need 3 minutes here.
    params.keepalive = Some((Duration::from_secs(60), 3));
    params.stall_limit = Some(STALL);
    let conn = SshConnection::connect(&params).await.expect("connect");
    let link = Link::new(conn);
    let fs = Arc::new(RemoteFs::new(link.clone()));
    let sup = supervise(link.clone(), params);
    Some(Env { srv, proxy, link, fs, _sup: sup })
}

fn data(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frozen_line_is_given_up_after_the_stall_limit_and_the_upload_continues() {
    let Some(e) = env("stall").await else { return };
    let local = e.srv.data_dir("local");
    let up = e.srv.data_dir("up");
    let big = data(24 * 1024 * 1024, 5);
    std::fs::write(local.join("big.bin"), &big).unwrap();

    // The line freezes (no packet goes either way, nothing is closed) after 8 MB.
    let frozen_at = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let (proxy, fa) = (e.proxy.clone(), frozen_at.clone());
    let once = AtomicBool::new(false);
    let cb = move |p: &Progress| {
        if p.bytes_done >= 8 << 20 && !once.swap(true, Ordering::SeqCst) {
            proxy.frozen.store(true, Ordering::SeqCst);
            *fa.lock().unwrap() = Some(Instant::now());
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut job = CopyJob::new(&LocalFs, e.fs.as_ref(), cancel.clone(), &cb);
    let gen = e.link.generation();
    let (link, proxy, fa) = (e.link.clone(), e.proxy.clone(), frozen_at.clone());
    let noticed = Arc::new(std::sync::Mutex::new(None::<Duration>));
    let n2 = noticed.clone();
    job.copy_items_retrying(&[(local.to_string_lossy().into_owned(), "big.bin".into())], &up.to_string_lossy(), 5, |_| {
        let since = fa.lock().unwrap().map(|t| t.elapsed());
        n2.lock().unwrap().get_or_insert(since.unwrap_or_default());
        // The line works again (the frozen connection stays dead).
        proxy.frozen.store(false, Ordering::SeqCst);
        proxy.cut();
        let (link, cancel) = (link.clone(), cancel.clone());
        async move { link.wait_up(&cancel).await }
    })
    .await
    .expect("the upload finished on a new connection");
    let noticed = noticed.lock().unwrap().expect("the transfer was interrupted");
    assert!(noticed >= STALL, "given up after {noticed:?}, before the limit");
    assert!(noticed < STALL * 4, "given up only after {noticed:?} (the keepalive would need 180 s)");
    assert!(e.link.generation() > gen, "a new connection");
    assert!(std::fs::read(up.join("big.bin")).unwrap() == big, "byte for byte identical");
    assert!(!up.join("big.bin.filepart").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_that_only_takes_long_is_not_given_up() {
    let Some(e) = env("slow").await else { return };
    let dir = e.srv.data_dir("slow");
    let body = data(24 * 1024 * 1024, 2); // more than is read ahead
    std::fs::write(dir.join("f.bin"), &body).unwrap();
    let path = dir.join("f.bin").to_string_lossy().into_owned();
    // Both SFTP sessions of the panel are open.
    e.fs.list(&dir.to_string_lossy()).await.unwrap();
    let mut r = e.fs.open_read_at(&path, 0).await.unwrap();
    let mut first = vec![0u8; 1000];
    r.read_exact(&mut first).await.unwrap();
    // The server's SFTP processes stop answering for three stall limits (a
    // busy disk), while the connection itself is fine: it answers pings.
    let workers = e.srv.sftp_workers();
    assert!(!workers.is_empty(), "no SFTP process found");
    let signal = |sig: &str| {
        for p in &workers {
            let _ = std::process::Command::new("kill").args([sig, &p.to_string()]).status();
        }
    };
    let gen = e.link.generation();
    signal("-STOP");
    let t = Instant::now();
    let w2 = workers.clone();
    let resume = std::thread::spawn(move || {
        std::thread::sleep(STALL * 3);
        for p in &w2 {
            let _ = std::process::Command::new("kill").args(["-CONT", &p.to_string()]).status();
        }
    });
    let mut rest = Vec::new();
    let read = r.read_to_end(&mut rest).await;
    let listed = e.fs.list(&dir.to_string_lossy()).await;
    resume.join().unwrap();
    signal("-CONT");
    read.expect("the slow answer came, the connection was kept");
    listed.expect("a listing waits for the slow server too");
    first.extend(rest);
    assert!(first == body);
    assert!(t.elapsed() >= STALL * 2, "{:?}", t.elapsed());
    assert_eq!(e.link.generation(), gen, "no reconnection");
    assert!(e.link.state().is_up());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_knows_how_far_the_server_confirmed_it() {
    let Some(e) = env("confirmed").await else { return };
    let dir = e.srv.data_dir("c");
    let path = dir.join("f.filepart").to_string_lossy().into_owned();
    assert_eq!(e.fs.write_confirmed(&path), None);
    let chunk = data(3 * 1024 * 1024 + 17, 9);
    let mut w = e.fs.open_write_at(&path, 0).await.unwrap();
    w.write_all(&chunk).await.unwrap();
    w.flush().await.unwrap();
    assert_eq!(e.fs.write_confirmed(&path), Some(chunk.len() as u64), "all confirmed after a flush");
    w.shutdown().await.unwrap();
    drop(w);
    assert_eq!(e.fs.write_confirmed(&path), Some(chunk.len() as u64), "kept after the writer is gone");

    // Continuing at an offset counts from there.
    let mut w = e.fs.open_write_at(&path, 1024 * 1024).await.unwrap();
    assert_eq!(e.fs.write_confirmed(&path), Some(1024 * 1024));
    w.write_all(&chunk[1024 * 1024..]).await.unwrap();
    w.shutdown().await.unwrap();
    assert_eq!(e.fs.write_confirmed(&path), Some(chunk.len() as u64));
    assert!(std::fs::read(dir.join("f.filepart")).unwrap() == chunk);
}
