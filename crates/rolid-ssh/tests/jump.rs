//! Connecting through a jump host (ProxyJump) against two real OpenSSH
//! servers: both keys are checked, each server signs in with its own
//! credentials, errors say which server they come from, and after the line to
//! the jump host is cut the connection comes back the same way.

mod sshd;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::Vfs;
use rolid_ssh::link::drive_reconnect;
use rolid_ssh::{Attempt, ConnectParams, Link, RemoteFs, SshConnection, SshError};

/// The jump host (behind a proxy that can cut the line) and the target.
struct Two {
    jump: sshd::Sshd,
    target: sshd::Sshd,
    proxy: sshd::Proxy,
}

async fn two(name: &str) -> Option<Two> {
    let jump = sshd::Sshd::start(&format!("{name}-jump"))?;
    let target = sshd::Sshd::start(&format!("{name}-target"))?;
    let proxy = sshd::Proxy::start(jump.port).await;
    jump.add_known_port(proxy.port);
    Some(Two { jump, target, proxy })
}

/// A list of known servers with the keys of the given servers.
fn known(dir: &Path, name: &str, of: &[&sshd::Sshd]) -> std::path::PathBuf {
    let p = dir.join(name);
    let text: String = of.iter().map(|s| std::fs::read_to_string(&s.known_hosts).unwrap()).collect();
    std::fs::write(&p, text).unwrap();
    p
}

fn params(t: &Two, own: &Path) -> ConnectParams {
    let mut jump = t.jump.params(t.proxy.port);
    jump.known_hosts = Some(own.to_path_buf());
    let mut target = t.target.params(t.target.port);
    target.known_hosts = Some(own.to_path_buf());
    target.via = Some(Box::new(jump));
    target
}

#[tokio::test]
async fn files_are_listed_through_the_jump_host() {
    let Some(t) = two("list").await else { return };
    let own = known(&t.target.dir, "both", &[&t.jump, &t.target]);
    let p = params(&t, &own);
    let conn = SshConnection::connect(&p).await.expect("connect through the jump host");
    assert!(conn.via().is_some());
    assert!(conn.peer.is_none());
    assert_eq!(conn.key, format!("{}@127.0.0.1:{} via {}@127.0.0.1:{}", t.target.user, t.target.port, t.jump.user, t.proxy.port));
    let dir = t.target.data_dir("files");
    std::fs::write(dir.join("hello.txt"), b"through").unwrap();
    let fs = conn.sftp().await.expect("sftp");
    let names: Vec<String> = fs.list(&dir.display().to_string()).await.unwrap().into_iter().map(|e| e.name).collect();
    assert!(names.contains(&"hello.txt".to_string()), "{names:?}");
    // A file of 3 MB up and down again, through the jump host.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let up = format!("{}/up.bin", dir.display());
    let mut w = fs.open_write(&up).await.unwrap();
    w.write_all(&data).await.unwrap();
    w.shutdown().await.unwrap();
    drop(w);
    assert_eq!(std::fs::read(dir.join("up.bin")).unwrap(), data);
    let mut r = fs.open_read(&up).await.unwrap();
    let mut back = Vec::new();
    r.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, data);
}

#[tokio::test]
async fn the_key_of_each_server_is_checked() {
    let Some(t) = two("keys").await else { return };
    // The jump host is not known: its key is asked about, by its address.
    let own = known(&t.target.dir, "target-only", &[&t.target]);
    match SshConnection::connect(&params(&t, &own)).await.map(|_| ()) {
        Err(SshError::UnknownHostKey { port, .. }) => assert_eq!(port, t.proxy.port),
        r => panic!("{r:?}"),
    }
    // The target is not known.
    let own = known(&t.target.dir, "jump-only", &[&t.jump]);
    match SshConnection::connect(&params(&t, &own)).await.map(|_| ()) {
        Err(SshError::UnknownHostKey { port, .. }) => assert_eq!(port, t.target.port),
        r => panic!("{r:?}"),
    }
    // Accepting the target's key writes only the target's line.
    let mut p = params(&t, &own);
    let fp = {
        let k = std::fs::read_to_string(&t.target.known_hosts).unwrap();
        let k = k.split_whitespace().skip(1).take(2).collect::<Vec<_>>().join(" ");
        russh::keys::PublicKey::from_openssh(&k).unwrap().fingerprint(russh::keys::HashAlg::Sha256).to_string()
    };
    p.accept_fingerprint = Some(fp);
    SshConnection::connect(&p).await.expect("connect after accepting the target's key");
    let text = std::fs::read_to_string(&own).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.lines().last().unwrap().starts_with(&format!("[127.0.0.1]:{} ", t.target.port)));
    // A changed key of the jump host is refused.
    let wrong = t.target.dir.join("wrong");
    let jump_key = std::fs::read_to_string(&t.target.known_hosts).unwrap().replace(&format!("]:{} ", t.target.port), &format!("]:{} ", t.proxy.port));
    std::fs::write(&wrong, format!("{jump_key}{}", std::fs::read_to_string(&t.target.known_hosts).unwrap())).unwrap();
    match SshConnection::connect(&params(&t, &wrong)).await.map(|_| ()) {
        Err(SshError::HostKeyChanged { port, .. }) => assert_eq!(port, t.proxy.port),
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn errors_say_which_server() {
    let Some(t) = two("errors").await else { return };
    let own = known(&t.target.dir, "both", &[&t.jump, &t.target]);
    // The jump host refuses the key: a login error of the jump host.
    let mut p = params(&t, &own);
    p.via.as_mut().unwrap().key_path = Some(t.jump.dir.join("host_key"));
    match SshConnection::connect(&p).await.map(|_| ()) {
        Err(e @ SshError::JumpAuthFailed { .. }) => assert!(e.to_string().contains("jump host"), "{e}"),
        r => panic!("{r:?}"),
    }
    // The target signs in with its own key, not the jump host's.
    let mut p = params(&t, &own);
    p.key_path = Some(t.jump.dir.join("host_key"));
    assert!(matches!(SshConnection::connect(&p).await.map(|_| ()), Err(SshError::AuthFailed(_))));
    // Nothing listens at the target's address: the jump host says so.
    let mut p = params(&t, &own);
    p.port = 1;
    match SshConnection::connect(&p).await.map(|_| ()) {
        Err(SshError::Other(m)) => assert!(m.to_string().contains("could not connect to 127.0.0.1:1"), "{m}"),
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn a_cut_line_to_the_jump_host_reconnects_the_same_way() {
    let Some(t) = two("cut").await else { return };
    let own = known(&t.target.dir, "both", &[&t.jump, &t.target]);
    let mut p = params(&t, &own);
    p.keepalive = Some((Duration::from_secs(1), 3));
    p.via.as_mut().unwrap().keepalive = Some((Duration::from_secs(1), 3));
    let conn = SshConnection::connect(&p).await.expect("connect");
    let link = Link::new(conn);
    let fs = Arc::new(RemoteFs::new(link.clone()));
    let dir = t.target.data_dir("files");
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    assert_eq!(fs.list(&dir.display().to_string()).await.unwrap().len(), 1);
    let pp = p.clone();
    let sup = {
        let link = link.clone();
        tokio::spawn(async move {
            loop {
                link.check();
                if link.claim_driver() {
                    let p = pp.clone();
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
    };
    t.proxy.cut();
    // The target notices that its jump host is gone.
    let end = Instant::now() + Duration::from_secs(20);
    while link.state().is_up() {
        assert!(Instant::now() < end, "the cut was not noticed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let end = Instant::now() + Duration::from_secs(30);
    while !link.state().is_up() {
        assert!(Instant::now() < end, "not reconnected: {:?}", link.state());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    assert_eq!(fs.list(&dir.display().to_string()).await.unwrap().len(), 2);
    let (_, c) = link.current().unwrap();
    assert!(c.via().is_some(), "reconnected through the jump host");
    sup.abort();
}
