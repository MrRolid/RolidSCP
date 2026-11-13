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
pub mod link;
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
    /// accepted; it will be added to `~/.ssh/known_hosts`.
    pub accept_fingerprint: Option<String>,
    /// Override the known_hosts file (default `~/.ssh/known_hosts`).
    pub known_hosts: Option<PathBuf>,
    /// Answers keyboard-interactive questions the client cannot answer itself.
    pub prompter: Option<Arc<dyn KbdPrompter>>,
    /// Do not use ssh-agent (tests, explicit choice).
    pub skip_agent: bool,
    /// Do not try `~/.ssh/id_*` (tests, explicit choice).
    pub skip_default_keys: bool,
    /// Keepalive (interval, unanswered probes before the connection counts as
    /// dead); default [`link::KEEPALIVE_INTERVAL`] × [`link::KEEPALIVE_MAX`].
    pub keepalive: Option<(Duration, usize)>,
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
        line: usize,
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
            SshError::HostKeyChanged { host, port, .. } => txt!(
                "WARNING: the key of server {host}:{port} has changed compared to known_hosts! It may be a man-in-the-middle attack. Connection refused.",
                host = host,
                port = port.to_string()
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
    Changed {
        line: usize,
    },
}

struct ClientHandler {
    host: String,
    port: u16,
    accept_fingerprint: Option<String>,
    known_hosts: Option<PathBuf>,
    verdict: Arc<Mutex<Option<HostKeyVerdict>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            // Host certificates are not supported yet.
            return Ok(false);
        };
        let checked = match &self.known_hosts {
            Some(p) if !p.exists() => Ok(false),
            Some(p) => keys::check_known_hosts_path(&self.host, self.port, key, p),
            None => keys::check_known_hosts(&self.host, self.port, key),
        };
        match checked {
            Ok(true) => Ok(true),
            Ok(false) => {
                let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
                if self.accept_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                    let learned = match &self.known_hosts {
                        Some(p) => {
                            keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, p)
                        }
                        None => keys::known_hosts::learn_known_hosts(&self.host, self.port, key),
                    };
                    if let Err(e) = learned {
                        tracing_warn(&format!("cannot write known_hosts: {e}"));
                    }
                    return Ok(true);
                }
                *self.verdict.lock().unwrap() = Some(HostKeyVerdict::Unknown {
                    algorithm: key.algorithm().to_string(),
                    fingerprint,
                });
                Ok(false)
            }
            Err(keys::Error::KeyChanged { line }) => {
                *self.verdict.lock().unwrap() = Some(HostKeyVerdict::Changed { line });
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }
}

fn tracing_warn(msg: &str) {
    tracing::warn!("{msg}");
}

/// An authenticated SSH connection. Cheap to clone; multiple panels/tabs can
/// share one connection (each opens its own channels).
#[derive(Clone)]
pub struct SshConnection {
    handle: Arc<client::Handle<ClientHandler>>,
    pub label: String,
    /// The address actually connected to (after the DNS lookup).
    pub peer: Option<std::net::SocketAddr>,
}

impl SshConnection {
    pub async fn connect(p: &ConnectParams) -> Result<Self, SshError> {
        let (ka_interval, ka_max) = p.keepalive.unwrap_or((link::KEEPALIVE_INTERVAL, link::KEEPALIVE_MAX));
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(ka_interval),
            keepalive_max: ka_max,
            nodelay: true,
            ..Default::default()
        });
        let verdict = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            host: p.host.clone(),
            port: p.port,
            accept_fingerprint: p.accept_fingerprint.clone(),
            known_hosts: p.known_hosts.clone(),
            verdict: verdict.clone(),
        };
        // The TCP connection is opened here (not by russh) to learn which of
        // the addresses of the name it went to.
        let addr = (p.host.as_str(), p.port);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let stream = match tokio::time::timeout_at(deadline, tokio::net::TcpStream::connect(addr)).await {
            Err(_) => return Err(SshError::Other(txt!("the connection timed out"))),
            Ok(Err(e)) => return Err(SshError::Other(txt!("connection failed: {error}", error = e.to_string()))),
            Ok(Ok(s)) => s,
        };
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
                    Some(HostKeyVerdict::Changed { line }) => SshError::HostKeyChanged {
                        host: p.host.clone(),
                        port: p.port,
                        line,
                    },
                    None => SshError::Other(txt!("connection failed: {error}", error = e.to_string())),
                });
            }
        };

        auth::authenticate(&mut handle, p).await?;
        Ok(Self {
            handle: Arc::new(handle),
            label: format!("{}@{}", p.user, p.host),
            peer,
        })
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
        use std::sync::atomic::Ordering;
        const STDERR_MAX: usize = 4096;
        let mut ch = self.handle.channel_open_session().await.map_err(other)?;
        ch.exec(true, cmd).await.map_err(other)?;
        let mut status = None;
        let mut stderr = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = ch.close().await;
                break;
            }
            let msg = match tokio::time::timeout(Duration::from_millis(200), ch.wait()).await {
                Err(_) => continue, // poll cancel
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
            max_concurrent_reads: 64,
            max_concurrent_writes: 64,
            request_timeout_secs: 30,
            ..Default::default()
        };
        let sftp = SftpSession::new_with_config(channel.into_stream(), cfg)
            .await
            .map_err(other)?;
        Ok(SftpFs {
            conn: self.clone(),
            sftp,
            posix_rename: std::sync::atomic::AtomicU8::new(POSIX_RENAME_UNKNOWN),
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
        raw.set_timeout(30);
        let version = raw.init().await.map_err(other)?;
        Ok((raw, version))
    }
}

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

/// SFTP-backed filesystem.
pub struct SftpFs {
    conn: SshConnection,
    sftp: SftpSession,
    /// Does the server offer [`POSIX_RENAME`]? (0 = not asked yet, 1 = yes, 2 = no)
    posix_rename: std::sync::atomic::AtomicU8,
}

impl SftpFs {
    pub fn connection(&self) -> &SshConnection {
        &self.conn
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
        let (raw, version) = self.conn.raw_sftp().await.map_err(verr)?;
        let offered = version.extensions.get(POSIX_RENAME).is_some_and(|v| v == "1");
        let flag = if offered { POSIX_RENAME_YES } else { POSIX_RENAME_NO };
        // Keep an explicit "disabled" (set meanwhile) as it is.
        let _ = self.posix_rename.compare_exchange(POSIX_RENAME_UNKNOWN, flag, Ordering::Relaxed, Ordering::Relaxed);
        if !offered {
            let _ = raw.close_session();
            return Ok(false);
        }
        // Same wire format as hardlink@openssh.com: string oldpath, string newpath.
        let data: Vec<u8> = russh_sftp::extensions::HardlinkExtension { oldpath: from.into(), newpath: to.into() }
            .try_into()
            .map_err(verr)?;
        let reply = raw.extended(POSIX_RENAME, data).await;
        let _ = raw.close_session();
        match reply {
            Ok(Packet::Status(st)) if st.status_code == StatusCode::Ok => Ok(true),
            Ok(Packet::Status(st)) => Err(VfsError::raw(format!("{}: {}", st.status_code, st.error_message))),
            Ok(_) => Err(VfsError::Backend(txt!("{name}: unexpected answer from the server", name = POSIX_RENAME))),
            Err(russh_sftp::client::error::Error::Status(st)) => {
                Err(VfsError::raw(format!("{}: {}", st.status_code, st.error_message)))
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
        let rd = self.sftp.read_dir(path).await.map_err(link::classify_sftp)?;
        let mut out = Vec::new();
        for de in rd {
            let name = de.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let meta = de.metadata();
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
        let mut flags = OpenFlags::CREATE | OpenFlags::WRITE;
        if offset == 0 {
            flags |= OpenFlags::TRUNCATE;
        }
        let mut f = self.sftp.open_with_flags(path, flags).await.map_err(link::classify_sftp)?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
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
}
