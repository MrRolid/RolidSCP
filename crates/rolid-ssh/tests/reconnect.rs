//! Connection loss and automatic reconnection against a real OpenSSH server, with
//! a TCP proxy in between that cuts the connection at the worst moments:
//! (a) in the middle of a directory listing, (b) in the middle of a large
//! transfer (both directions), (c) in the middle of an F4 upload. The result must
//! be byte for byte identical. Also: keepalive notices a silent network.

mod sshd;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::copy::{CopyJob, Progress, MAX_AUTO_RETRIES};
use rolid_core::Vfs;
use rolid_edit::transfer::{upload_replace, Ctl};
use rolid_local::LocalFs;
use rolid_ssh::link::drive_reconnect;
use rolid_ssh::{Attempt, ConnectParams, Link, LinkState, RemoteFs, SshConnection};

/// What the app does every frame: notice a closed connection, start reconnecting.
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
    params.keepalive = Some((Duration::from_secs(1), 3));
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

async fn wait_up(link: &Link, secs: u64) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !link.state().is_up() {
        assert!(Instant::now() < end, "not reconnected: {:?}", link.state());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listing_interrupted_then_reconnected() {
    let Some(e) = env("listing").await else { return };
    let dir = e.srv.data_dir("many");
    for i in 0..3000 {
        std::fs::write(dir.join(format!("file-with-a-long-name-for-a-bigger-listing-{i:05}.txt")), b"x").unwrap();
    }
    let path = dir.to_string_lossy().into_owned();
    assert_eq!(e.fs.list(&path).await.unwrap().len(), 3000);
    let gen = e.link.generation();

    // Cut in the middle of the listing (and keep the server unreachable for now).
    e.proxy.refuse.store(true, Ordering::SeqCst);
    e.proxy.cut_after(60_000);
    let err = e.fs.list(&path).await.unwrap_err();
    assert!(err.is_connection_lost(), "{err}");
    assert!(!e.link.state().is_up());

    // While disconnected, operations fail at once with a clear message (no hang).
    let t = Instant::now();
    let err = e.fs.stat(&path).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_millis(200), "{:?}", t.elapsed());
    assert!(err.to_string().contains("is interrupted"), "{err}");
    assert!(err.is_connection_lost());
    // The UI only reads the state: never waits.
    let t = Instant::now();
    for _ in 0..1000 {
        let _ = e.link.state().text();
        e.link.check();
    }
    assert!(t.elapsed() < Duration::from_millis(100), "state reads are cheap: {:?}", t.elapsed());

    // Network back: reconnects by itself (backoff 1, 2, 4 s…).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    e.proxy.refuse.store(false, Ordering::SeqCst);
    wait_up(&e.link, 20).await;
    assert!(e.link.generation() > gen);
    let list = e.fs.list(&path).await.unwrap();
    assert_eq!(list.len(), 3000, "the same panel works on the new connection");
}

async fn copy_with_cut(e: &Env, src: &dyn Vfs, src_dir: &str, dst: &dyn Vfs, dst_dir: &str, name: &str, cut_at: u64) -> u32 {
    let cut = AtomicBool::new(false);
    let proxy = e.proxy.clone();
    let cb = move |p: &Progress| {
        if p.bytes_done >= cut_at && !cut.swap(true, Ordering::SeqCst) {
            proxy.cut(); // network failure in the middle of the file
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut job = CopyJob::new(src, dst, cancel.clone(), &cb);
    let mut waits = 0;
    let link = e.link.clone();
    job.copy_items_retrying(&[(src_dir.to_string(), name.to_string())], dst_dir, MAX_AUTO_RETRIES, |_| {
        waits += 1;
        let (link, cancel) = (link.clone(), cancel.clone());
        async move { link.wait_up(&cancel).await }
    })
    .await
    .expect("copy finished after reconnection");
    waits
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b_large_transfer_continues_after_reconnection() {
    let Some(e) = env("transfer").await else { return };
    let remote = e.srv.data_dir("remote");
    let local = e.srv.data_dir("local");
    let big = data(48 * 1024 * 1024, 7);
    std::fs::write(remote.join("big.bin"), &big).unwrap();
    let (r, l) = (remote.to_string_lossy().into_owned(), local.to_string_lossy().into_owned());

    // Download, cut at 20 MB.
    let gen = e.link.generation();
    let waits = copy_with_cut(&e, e.fs.as_ref(), &r, &LocalFs, &l, "big.bin", 20 << 20).await;
    assert_eq!(waits, 1, "one interruption");
    assert!(e.link.generation() > gen, "reconnected");
    assert!(std::fs::read(local.join("big.bin")).unwrap() == big, "download byte for byte identical");
    assert!(!local.join("big.bin.filepart").exists());

    // Upload, cut at 30 MB.
    std::fs::remove_file(remote.join("big.bin")).unwrap();
    let up = e.srv.data_dir("up");
    let u = up.to_string_lossy().into_owned();
    let gen = e.link.generation();
    let waits = copy_with_cut(&e, &LocalFs, &l, e.fs.as_ref(), &u, "big.bin", 30 << 20).await;
    assert_eq!(waits, 1);
    assert!(e.link.generation() > gen);
    assert!(std::fs::read(up.join("big.bin")).unwrap() == big, "upload byte for byte identical");
    assert!(!up.join("big.bin.filepart").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c_f4_upload_interrupted_then_repeated() {
    let Some(e) = env("f4").await else { return };
    let remote = e.srv.data_dir("remote");
    let local = e.srv.data_dir("local");
    let original = b"<?php echo 'original';\n".to_vec();
    std::fs::write(remote.join("index.php"), &original).unwrap();
    let edited = data(24 * 1024 * 1024, 3);
    std::fs::write(local.join("index.php"), &edited).unwrap();
    let dst = remote.join("index.php").to_string_lossy().into_owned();
    let src = local.join("index.php").to_string_lossy().into_owned();

    let cut = AtomicBool::new(false);
    let proxy = e.proxy.clone();
    let progress = move |n: u64| {
        if n >= 8 << 20 && !cut.swap(true, Ordering::SeqCst) {
            proxy.cut();
        }
    };
    let ctl = Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &progress };
    let err = upload_replace(&LocalFs, &src, e.fs.as_ref(), &dst, Some(0o640), &ctl).await.unwrap_err();
    assert!(err.is_connection_lost(), "{err}");
    assert_eq!(std::fs::read(remote.join("index.php")).unwrap(), original, "original untouched, never truncated");

    // After reconnection the app repeats the upload (conflict check first: unchanged).
    wait_up(&e.link, 20).await;
    let st = e.fs.stat(&dst).await.unwrap();
    assert_eq!(st.size, original.len() as u64, "no conflict: remote still the original");
    static NOOP: fn(u64) = |_| {};
    let ctl = Ctl { limit: None, cancel: Arc::new(AtomicBool::new(false)), progress: &NOOP };
    upload_replace(&LocalFs, &src, e.fs.as_ref(), &dst, Some(0o640), &ctl).await.unwrap();
    assert!(std::fs::read(remote.join("index.php")).unwrap() == edited, "byte for byte identical");
    let mut names: Vec<String> =
        std::fs::read_dir(&remote).unwrap().map(|d| d.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["index.php"], "no .filepart / .rolidscp-old left");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepalive_notices_a_silent_network() {
    let Some(e) = env("keepalive").await else { return };
    let home = e.fs.home().await.unwrap();
    // Packets stop flowing, nothing is closed (cable pulled, laptop asleep).
    e.proxy.frozen.store(true, Ordering::SeqCst);
    let t = Instant::now();
    while e.link.state().is_up() {
        assert!(t.elapsed() < Duration::from_secs(15), "keepalive did not notice");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // 1 s interval × 3 probes in the test (15 s × 3 in the app).
    assert!(t.elapsed() >= Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(matches!(e.link.state(), LinkState::Reconnecting { .. }));
    e.proxy.cut();
    e.proxy.frozen.store(false, Ordering::SeqCst);
    wait_up(&e.link, 20).await;
    assert_eq!(e.fs.home().await.unwrap(), home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_restart_is_survived() {
    let Some(mut e) = env("restart").await else { return };
    let home = e.fs.home().await.unwrap();
    e.srv.kill();
    let t = Instant::now();
    while e.link.state().is_up() {
        assert!(t.elapsed() < Duration::from_secs(10), "loss not noticed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    e.srv.restart();
    wait_up(&e.link, 30).await;
    assert_eq!(e.fs.home().await.unwrap(), home);
}

