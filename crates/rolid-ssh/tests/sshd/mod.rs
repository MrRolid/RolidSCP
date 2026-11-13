//! A real OpenSSH server (`/usr/sbin/sshd`, internal-sftp) for integration tests,
//! listening on 127.0.0.1 only, with a throwaway host key and client key, plus a
//! TCP proxy in front of it that can cut connections (network failure) on demand.
//!
//! Tests that need it are skipped (with a note) when sshd is not installed.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rolid_ssh::ConnectParams;

pub const SSHD: &str = "/usr/sbin/sshd";

pub struct Sshd {
    pub dir: PathBuf,
    pub port: u16,
    pub user: String,
    pub client_key: PathBuf,
    pub known_hosts: PathBuf,
    host_pub: String,
    child: Option<Child>,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn keygen(path: &Path) {
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "ssh-keygen failed");
}

impl Sshd {
    /// `None` when OpenSSH is not available (the test should return early).
    pub fn start(name: &str) -> Option<Sshd> {
        if !Path::new(SSHD).exists() || Command::new("ssh-keygen").arg("-?").output().is_err() {
            eprintln!("SKIP: {SSHD} / ssh-keygen not available");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("rolid-sshd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let host_key = dir.join("host_key");
        let client_key = dir.join("client_key");
        keygen(&host_key);
        keygen(&client_key);
        std::fs::copy(dir.join("client_key.pub"), dir.join("authorized_keys")).unwrap();
        let user = std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| {
            String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap().trim().to_string()
        });
        let host_pub = std::fs::read_to_string(dir.join("host_key.pub")).unwrap();
        let host_pub = host_pub.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let mut s = Sshd {
            port: 0,
            user,
            client_key,
            known_hosts: dir.join("known_hosts"),
            host_pub,
            child: None,
            dir,
        };
        s.port = free_port();
        s.spawn();
        s.add_known_port(s.port);
        Some(s)
    }

    fn spawn(&mut self) {
        let d = &self.dir;
        let cfg = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\n\
             AuthorizedKeysFile {d}/authorized_keys\nPasswordAuthentication no\n\
             KbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\n\
             PidFile {d}/sshd.pid\nSubsystem sftp internal-sftp\nAllowUsers {user}\n\
             MaxSessions 50\nMaxStartups 50\nLogLevel ERROR\n",
            port = self.port,
            d = d.display(),
            user = self.user,
        );
        std::fs::write(d.join("sshd_config"), cfg).unwrap();
        let child = Command::new(SSHD)
            .args(["-D", "-e", "-f"])
            .arg(d.join("sshd_config"))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(d.join("sshd.log")).unwrap())
            .spawn()
            .expect("sshd start");
        self.child = Some(child);
        // Wait until it listens.
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("sshd did not start: {}", std::fs::read_to_string(d.join("sshd.log")).unwrap_or_default());
    }

    /// Stop the listening server and every connection it serves (server restart / crash).
    pub fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            // Per-connection processes detach from the listener: find them by
            // the server port of their sockets (only this server's).
            let out = Command::new("ss")
                .args(["-tnpH", "state", "established", &format!("sport = :{}", self.port)])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            for pid in out.split("pid=").skip(1).filter_map(|t| t.split(|c: char| !c.is_ascii_digit()).next()) {
                let _ = Command::new("kill").args(["-KILL", pid]).status();
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Start again on the same port (after [`Sshd::kill`]).
    pub fn restart(&mut self) {
        self.kill();
        self.spawn();
    }

    /// Trust the host key also under `port` (connections through a proxy).
    pub fn add_known_port(&self, port: u16) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.known_hosts).unwrap();
        writeln!(f, "[127.0.0.1]:{port} {}", self.host_pub).unwrap();
    }

    /// Connection parameters (key auth, no agent) for `port` (the server or a proxy).
    pub fn params(&self, port: u16) -> ConnectParams {
        ConnectParams {
            host: "127.0.0.1".into(),
            port,
            user: self.user.clone(),
            key_path: Some(self.client_key.clone()),
            known_hosts: Some(self.known_hosts.clone()),
            skip_agent: true,
            skip_default_keys: true,
            ..Default::default()
        }
    }

    /// A fresh directory for test files (inside the test dir).
    pub fn data_dir(&self, name: &str) -> PathBuf {
        let d = self.dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// TCP proxy 127.0.0.1:port → upstream that can drop all its connections
/// (like a network failure), drop them after N more bytes from the server,
/// refuse new ones (server unreachable) or silently stop forwarding (black hole).
#[derive(Clone)]
pub struct Proxy {
    pub port: u16,
    conns: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    /// Bytes forwarded server → client so far.
    pub down: Arc<AtomicU64>,
    cut_at: Arc<AtomicU64>,
    pub refuse: Arc<AtomicBool>,
    pub frozen: Arc<AtomicBool>,
    /// Connections accepted so far.
    pub accepted: Arc<AtomicU64>,
}

impl Proxy {
    pub async fn start(upstream: u16) -> Proxy {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = Proxy {
            port: listener.local_addr().unwrap().port(),
            conns: Arc::default(),
            down: Arc::default(),
            cut_at: Arc::new(AtomicU64::new(u64::MAX)),
            refuse: Arc::default(),
            frozen: Arc::default(),
            accepted: Arc::default(),
        };
        let me = p.clone();
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else { return };
                if me.refuse.load(Ordering::SeqCst) {
                    drop(client);
                    continue;
                }
                me.accepted.fetch_add(1, Ordering::SeqCst);
                let Ok(server) = tokio::net::TcpStream::connect(("127.0.0.1", upstream)).await else {
                    continue;
                };
                let m = me.clone();
                let h = tokio::spawn(async move { m.pipe(client, server).await });
                me.conns.lock().unwrap().push(h.abort_handle());
            }
        });
        p
    }

    async fn pipe(&self, client: tokio::net::TcpStream, server: tokio::net::TcpStream) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut cr, mut cw) = client.into_split();
        let (mut sr, mut sw) = server.into_split();
        let frozen = self.frozen.clone();
        let up = async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                while frozen.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let n = match cr.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                if sw.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        };
        let (down, cut_at, frozen) = (self.down.clone(), self.cut_at.clone(), self.frozen.clone());
        let dn = async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                while frozen.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let n = match sr.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                if cw.write_all(&buf[..n]).await.is_err() {
                    return;
                }
                if down.fetch_add(n as u64, Ordering::SeqCst) + n as u64 >= cut_at.load(Ordering::SeqCst) {
                    cut_at.store(u64::MAX, Ordering::SeqCst);
                    return; // dropping both halves closes both sockets
                }
            }
        };
        tokio::select! { _ = up => {}, _ = dn => {} }
    }

    /// Drop every open connection now.
    pub fn cut(&self) {
        for h in self.conns.lock().unwrap().drain(..) {
            h.abort();
        }
    }

    /// Drop the connection once `n` more bytes came from the server.
    pub fn cut_after(&self, n: u64) {
        self.cut_at.store(self.down.load(Ordering::SeqCst) + n, Ordering::SeqCst);
    }
}
