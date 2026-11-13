//! Manual smoke test: `cargo run -p rolid-ssh --example smoke -- user@host[:port] KNOWN_HOSTS WORKDIR`
//! Uses a separate known_hosts file; accepts an unknown host key only for that file.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rolid_core::copy::CopyJob;
use rolid_core::Vfs;
use rolid_ssh::{ConnectParams, SshConnection, SshError};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (user, rest) = args[1].split_once('@').unwrap();
    let (host, port) = rest
        .split_once(':')
        .map(|(h, p)| (h, p.parse().unwrap()))
        .unwrap_or((rest, 22));
    let mut p = ConnectParams {
        host: host.into(),
        port,
        user: user.into(),
        known_hosts: Some(args[2].clone().into()),
        key_path: args.get(4).map(Into::into),
        ..Default::default()
    };
    let conn = match SshConnection::connect(&p).await {
        Err(SshError::UnknownHostKey {
            fingerprint,
            algorithm,
            ..
        }) => {
            println!(
                "unknown host key {algorithm} {fingerprint} -> accepting into test known_hosts"
            );
            p.accept_fingerprint = Some(fingerprint);
            SshConnection::connect(&p)
                .await
                .expect("connect after accept")
        }
        r => r.expect("connect"),
    };
    println!("connected as {}", conn.label);
    // Second connect must now pass strict check without accept.
    p.accept_fingerprint = None;
    SshConnection::connect(&p).await.expect("strict reconnect");
    println!("strict reconnect ok");

    let sftp = conn.sftp().await.expect("sftp");
    let sftp2 = conn
        .sftp()
        .await
        .expect("second sftp channel on same connection");
    let home = sftp.home().await.unwrap();
    println!(
        "home = {home}, entries = {}",
        sftp.list(&home).await.unwrap().len()
    );

    let work = &args[3];
    let local = rolid_local::LocalFs;
    let src_dir = format!("{work}/src");
    std::fs::create_dir_all(format!("{src_dir}/tree/sub")).unwrap();
    let big: Vec<u8> = (0..20_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(format!("{src_dir}/tree/big.bin"), &big).unwrap();
    std::fs::write(format!("{src_dir}/tree/sub/small.txt"), b"hello").unwrap();
    let up_dir = format!("{work}/up");
    std::fs::create_dir_all(&up_dir).unwrap();

    let cb = |_: &rolid_core::copy::Progress| {};
    let t = std::time::Instant::now();
    // local -> remote (the "remote" is the same machine, so paths are shared)
    CopyJob::new(&local, &sftp, Arc::new(AtomicBool::new(false)), &cb)
        .copy_entry(&src_dir, "tree", &up_dir)
        .await
        .expect("upload");
    println!("upload 20 MB in {:?}", t.elapsed());
    let down_dir = format!("{work}/down");
    std::fs::create_dir_all(&down_dir).unwrap();
    let t = std::time::Instant::now();
    CopyJob::new(&sftp2, &local, Arc::new(AtomicBool::new(false)), &cb)
        .copy_entry(&up_dir, "tree", &down_dir)
        .await
        .expect("download");
    println!("download 20 MB in {:?}", t.elapsed());
    assert_eq!(
        std::fs::read(format!("{down_dir}/tree/big.bin")).unwrap(),
        big
    );
    assert_eq!(
        std::fs::read(format!("{down_dir}/tree/sub/small.txt")).unwrap(),
        b"hello"
    );
    println!("roundtrip OK");

    // Resume over SFTP: fake an interrupted upload (.filepart with the first 7 MB).
    let resume_dir = format!("{work}/resume");
    std::fs::create_dir_all(&resume_dir).unwrap();
    std::fs::write(format!("{resume_dir}/big.bin.filepart"), &big[..7_000_000]).unwrap();
    let t = std::time::Instant::now();
    let mut job = CopyJob::new(&local, &sftp, Arc::new(AtomicBool::new(false)), &cb)
        .with_conflict(rolid_core::copy::Conflict::Resume);
    job.copy_entry(&format!("{src_dir}/tree"), "big.bin", &resume_dir)
        .await
        .expect("resume");
    let p = job.progress().clone();
    println!(
        "resume: resumed {} B, sent {} B in {:?}",
        p.bytes_resumed,
        p.bytes_done,
        t.elapsed()
    );
    assert_eq!(p.bytes_resumed, 7_000_000);
    assert_eq!(std::fs::read(format!("{resume_dir}/big.bin")).unwrap(), big);
    assert!(!std::path::Path::new(&format!("{resume_dir}/big.bin.filepart")).exists());
    // Skip policy over SFTP
    let mut job = CopyJob::new(&local, &sftp, Arc::new(AtomicBool::new(false)), &cb)
        .with_conflict(rolid_core::copy::Conflict::Skip);
    job.copy_entry(&format!("{src_dir}/tree"), "big.bin", &resume_dir)
        .await
        .unwrap();
    assert_eq!(job.progress().files_skipped, 1);
    // mtime preserved over SFTP
    let sm = std::fs::metadata(format!("{src_dir}/tree/big.bin"))
        .unwrap()
        .modified()
        .unwrap();
    let dm = std::fs::metadata(format!("{resume_dir}/big.bin"))
        .unwrap()
        .modified()
        .unwrap();
    let secs =
        |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(secs(sm), secs(dm));
    println!("resume/skip/mtime over SFTP OK");

    // Remote search: server-side find/grep vs. SFTP walk must agree.
    use rolid_search::{remote, search_vfs, Hit, Query, Sink};
    std::fs::write(format!("{src_dir}/tree/sub/note's file.txt"), "Searched TEXT here\n").unwrap();
    let run = |q: Query, walk: bool| {
        let sftp = &sftp;
        let conn = &conn;
        async move {
            let hits = std::sync::Mutex::new(Vec::<Hit>::new());
            let emit = |h: Hit| hits.lock().unwrap().push(h);
            let cancel = AtomicBool::new(false);
            let sink = Sink::new(&emit, 1000, &cancel);
            let o = if walk {
                search_vfs(sftp, &q, &sink).await.unwrap()
            } else {
                remote::search_remote(conn, sftp, &q, &sink).await.unwrap()
            };
            let mut v: Vec<String> = hits.into_inner().unwrap().into_iter().map(|h| h.rel).collect();
            v.sort();
            (o.method, v)
        }
    };
    let root = format!("{src_dir}/tree");
    for q in [
        Query { root: root.clone(), name: "*.txt".into(), ..Default::default() },
        Query { root: root.clone(), content: "searched text".into(), ..Default::default() },
        Query { root: root.clone(), min_size: Some(1 << 20), ..Default::default() },
    ] {
        let (m1, a) = run(q.clone(), false).await;
        let (m2, b) = run(q.clone(), true).await;
        println!("search {:?}/{:?}: {:?}", m1, m2, a);
        assert_eq!(m1, rolid_search::Method::RemoteShell);
        assert_eq!(a, b, "shell vs walk differ for {q:?}");
        assert!(!a.is_empty());
    }
    println!("remote search OK");
}
