//! SSH/SFTP backend built on `russh` + `russh-sftp` (pure Rust).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, EntryKind, Vfs, VfsError, VfsResult};
use rolid_core::vfs::raw_text;
use rolid_text::{txt, Text};
use russh::client;
use russh::keys::{self, HashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use tokio::io::AsyncSeekExt;

pub mod auth;
pub mod clock;
pub mod hostkeys;
pub mod link;
mod upload;
pub use auth::{KbdPrompter, KbdRequest};
pub use link::{Attempt, Link, LinkState, NeedUser, RemoteFs};

/// Parameters for opening an SSH session.
#[derive(Clone, Default)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub key_path: Option<PathBuf>,
    pub key_passphrase: Option<String>,
    /// SHA256 fingerprint of an unknown host key the user explicitly
    /// accepted; it is added to [`known_hosts`](Self::known_hosts).
    pub accept_fingerprint: Option<String>,
    /// The program's own list of known server keys: checked, and the keys the
    /// user accepts are added to it. `None`: an accepted key is not saved.
    pub known_hosts: Option<PathBuf>,
    /// Lists only read, never written (`~/.ssh/known_hosts`). See [`hostkeys`].
    pub known_hosts_read: Vec<PathBuf>,
    /// Answers keyboard-interactive questions the client cannot answer itself.
    pub prompter: Option<Arc<dyn KbdPrompter>>,
    /// Do not use ssh-agent (tests, explicit choice).
    pub skip_agent: bool,
    /// Do not try `~/.ssh/id_*` (tests, explicit choice).
    pub skip_default_keys: bool,
    /// Keepalive (interval, unanswered probes before the connection counts as
    /// dead); default [`link::KEEPALIVE_INTERVAL`] × [`link::KEEPALIVE_MAX`].
    pub keepalive: Option<(Duration, usize)>,
    /// A transfer that gets nothing from the server for this long counts the
    /// connection as dead; default [`link::STALL_LIMIT`].
    pub stall_limit: Option<Duration>,
    /// How long one SFTP request may wait for its answer; default
    /// [`SFTP_REQUEST_TIMEOUT`].
    pub request_timeout: Option<Duration>,
}

impl std::fmt::Debug for ConnectParams {
    // Never print secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("key_path", &self.key_path)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub enum SshError {
    UnknownHostKey {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
    },
    HostKeyChanged {
        host: String,
        port: u16,
        file: String,
        line: usize,
    },
    /// The key is marked `@revoked` in a list of known servers.
    HostKeyRevoked {
        host: String,
        port: u16,
        file: String,
        line: usize,
    },
    /// One list knows the key the server sent, another has a different one.
    HostKeysDisagree {
        host: String,
        port: u16,
        knows: (String, usize),
        other: (String, usize),
    },
    /// The authentication report (see [`auth::Report::text`]).
    AuthFailed(Text),
    Other(Text),
}

impl SshError {
    /// The message for the user (English template, translated by the UI).
    pub fn text(&self) -> Text {
        match self {
            SshError::UnknownHostKey { host, port, algorithm, fingerprint } => txt!(
                "unknown server key {host}:{port} ({algorithm}) {fingerprint}",
                host = host,
                port = port.to_string(),
                algorithm = algorithm,
                fingerprint = fingerprint
            ),
            SshError::HostKeyChanged { host, port, file, line } => txt!(
                "WARNING: the key of server {host}:{port} has changed compared to {file}, line {line}! It may be a man-in-the-middle attack. Connection refused.",
                host = host,
                port = port.to_string(),
                file = file,
                line = line.to_string()
            ),
            SshError::HostKeyRevoked { host, port, file, line } => txt!(
                "The key of server {host}:{port} is marked as revoked in {file}, line {line}. Connection refused.",
                host = host,
                port = port.to_string(),
                file = file,
                line = line.to_string()
            ),
            SshError::HostKeysDisagree { host, port, knows, other } => txt!(
                "The lists of known servers disagree about server {host}:{port}: {knows}, line {knows_line}, has the key the server sent, but {other}, line {other_line}, has a different key of the same type. Connection refused. Find out from the server's administrator which key is right and delete the wrong line.",
                host = host,
                port = port.to_string(),
                knows = knows.0.clone(),
                knows_line = knows.1.to_string(),
                other = other.0.clone(),
                other_line = other.1.to_string()
            ),
            SshError::AuthFailed(report) => txt!("login failed. {report}", report = report),
            SshError::Other(t) => t.clone(),
        }
    }
}

impl std::fmt::Display for SshError {
    /// English (the log).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text())
    }
}

impl std::error::Error for SshError {}

#[derive(Debug, Clone)]
enum HostKeyVerdict {
    Unknown {
        algorithm: String,
        fingerprint: String,
    },
    /// Refused: changed, revoked, the lists disagree, a list cannot be read.
    Refused(SshError),
}

/// A file name for the user: the home folder as `~`.
fn shown(p: &std::path::Path) -> String {
    if let Some(home) = std::env::home_dir() {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}

struct ClientHandler {
    host: String,
    port: u16,
    accept_fingerprint: Option<String>,
    known_hosts: Option<PathBuf>,
    known_hosts_read: Vec<PathBuf>,
    verdict: Arc<Mutex<Option<HostKeyVerdict>>>,
    alive: Arc<Alive>,
}

/// Milliseconds since the first call (a clock shared by all connections).
fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}

/// When the server was last heard from (see [`SshConnection::quiet_for`]).
struct Alive {
    /// [`now_ms`] of the last data or answer to a ping.
    last_ms: std::sync::atomic::AtomicU64,
    /// A ping is on the way.
    pinging: std::sync::atomic::AtomicBool,
    stall_limit: Duration,
    request_timeout: Duration,
}

impl Alive {
    fn heard(&self) {
        self.last_ms.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn data(&mut self, _channel: russh::ChannelId, _data: &[u8], _session: &mut client::Session) -> Result<(), Self::Error> {
        self.alive.heard();
        Ok(())
    }

    async fn extended_data(
        &mut self,
        _channel: russh::ChannelId,
        _ext: u32,
        _data: &[u8],
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.alive.heard();
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            // Host certificates are not supported yet.
            return Ok(false);
        };
        let name = hostkeys::host_port(&self.host, self.port);
        let mut found = Vec::new();
        for p in self.known_hosts.iter().chain(&self.known_hosts_read) {
            match hostkeys::read(p) {
                Ok(text) => found.push((p.clone(), hostkeys::check_text(&text, &name, key))),
                Err(e) => {
                    let refused = SshError::Other(txt!(
                        "cannot read the list of known servers {file}: {error}. Connection refused.",
                        file = shown(p),
                        error = e.to_string()
                    ));
                    *self.verdict.lock().unwrap() = Some(HostKeyVerdict::Refused(refused));
                    return Ok(false);
                }
            }
        }
        let (host, port) = (self.host.clone(), self.port);
        let refused = match hostkeys::decide(&found) {
            hostkeys::Verdict::Known => return Ok(true),
            hostkeys::Verdict::Unknown => {
                let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
                if self.accept_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                    match &self.known_hosts {
                        Some(p) => {
                            if let Err(e) = hostkeys::learn(p, &self.host, self.port, key) {
                                tracing_warn(&format!("cannot write {}: {e}", p.display()));
                            }
                        }
                        None => tracing_warn("an accepted server key is not saved: no list of known servers"),
                    }
                    return Ok(true);
                }
                *self.verdict.lock().unwrap() = Some(HostKeyVerdict::Unknown {
                    algorithm: key.algorithm().to_string(),
                    fingerprint,
                });
                return Ok(false);
            }
            hostkeys::Verdict::Changed { file, line } => SshError::HostKeyChanged { host, port, file: shown(&file), line },
            hostkeys::Verdict::Revoked { file, line } => SshError::HostKeyRevoked { host, port, file: shown(&file), line },
            hostkeys::Verdict::Disagree { knows, other } => SshError::HostKeysDisagree {
                host,
                port,
                knows: (shown(&knows.0), knows.1),
                other: (shown(&other.0), other.1),
            },
        };
        tracing::warn!("server key refused: {refused}");
        *self.verdict.lock().unwrap() = Some(HostKeyVerdict::Refused(refused));
        Ok(false)
    }
}

/// The key types the lists of known servers have for the server of `p`.
fn known_types(p: &ConnectParams) -> Vec<keys::Algorithm> {
    let name = hostkeys::host_port(&p.host, p.port);
    let mut v = Vec::new();
    for f in p.known_hosts.iter().chain(&p.known_hosts_read) {
        for a in hostkeys::known_types(&hostkeys::read(f).unwrap_or_default(), &name) {
            if !v.contains(&a) {
                v.push(a);
            }
        }
    }
    v
}

fn tracing_warn(msg: &str) {
    tracing::warn!("{msg}");
}

/// How long a short check command on the server may run (see
/// [`SshConnection::exec_probe`]).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// An authenticated SSH connection. Cheap to clone; multiple panels/tabs can
/// share one connection (each opens its own channels).
#[derive(Clone)]
pub struct SshConnection {
    handle: Arc<client::Handle<ClientHandler>>,
    pub label: String,
    /// `user@host:port`: names the server in the local journal of unfinished
    /// replacements ([`rolid_core::replace`]).
    pub key: String,
    /// The address actually connected to (after the DNS lookup).
    pub peer: Option<std::net::SocketAddr>,
    alive: Arc<Alive>,
}

impl SshConnection {
    pub async fn connect(p: &ConnectParams) -> Result<Self, SshError> {
        let (ka_interval, ka_max) = p.keepalive.unwrap_or((link::KEEPALIVE_INTERVAL, link::KEEPALIVE_MAX));
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(ka_interval),
            keepalive_max: ka_max,
            nodelay: true,
            window_size: SSH_CHANNEL_WINDOW,
            preferred: russh::Preferred { key: hostkeys::preferred(&known_types(p)).into(), ..Default::default() },
            ..Default::default()
        });
        let verdict = Arc::new(Mutex::new(None));
        let alive = Arc::new(Alive {
            last_ms: std::sync::atomic::AtomicU64::new(now_ms()),
            pinging: std::sync::atomic::AtomicBool::new(false),
            stall_limit: p.stall_limit.unwrap_or(link::STALL_LIMIT),
            request_timeout: p.request_timeout.unwrap_or(SFTP_REQUEST_TIMEOUT),
        });
        let handler = ClientHandler {
            host: p.host.clone(),
            port: p.port,
            accept_fingerprint: p.accept_fingerprint.clone(),
            known_hosts: p.known_hosts.clone(),
            known_hosts_read: p.known_hosts_read.clone(),
            verdict: verdict.clone(),
            alive: alive.clone(),
        };
        // The TCP connection is opened here (not by russh) to learn which of
        // the addresses of the name it went to.
        let addr = (p.host.as_str(), p.port);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let stream = tcp_connect(addr, deadline).await?;
        if let Err(e) = stream.set_nodelay(true) {
            tracing::warn!("set_nodelay failed: {e}");
        }
        let peer = stream.peer_addr().ok();
        let connect = tokio::time::timeout_at(deadline, client::connect_stream(config, stream, handler));
        let mut handle = match connect.await {
            Err(_) => return Err(SshError::Other(txt!("the connection timed out"))),
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                return Err(match verdict.lock().unwrap().take() {
                    Some(HostKeyVerdict::Unknown {
                        algorithm,
                        fingerprint,
                    }) => SshError::UnknownHostKey {
                        host: p.host.clone(),
                        port: p.port,
                        algorithm,
                        fingerprint,
                    },
                    Some(HostKeyVerdict::Refused(e)) => e,
                    None => SshError::Other(txt!("connection failed: {error}", error = e.to_string())),
                });
            }
        };

        auth::authenticate(&mut handle, p).await?;
        alive.heard();
        Ok(Self {
            handle: Arc::new(handle),
            label: format!("{}@{}", p.user, p.host),
            key: format!("{}@{}:{}", p.user, p.host, p.port),
            peer,
            alive,
        })
    }

    /// How long nothing came from the server (no data on any channel, no
    /// answer to a ping).
    pub fn quiet_for(&self) -> Duration {
        Duration::from_millis(now_ms().saturating_sub(self.alive.last_ms.load(std::sync::atomic::Ordering::Relaxed)))
    }

    /// A transfer that hears nothing from the server for this long counts the
    /// connection as dead (see [`link::STALL_LIMIT`]).
    pub fn stall_limit(&self) -> Duration {
        self.alive.stall_limit
    }

    /// Ask the server for an answer (in the background; one at a time): a
    /// server that is only slow answers, and [`Self::quiet_for`] starts again.
    pub fn ping(&self) {
        use std::sync::atomic::Ordering;
        if self.alive.pinging.swap(true, Ordering::Relaxed) {
            return;
        }
        let (handle, alive) = (self.handle.clone(), self.alive.clone());
        tokio::spawn(async move {
            if tokio::time::timeout(alive.stall_limit, handle.send_ping()).await.is_ok_and(|r| r.is_ok()) && !handle.is_closed() {
                alive.heard();
            }
            alive.pinging.store(false, Ordering::Relaxed);
        });
    }

    /// Give up this connection (it is replaced by a new one): close it in the
    /// background, so everything still waiting on it ends at once.
    pub fn abandon(&self) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                handle.disconnect(russh::Disconnect::ByApplication, "connection stalled", ""),
            )
            .await;
        });
    }

    /// Both are the same connection (not just to the same server).
    pub fn same(&self, other: &SshConnection) -> bool {
        Arc::ptr_eq(&self.handle, &other.handle)
    }

    /// The SSH session has ended (keepalive unanswered, server gone, closed).
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Resolves once the connection is closed (checked every [`CLOSED_POLL`]).
    pub async fn closed(&self) {
        while !self.is_closed() {
            tokio::time::sleep(CLOSED_POLL).await;
        }
    }

    /// Run `cmd` on the server (in the user's login shell) and stream stdout to
    /// `on_stdout`; return `false` from it to stop early. `cancel` is polled even
    /// while the command produces no output. Returns (exit status, stderr head).
    pub async fn exec_stream(
        &self,
        cmd: &str,
        cancel: &std::sync::atomic::AtomicBool,
        on_stdout: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<(Option<u32>, String), SshError> {
        self.exec_limited(cmd, cancel, None, on_stdout).await
    }

    /// A short check of what the server can do ("is there GNU find?"): like
    /// [`exec_stream`](Self::exec_stream), but ended after [`PROBE_TIMEOUT`]
    /// with an error, so that a shell that hangs (a login script waiting for
    /// input, a stuck profile) only makes the caller use SFTP instead.
    pub async fn exec_probe(
        &self,
        cmd: &str,
        cancel: &std::sync::atomic::AtomicBool,
        on_stdout: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<(Option<u32>, String), SshError> {
        self.exec_limited(cmd, cancel, Some(PROBE_TIMEOUT), on_stdout).await
    }

    /// [`exec_stream`](Self::exec_stream) ended after `limit` (counted from
    /// the start, the opening of the channel included).
    pub async fn exec_limited(
        &self,
        cmd: &str,
        cancel: &std::sync::atomic::AtomicBool,
        limit: Option<Duration>,
        on_stdout: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<(Option<u32>, String), SshError> {
        use std::sync::atomic::Ordering;
        const STDERR_MAX: usize = 4096;
        let deadline = limit.map(|l| tokio::time::Instant::now() + l);
        let timed_out = || {
            SshError::Other(txt!(
                "the command on the server did not answer within {n} s",
                n = limit.unwrap_or_default().as_secs().to_string()
            ))
        };
        let open = async {
            let ch = self.handle.channel_open_session().await.map_err(other)?;
            ch.exec(true, cmd).await.map_err(other)?;
            // No end of input is sent: commands run through
            // `rolid_search::remote::guarded` watch their input and stop when it
            // ends (the channel was closed: canceled). They read nothing from it.
            // A shell that waits for input is ended by `limit` instead.
            Ok::<_, SshError>(ch)
        };
        let mut ch = match deadline {
            Some(d) => tokio::time::timeout_at(d, open).await.map_err(|_| timed_out())??,
            None => open.await?,
        };
        let mut status = None;
        let mut stderr = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = ch.close().await;
                break;
            }
            if deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
                let _ = ch.close().await;
                return Err(timed_out());
            }
            let msg = match tokio::time::timeout(Duration::from_millis(200), ch.wait()).await {
                Err(_) => continue, // poll cancel and the deadline
                Ok(None) => break,
                Ok(Some(m)) => m,
            };
            match msg {
                russh::ChannelMsg::Data { data } => {
                    if !on_stdout(&data) {
                        let _ = ch.close().await;
                        break;
                    }
                }
                russh::ChannelMsg::ExtendedData { data, .. } => {
                    let room = STDERR_MAX.saturating_sub(stderr.len());
                    stderr.extend_from_slice(&data[..data.len().min(room)]);
                }
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok((status, String::from_utf8_lossy(&stderr).into_owned()))
    }

    /// Open a new SFTP session over a fresh channel of this connection.
    pub async fn sftp(&self) -> Result<SftpFs, SshError> {
        let channel = self.handle.channel_open_session().await.map_err(other)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(other)?;
        let cfg = russh_sftp::client::Config {
            max_concurrent_reads: SFTP_READS_IN_FLIGHT,
            max_concurrent_writes: SFTP_WRITES_IN_FLIGHT,
            request_timeout_secs: self.alive.request_timeout.as_secs().max(1),
            ..Default::default()
        };
        let sftp = SftpSession::new_with_config(channel.into_stream(), cfg)
            .await
            .map_err(other)?;
        Ok(SftpFs {
            conn: self.clone(),
            sftp,
            posix_rename: std::sync::atomic::AtomicU8::new(POSIX_RENAME_UNKNOWN),
            aux: tokio::sync::OnceCell::new(),
            confirmed: upload::Confirmed::default(),
        })
    }

    /// A low-level SFTP session on a new channel (for extension requests the
    /// high-level session does not offer), with the server's extension list.
    async fn raw_sftp(
        &self,
    ) -> Result<(russh_sftp::client::RawSftpSession, russh_sftp::protocol::Version), SshError> {
        let channel = self.handle.channel_open_session().await.map_err(other)?;
        channel.request_subsystem(true, "sftp").await.map_err(other)?;
        let raw = russh_sftp::client::RawSftpSession::new(channel.into_stream());
        raw.set_timeout(self.alive.request_timeout.as_secs().max(1));
        let version = raw.init().await.map_err(other)?;
        Ok((raw, version))
    }
}

/// One try of the TCP connection: when the line was down, the first packets
/// were lost and the system would send them again only after 1, 3, 7, 15 s…
/// A new try every few seconds gets through a line that is up for a moment
/// (the flapping line of the bad-line test: down for 30 s, up for 8).
pub const TCP_TRY: Duration = Duration::from_secs(4);

/// Open the TCP connection to `addr` until `deadline`, a new try every [`TCP_TRY`].
async fn tcp_connect(addr: (&str, u16), deadline: tokio::time::Instant) -> Result<tokio::net::TcpStream, SshError> {
    loop {
        let try_until = (tokio::time::Instant::now() + TCP_TRY).min(deadline);
        match tokio::time::timeout_at(try_until, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(s)) => return Ok(s),
            // Refused, unreachable: said at once (the reconnection tries again later).
            Ok(Err(e)) => return Err(SshError::Other(txt!("connection failed: {error}", error = e.to_string()))),
            Err(_) if try_until >= deadline => return Err(SshError::Other(txt!("the connection timed out"))),
            Err(_) => {}
        }
    }
}

/// Read requests of a transfer sent ahead. OpenSSH answers up to 255 kB each,
/// so about 4 MB are on the way: enough for a fast line far away (40 MB/s at
/// a round trip of 0.1 s), twice what OpenSSH's own `sftp` keeps.
pub const SFTP_READS_IN_FLIGHT: usize = 16;
/// Write requests sent ahead (32 kB each: 2 MB on the way).
pub const SFTP_WRITES_IN_FLIGHT: usize = 64;
/// How long one SFTP request may wait for its answer. The answers come in
/// order, so on a slow line the last request sent ahead waits for all the
/// data before it: with 64 reads of 255 kB and 30 s, every line slower than
/// about 0.5 MB/s "timed out" and was reconnected while it still worked
/// (found with the bad-line test of 0.8.17). A dead connection is noticed by
/// the keepalive anyway (3 × 15 s) and then fails every request at once.
pub const SFTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
/// The data a transfer keeps on the way, in bytes (the larger direction).
pub const SFTP_BYTES_IN_FLIGHT: u64 = {
    let r = SFTP_READS_IN_FLIGHT as u64 * 255 * 1024;
    let w = SFTP_WRITES_IN_FLIGHT as u64 * 32 * 1024;
    if r > w { r } else { w }
};

/// How much the server may send on one channel before we confirm it (the SSH
/// channel window). russh's default of 2 MB, topped up only when half is
/// used, let a download over a distant line move only 1–2 MB per round trip:
/// 128 MiB at 100 ms took 29 s against 19–26 s for OpenSSH's `sftp`; with
/// room for twice the data a transfer keeps on the way it takes 14 s.
pub const SSH_CHANNEL_WINDOW: u32 = 8 * 1024 * 1024;

/// How often an operation in progress checks that its connection still exists.
pub const CLOSED_POLL: Duration = Duration::from_millis(100);

/// `posix-rename@openssh.com`: rename that replaces the target atomically
/// (plain SFTPv3 rename refuses an existing target).
pub const POSIX_RENAME: &str = "posix-rename@openssh.com";
const POSIX_RENAME_UNKNOWN: u8 = 0;
const POSIX_RENAME_YES: u8 = 1;
const POSIX_RENAME_NO: u8 = 2;

fn other(e: impl std::fmt::Display) -> SshError {
    SshError::Other(raw_text(e))
}

/// More entries than this in one folder: the listing of a broken or hostile
/// server that never ends stops here instead of filling the memory (a real
/// folder of a million files takes about a fifth of it).
pub const MAX_LISTING: usize = 5_000_000;
/// The error of such a listing (`Error::Limited`; told to the user by
/// [`link::classify_sftp`]).
const TOO_LONG: &str = "the listing does not end";

/// SFTP-backed filesystem.
pub struct SftpFs {
    conn: SshConnection,
    sftp: SftpSession,
    /// Does the server offer [`POSIX_RENAME`]? (0 = not asked yet, 1 = yes, 2 = no)
    posix_rename: std::sync::atomic::AtomicU8,
    /// A second, low-level SFTP session (opened when first needed) for reading
    /// folders and for [`POSIX_RENAME`], with whether the server offers that.
    /// `None` = the server refused another channel.
    aux: tokio::sync::OnceCell<Option<(Arc<russh_sftp::client::RawSftpSession>, bool)>>,
    /// How far the server confirmed the files written last.
    confirmed: upload::Confirmed,
}

/// Read the whole folder `path` with READDIR requests: each batch is appended
/// (russh-sftp's `read_dir` rebuilds the whole list for every batch of ~100
/// names, which makes a folder with a million files take many minutes).
async fn read_dir_batched(
    raw: &russh_sftp::client::RawSftpSession,
    path: &str,
) -> Result<Vec<russh_sftp::protocol::File>, russh_sftp::client::error::Error> {
    use russh_sftp::client::error::Error;
    use russh_sftp::protocol::StatusCode;
    let handle = raw.opendir(path).await?.handle;
    let mut files = Vec::new();
    let r = loop {
        if files.len() > MAX_LISTING {
            break Err(Error::Limited(TOO_LONG.into()));
        }
        match raw.readdir(handle.as_str()).await {
            Ok(name) => files.extend(name.files),
            Err(Error::Status(st)) if st.status_code == StatusCode::Eof => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    let _ = raw.close(handle).await;
    r.map(|()| files)
}

impl SftpFs {
    async fn aux(&self) -> Option<&(Arc<russh_sftp::client::RawSftpSession>, bool)> {
        self.aux
            .get_or_init(|| async {
                let (raw, version) = self.conn.raw_sftp().await.ok()?;
                let posix = version.extensions.get(POSIX_RENAME).is_some_and(|v| v == "1");
                Some((Arc::new(raw), posix))
            })
            .await
            .as_ref()
    }

    /// The folder's entries (names and attributes, without `.` and `..`).
    async fn read_dir(&self, path: &str) -> VfsResult<Vec<(String, FileAttributes)>> {
        let all: Vec<(String, FileAttributes)> = match self.aux().await {
            Some((raw, _)) => read_dir_batched(raw, path)
                .await
                .map_err(link::classify_sftp)?
                .into_iter()
                .map(|f| (f.filename, f.attrs))
                .collect(),
            None => self
                .sftp
                .read_dir(path)
                .await
                .map_err(link::classify_sftp)?
                .map(|de| (de.file_name(), de.metadata()))
                .collect(),
        };
        Ok(all.into_iter().filter(|(n, _)| n != "." && n != "..").collect())
    }

    pub fn connection(&self) -> &SshConnection {
        &self.conn
    }

    /// Open `path` for writing at `offset`; `confirmed` follows how far the
    /// server confirmed it (see [`upload::AckWriter`]; without the second
    /// session it stays at `offset`).
    pub(crate) async fn open_write_counted(
        &self,
        path: &str,
        offset: u64,
        confirmed: Arc<std::sync::atomic::AtomicU64>,
    ) -> VfsResult<Writer> {
        let mut flags = OpenFlags::CREATE | OpenFlags::WRITE;
        if offset == 0 {
            flags |= OpenFlags::TRUNCATE;
        }
        confirmed.store(offset, std::sync::atomic::Ordering::Relaxed);
        if let Some((raw, _)) = self.aux().await {
            let handle = raw.open(path, flags, FileAttributes::empty()).await.map_err(link::classify_sftp)?.handle;
            return Ok(Box::new(upload::AckWriter::new(raw.clone(), handle, offset, SFTP_WRITES_IN_FLIGHT, confirmed)));
        }
        let mut f = self.sftp.open_with_flags(path, flags).await.map_err(link::classify_sftp)?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }

    /// Never use [`POSIX_RENAME`] (tests of the fallback path).
    pub fn disable_posix_rename(&self) {
        self.posix_rename.store(POSIX_RENAME_NO, std::sync::atomic::Ordering::Relaxed);
    }

    /// Atomic replace via [`POSIX_RENAME`]; `Ok(false)` when the server does not
    /// offer it.
    async fn posix_rename(&self, from: &str, to: &str) -> VfsResult<bool> {
        use russh_sftp::protocol::{Packet, StatusCode};
        use std::sync::atomic::Ordering;
        if self.posix_rename.load(Ordering::Relaxed) == POSIX_RENAME_NO {
            return Ok(false);
        }
        // The kept session; a new one (closed afterwards) when it could not be opened.
        let own;
        let (raw, offered, owned) = match self.aux().await {
            Some((raw, offered)) => (&**raw, *offered, false),
            None => {
                let (raw, version) = self.conn.raw_sftp().await.map_err(verr)?;
                own = raw;
                (&own, version.extensions.get(POSIX_RENAME).is_some_and(|v| v == "1"), true)
            }
        };
        let close = || {
            if owned {
                let _ = raw.close_session();
            }
        };
        let flag = if offered { POSIX_RENAME_YES } else { POSIX_RENAME_NO };
        // Keep an explicit "disabled" (set meanwhile) as it is.
        let _ = self.posix_rename.compare_exchange(POSIX_RENAME_UNKNOWN, flag, Ordering::Relaxed, Ordering::Relaxed);
        if !offered {
            close();
            return Ok(false);
        }
        // Same wire format as hardlink@openssh.com: string oldpath, string newpath.
        let data: Vec<u8> = russh_sftp::extensions::HardlinkExtension { oldpath: from.into(), newpath: to.into() }
            .try_into()
            .map_err(verr)?;
        let reply = raw.extended(POSIX_RENAME, data).await;
        close();
        match reply {
            Ok(Packet::Status(st)) if st.status_code == StatusCode::Ok => Ok(true),
            Ok(Packet::Status(st)) => Err(VfsError::raw(crate::link::status_text(&st))),
            Ok(_) => Err(VfsError::Backend(txt!("{name}: unexpected answer from the server", name = POSIX_RENAME))),
            Err(russh_sftp::client::error::Error::Status(st)) => {
                Err(VfsError::raw(crate::link::status_text(&st)))
            }
            Err(e) => Err(verr(e)),
        }
    }
}

fn verr(e: impl std::fmt::Display) -> VfsError {
    VfsError::raw(e)
}

fn attrs_to_entry(name: String, a: &FileAttributes, link_to_dir: bool) -> Entry {
    let ft = a.file_type();
    let kind = if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Entry {
        name,
        kind,
        link_to_dir,
        size: if kind == EntryKind::File {
            a.size.unwrap_or(0)
        } else {
            0
        },
        modified: a
            .mtime
            .map(|t| SystemTime::UNIX_EPOCH + Duration::from_secs(t as u64)),
        mode: a.permissions,
    }
}

pub fn posix_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn posix_parent(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    if t.is_empty() {
        return None; // root
    }
    match t.rfind('/') {
        Some(0) => Some("/".into()),
        Some(i) => Some(t[..i].to_string()),
        None => None,
    }
}

#[async_trait]
impl Vfs for SftpFs {
    fn label(&self) -> String {
        self.conn.label.clone()
    }
    fn journal_key(&self) -> Option<String> {
        Some(self.conn.key.clone())
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn join(&self, dir: &str, name: &str) -> String {
        posix_join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        posix_parent(path)
    }
    async fn home(&self) -> VfsResult<String> {
        self.sftp.canonicalize(".").await.map_err(link::classify_sftp)
    }
    async fn canonicalize(&self, path: &str) -> VfsResult<String> {
        self.sftp.canonicalize(path).await.map_err(link::classify_sftp)
    }
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>> {
        let rd = self.read_dir(path).await?;
        let mut out = Vec::with_capacity(rd.len());
        for (name, meta) in rd {
            let link_to_dir = meta.file_type().is_symlink()
                && self
                    .sftp
                    .metadata(posix_join(path, &name))
                    .await
                    .map(|m| m.file_type().is_dir())
                    .unwrap_or(false);
            out.push(attrs_to_entry(name, &meta, link_to_dir));
        }
        Ok(out)
    }
    async fn stat(&self, path: &str) -> VfsResult<Entry> {
        let meta = self.sftp.symlink_metadata(path).await.map_err(link::classify_sftp)?;
        let link_to_dir = meta.file_type().is_symlink()
            && self
                .sftp
                .metadata(path)
                .await
                .map(|m| m.file_type().is_dir())
                .unwrap_or(false);
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        Ok(attrs_to_entry(name, &meta, link_to_dir))
    }
    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.sftp.create_dir(path).await.map_err(link::classify_sftp)
    }
    async fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.sftp.remove_file(path).await.map_err(link::classify_sftp)
    }
    async fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.sftp.remove_dir(path).await.map_err(link::classify_sftp)
    }
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.sftp.rename(from, to).await.map_err(link::classify_sftp)
    }
    async fn rename_replace(&self, from: &str, to: &str) -> VfsResult<bool> {
        self.posix_rename(from, to).await
    }
    async fn rename_noreplace(&self, from: &str, to: &str) -> VfsResult<bool> {
        // SFTP version 3: "It is an error if there already exists a file with
        // the name specified by newpath" (OpenSSH links and unlinks).
        self.sftp.rename(from, to).await.map_err(link::classify_sftp)?;
        Ok(true)
    }
    async fn space(&self, path: &str) -> VfsResult<Option<rolid_core::vfs::Space>> {
        // statvfs@openssh.com; None when the server does not offer it.
        let st = self.sftp.fs_info(path).await.map_err(link::classify_sftp)?;
        Ok(st.map(|s| rolid_core::vfs::Space {
            avail: s.blocks_avail.saturating_mul(s.fragment_size),
            total: s.blocks.saturating_mul(s.fragment_size),
        }))
    }
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader> {
        let mut f = self.sftp.open(path).await.map_err(link::classify_sftp)?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }
    async fn open_write_at(&self, path: &str, offset: u64) -> VfsResult<Writer> {
        self.open_write_counted(path, offset, self.confirmed.start(path)).await
    }
    fn write_confirmed(&self, path: &str) -> Option<u64> {
        self.confirmed.get(path)
    }
    async fn set_attrs(
        &self,
        path: &str,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
    ) -> VfsResult<()> {
        let mut attrs = FileAttributes::empty();
        attrs.permissions = mode;
        if let Some(t) = mtime {
            let secs = t
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0) as u32;
            attrs.mtime = Some(secs);
            attrs.atime = Some(secs);
        }
        self.sftp.set_metadata(path, attrs).await.map_err(link::classify_sftp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_paths() {
        assert_eq!(posix_join("/", "etc"), "/etc");
        assert_eq!(posix_join("/home/u", "x"), "/home/u/x");
        assert_eq!(posix_parent("/home/u"), Some("/home".into()));
        assert_eq!(posix_parent("/home"), Some("/".into()));
        assert_eq!(posix_parent("/"), None);
    }

    #[test]
    fn a_slow_line_does_not_time_out_a_working_transfer() {
        // The last request sent ahead is answered after all the data before
        // it: that must fit the timeout on a line of 512 kbit/s (64 kB/s)…
        let slowest = SFTP_BYTES_IN_FLIGHT / SFTP_REQUEST_TIMEOUT.as_secs();
        assert!(slowest < 64 * 1024, "only lines over {slowest} B/s finish in time");
        // …while a dead connection is still noticed by the keepalive sooner.
        let keepalive = link::KEEPALIVE_INTERVAL * link::KEEPALIVE_MAX as u32;
        assert!(keepalive < SFTP_REQUEST_TIMEOUT);
        // And a fast line far away still has enough on the way (0.1 s round trip, 30 MB/s).
        assert!(SFTP_READS_IN_FLIGHT as u64 * 255 * 1024 >= 3 * 1024 * 1024);
    }

    #[test]
    fn the_channel_window_does_not_hold_back_a_download() {
        // The window is topped up when half of it is used: the reads sent
        // ahead must fit into that half, or the server waits for us every
        // round trip.
        assert!(u64::from(SSH_CHANNEL_WINDOW) / 2 >= SFTP_READS_IN_FLIGHT as u64 * 255 * 1024);
    }
}
