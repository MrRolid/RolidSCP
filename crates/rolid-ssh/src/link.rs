//! Connection loss detection and automatic reconnection.
//!
//! A [`Link`] is the shared connection to one server: all panels and tabs on that
//! server use it through their own [`RemoteFs`] (each with its own SFTP channel).
//! When the connection breaks (keepalive unanswered, closed channel, EOF,
//! timeout), the link goes to [`LinkState::Reconnecting`]; every operation then
//! fails at once with [`VfsError::ConnectionLost`] instead of hanging.
//! [`drive_reconnect`] tries to connect again with growing pauses
//! ([`backoff`]: 1, 2, 4, 8, 16, then 30 s), without limit, until it succeeds,
//! needs the user (locked passwords, unknown question, host key) or is stopped.
//! After success every `RemoteFs` opens a fresh SFTP channel on the new
//! connection with its next operation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use rolid_core::vfs::{io_is_connection_lost, raw_text, Reader, Writer};
use rolid_text::{txt, Text};
use rolid_core::{Entry, Vfs, VfsError, VfsResult};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{SftpFs, SshConnection};

/// SSH keepalive: a probe every 15 s…
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
/// …and the connection counts as dead after 3 unanswered probes.
pub const KEEPALIVE_MAX: usize = 3;

/// Pauses before reconnection attempts (seconds); the last one repeats.
pub const BACKOFF_SECS: [u64; 6] = [1, 2, 4, 8, 16, 30];

/// Pause before reconnection attempt `attempt` (1-based).
pub fn backoff(attempt: u32) -> Duration {
    let i = (attempt.max(1) as usize - 1).min(BACKOFF_SECS.len() - 1);
    Duration::from_secs(BACKOFF_SECS[i])
}

/// Does an error message (from the SFTP / SSH layer, not a server status reply)
/// mean the connection is gone?
pub fn message_means_lost(msg: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "session closed",
        "channel closed",
        "connection closed",
        "sender dropped",
        "recverror",
        "senderror",
        "timeout",
        "timed out",
        "connection reset",
        "connection aborted",
        "broken pipe",
        "not connected",
        "disconnected",
        "unexpected eof",
        "early eof",
    ];
    let m = msg.to_ascii_lowercase();
    m == "eof" || PATTERNS.iter().any(|p| m.contains(p))
}

/// The text of a status reply: the server's message, with the name of the
/// code only when the message says something else ("Permission denied", not
/// "Permission denied: Permission denied" as OpenSSH answers).
pub fn status_text(st: &russh_sftp::protocol::Status) -> String {
    let code = st.status_code.to_string();
    let msg = st.error_message.trim();
    if msg.is_empty() || msg.eq_ignore_ascii_case(&code) {
        code
    } else {
        format!("{code}: {msg}")
    }
}

/// Classify an SFTP client error: a status reply from the server (no such file,
/// permission denied…) is a file error; everything about the transport is a lost
/// connection.
pub fn classify_sftp(e: russh_sftp::client::error::Error) -> VfsError {
    use russh_sftp::client::error::Error as E;
    match e {
        // "No such file" keeps its kind (the same text), so callers can tell it apart.
        E::Status(st) if st.status_code == russh_sftp::protocol::StatusCode::NoSuchFile => {
            VfsError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, status_text(&st)))
        }
        E::Status(st) => VfsError::raw(status_text(&st)),
        E::Timeout => VfsError::ConnectionLost(txt!("the server does not respond (timeout)")),
        E::IO(m) => VfsError::ConnectionLost(txt!("connection interrupted: {error}", error = m)),
        E::UnexpectedBehavior(m) if message_means_lost(&m) => {
            VfsError::ConnectionLost(txt!("connection interrupted: {error}", error = m))
        }
        other => VfsError::raw(other),
    }
}

/// Why reconnecting waits for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedUser {
    /// The saved password is locked (master password).
    Unlock,
    /// The server wants something we cannot answer (password, unknown question).
    Credentials,
    /// Host key unknown or changed – never accepted automatically.
    HostKey,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    Up,
    /// Lost; trying to reconnect. `attempt` = attempts made so far; `next_at` =
    /// when the next one starts (`None` = one is running now).
    Reconnecting { attempt: u32, next_at: Option<Instant>, error: Text },
    /// Reconnecting needs the user (no dialog pops up by itself).
    NeedsUser { reason: NeedUser, error: Text },
    /// The user stopped the attempts.
    Stopped { error: Text },
}

impl LinkState {
    pub fn is_up(&self) -> bool {
        matches!(self, LinkState::Up)
    }

    /// Short description for panels and tabs.
    pub fn text(&self) -> Text {
        match self {
            LinkState::Up => txt!("connected"),
            LinkState::Reconnecting { attempt, next_at: None, .. } => {
                txt!("Reconnecting (attempt {attempt})…", attempt = *attempt.max(&1))
            }
            LinkState::Reconnecting { attempt, next_at: Some(t), .. } => {
                let s = t.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u64;
                txt!("Connection lost – reconnecting (attempt {attempt} in {secs} s)", attempt = attempt + 1, secs = s)
            }
            LinkState::NeedsUser { reason: NeedUser::Unlock, .. } => {
                txt!("Unlock the saved passwords to restore the connection")
            }
            LinkState::NeedsUser { reason: NeedUser::Credentials, .. } => {
                txt!("Enter the login details to restore the connection")
            }
            LinkState::NeedsUser { reason: NeedUser::HostKey, .. } => {
                txt!("The connection cannot be restored: the server key needs to be verified")
            }
            LinkState::Stopped { .. } => txt!("Disconnected (reconnecting stopped)"),
        }
    }

    pub fn error(&self) -> Option<&Text> {
        match self {
            LinkState::Up => None,
            LinkState::Reconnecting { error, .. } | LinkState::NeedsUser { error, .. } | LinkState::Stopped { error } => {
                Some(error)
            }
        }
    }
}

struct Inner {
    conn: Option<SshConnection>,
    generation: u64,
    state: LinkState,
    driver: bool,
}

/// The shared connection to one server (see the module docs).
pub struct Link {
    pub label: String,
    /// `user@host:port` (see [`SshConnection::key`]).
    pub key: String,
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
}

impl Link {
    pub fn new(conn: SshConnection) -> Arc<Link> {
        Arc::new(Link {
            label: conn.label.clone(),
            key: conn.key.clone(),
            inner: Mutex::new(Inner { conn: Some(conn), generation: 1, state: LinkState::Up, driver: false }),
            wake: tokio::sync::Notify::new(),
        })
    }

    /// A link without a connection yet, waiting for reconnection (tests).
    pub fn new_lost(label: &str, error: &str) -> Arc<Link> {
        Arc::new(Link {
            label: label.into(),
            key: label.into(),
            inner: Mutex::new(Inner {
                conn: None,
                generation: 0,
                state: LinkState::Reconnecting { attempt: 0, next_at: None, error: raw_text(error) },
                driver: false,
            }),
            wake: tokio::sync::Notify::new(),
        })
    }

    pub fn state(&self) -> LinkState {
        self.inner.lock().unwrap().state.clone()
    }

    /// Incremented by every new connection.
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }

    /// The connection, if up.
    pub fn current(&self) -> Option<(u64, SshConnection)> {
        let g = self.inner.lock().unwrap();
        match (&g.state, &g.conn) {
            (LinkState::Up, Some(c)) => Some((g.generation, c.clone())),
            _ => None,
        }
    }

    /// The connection of generation `generation` broke. Returns true if this
    /// changed the state (the first report of the loss).
    pub fn mark_lost(&self, generation: u64, error: Text) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.generation != generation || !g.state.is_up() {
            return false;
        }
        tracing::warn!("connection {} lost: {error}", self.label);
        g.state = LinkState::Reconnecting { attempt: 0, next_at: None, error };
        g.conn = None; // dead anyway; panels open new channels after reconnection
        true
    }

    /// Notice a connection closed by the SSH layer (keepalive, server gone) even
    /// when nothing is being done. Cheap; call it regularly.
    pub fn check(&self) -> bool {
        let closed = {
            let g = self.inner.lock().unwrap();
            match (&g.state, &g.conn) {
                (LinkState::Up, Some(c)) if c.is_closed() => Some(g.generation),
                _ => None,
            }
        };
        match closed {
            Some(gen) => self.mark_lost(gen, txt!("the connection to the server was interrupted")),
            None => false,
        }
    }

    /// Install a new connection (reconnected, or connected by the user).
    pub fn restore(&self, conn: SshConnection) {
        let mut g = self.inner.lock().unwrap();
        g.conn = Some(conn);
        g.generation += 1;
        g.state = LinkState::Up;
        tracing::info!("connection {} restored", self.label);
        drop(g);
        self.wake.notify_waiters();
    }

    /// "Connect now": try at once (also after "Cancel" or when the user is needed).
    /// Returns true if a reconnect driver should be started.
    pub fn connect_now(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        match &g.state {
            LinkState::Up => return false,
            LinkState::Reconnecting { .. } => {}
            LinkState::NeedsUser { error, .. } | LinkState::Stopped { error } => {
                g.state = LinkState::Reconnecting { attempt: 0, next_at: None, error: error.clone() };
            }
        }
        let start = !g.driver;
        drop(g);
        self.wake.notify_one();
        start
    }

    /// "Cancel": stop reconnecting.
    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap();
        if let Some(error) = g.state.error().cloned() {
            g.state = LinkState::Stopped { error };
        }
        drop(g);
        self.wake.notify_waiters();
        self.wake.notify_one();
    }

    /// Should a reconnect driver be started now? (reconnecting, none running)
    /// Marks it as running when true.
    pub fn claim_driver(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.state, LinkState::Reconnecting { .. }) && !g.driver {
            g.driver = true;
            true
        } else {
            false
        }
    }

    fn set_state(&self, s: LinkState) -> bool {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.state, LinkState::Stopped { .. } | LinkState::Up) {
            return false; // stopped or restored meanwhile
        }
        g.state = s;
        true
    }

    fn release_driver(&self) {
        self.inner.lock().unwrap().driver = false;
    }

    /// Wait until the link is up again (true), or stopped / `cancel` set (false).
    /// While the user is needed it keeps waiting.
    pub async fn wait_up(&self, cancel: &AtomicBool) -> bool {
        loop {
            match self.state() {
                LinkState::Up => return true,
                LinkState::Stopped { .. } => return false,
                _ => {}
            }
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Result of one reconnection attempt.
pub enum Attempt {
    Connected(SshConnection),
    /// Network problem: try again later.
    Retry(Text),
    NeedsUser(NeedUser, Text),
}

/// Reconnect `link` (after [`Link::claim_driver`] returned true): attempt,
/// wait [`backoff`], attempt again… until connected, the user is needed or the
/// attempts are stopped. `connect(n)` makes attempt `n`.
pub async fn drive_reconnect<F, Fut>(link: Arc<Link>, mut connect: F)
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Attempt>,
{
    let mut attempt = 0u32;
    loop {
        // Wait before the attempt (interrupted by "Connect now" / "Cancel").
        let pause = backoff(attempt + 1);
        let error = link.state().error().cloned().unwrap_or_else(|| raw_text(""));
        if !link.set_state(LinkState::Reconnecting {
            attempt,
            next_at: Some(Instant::now() + pause),
            error: error.clone(),
        }) {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = link.wake.notified() => {}
        }
        attempt += 1;
        if !link.set_state(LinkState::Reconnecting { attempt, next_at: None, error }) {
            break;
        }
        match connect(attempt).await {
            Attempt::Connected(c) => {
                if !matches!(link.state(), LinkState::Stopped { .. }) {
                    link.restore(c);
                }
                break;
            }
            Attempt::Retry(e) => {
                tracing::info!("reconnecting {}: attempt {attempt} failed: {e}", link.label);
                link.set_state(LinkState::Reconnecting { attempt, next_at: None, error: e });
            }
            Attempt::NeedsUser(reason, error) => {
                link.set_state(LinkState::NeedsUser { reason, error });
                break;
            }
        }
    }
    link.release_driver();
}

// ------------------------------------------------------------------ RemoteFs

fn lost_now(link: &Link) -> VfsError {
    let s = link.state();
    VfsError::ConnectionLost(match s.error() {
        Some(e) => txt!(
            "The connection to server {server} is interrupted ({error}). {state}",
            server = &link.label,
            error = e,
            state = s.text()
        ),
        None => txt!("The connection to server {server} is interrupted. {state}", server = &link.label, state = s.text()),
    })
}

/// Remote filesystem on a [`Link`]: survives reconnection (opens a new SFTP
/// channel on the new connection with the next operation).
pub struct RemoteFs {
    link: Arc<Link>,
    session: tokio::sync::Mutex<Option<(u64, Arc<SftpFs>)>>,
}

impl RemoteFs {
    pub fn new(link: Arc<Link>) -> Self {
        Self { link, session: tokio::sync::Mutex::new(None) }
    }

    /// With an already open SFTP session on the link's current connection.
    pub fn with_session(link: Arc<Link>, sftp: SftpFs) -> Self {
        let gen = link.generation();
        Self { link, session: tokio::sync::Mutex::new(Some((gen, Arc::new(sftp)))) }
    }

    pub fn link(&self) -> &Arc<Link> {
        &self.link
    }

    async fn sftp(&self) -> VfsResult<(u64, Arc<SftpFs>)> {
        let Some((gen, conn)) = self.link.current() else {
            return Err(lost_now(&self.link));
        };
        let mut g = self.session.lock().await;
        if let Some((g0, s)) = g.as_ref() {
            if *g0 == gen {
                return Ok((gen, s.clone()));
            }
        }
        *g = None;
        match conn.sftp().await {
            Ok(s) => {
                let s = Arc::new(s);
                *g = Some((gen, s.clone()));
                Ok((gen, s))
            }
            Err(e) => {
                let msg = e.to_string();
                if conn.is_closed() || message_means_lost(&msg) {
                    self.link.mark_lost(gen, e.text());
                    Err(lost_now(&self.link))
                } else {
                    Err(VfsError::Backend(e.text()))
                }
            }
        }
    }

    /// Turn an error into [`VfsError::ConnectionLost`] if the connection is gone
    /// (and report the loss to the link).
    fn check(&self, gen: u64, conn_closed: bool, e: VfsError) -> VfsError {
        if e.is_connection_lost() || conn_closed {
            self.link.mark_lost(gen, e.text());
            return match e {
                VfsError::ConnectionLost(_) => e,
                other => VfsError::ConnectionLost(txt!("connection interrupted: {error}", error = other.text())),
            };
        }
        e
    }

    async fn run<T, F, Fut>(&self, f: F) -> VfsResult<T>
    where
        F: FnOnce(Arc<SftpFs>) -> Fut,
        Fut: std::future::Future<Output = VfsResult<T>>,
    {
        let (gen, s) = self.sftp().await?;
        // An SFTP request sent just as the connection ends may never be woken
        // (russh-sftp clears its waiting requests when the transport stops, and
        // one registered right after that waits for its 30 s timeout): when the
        // connection is gone, the operation fails at once.
        let conn = s.connection().clone();
        let r = tokio::select! {
            biased;
            r = f(s.clone()) => r,
            _ = conn.closed() => Err(VfsError::ConnectionLost(txt!("the connection to the server was interrupted"))),
        };
        match r {
            Ok(v) => Ok(v),
            Err(e) => Err(self.check(gen, s.connection().is_closed(), e)),
        }
    }
}

#[async_trait]
impl Vfs for RemoteFs {
    fn label(&self) -> String {
        self.link.label.clone()
    }
    fn journal_key(&self) -> Option<String> {
        Some(self.link.key.clone())
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn join(&self, dir: &str, name: &str) -> String {
        crate::posix_join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        crate::posix_parent(path)
    }
    async fn home(&self) -> VfsResult<String> {
        self.run(|s| async move { s.home().await }).await
    }
    async fn canonicalize(&self, path: &str) -> VfsResult<String> {
        self.run(|s| async move { s.canonicalize(path).await }).await
    }
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>> {
        self.run(|s| async move { s.list(path).await }).await
    }
    async fn stat(&self, path: &str) -> VfsResult<Entry> {
        self.run(|s| async move { s.stat(path).await }).await
    }
    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.run(|s| async move { s.mkdir(path).await }).await
    }
    async fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.run(|s| async move { s.remove_file(path).await }).await
    }
    async fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.run(|s| async move { s.remove_dir(path).await }).await
    }
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.run(|s| async move { s.rename(from, to).await }).await
    }
    async fn rename_replace(&self, from: &str, to: &str) -> VfsResult<bool> {
        self.run(|s| async move { s.rename_replace(from, to).await }).await
    }
    async fn rename_noreplace(&self, from: &str, to: &str) -> VfsResult<bool> {
        self.run(|s| async move { s.rename_noreplace(from, to).await }).await
    }
    async fn space(&self, path: &str) -> VfsResult<Option<rolid_core::vfs::Space>> {
        self.run(|s| async move { s.space(path).await }).await
    }
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader> {
        let (gen, s) = self.sftp().await?;
        match s.open_read_at(path, offset).await {
            Ok(r) => Ok(Box::new(Guarded { inner: r, link: self.link.clone(), gen, sftp: s })),
            Err(e) => Err(self.check(gen, s.connection().is_closed(), e)),
        }
    }
    async fn open_write_at(&self, path: &str, offset: u64) -> VfsResult<Writer> {
        let (gen, s) = self.sftp().await?;
        match s.open_write_at(path, offset).await {
            Ok(w) => Ok(Box::new(Guarded { inner: w, link: self.link.clone(), gen, sftp: s })),
            Err(e) => Err(self.check(gen, s.connection().is_closed(), e)),
        }
    }
    async fn set_attrs(&self, path: &str, mode: Option<u32>, mtime: Option<SystemTime>) -> VfsResult<()> {
        self.run(|s| async move { s.set_attrs(path, mode, mtime).await }).await
    }
}

/// Reader / writer that reports a lost connection (and says so in the error kind,
/// so copy jobs can continue after reconnection).
struct Guarded<T> {
    inner: T,
    link: Arc<Link>,
    gen: u64,
    sftp: Arc<SftpFs>,
}

impl<T> Guarded<T> {
    fn map(&self, e: std::io::Error) -> std::io::Error {
        let lost = io_is_connection_lost(&e)
            || e.kind() == std::io::ErrorKind::TimedOut
            || self.sftp.connection().is_closed()
            || message_means_lost(&e.to_string());
        if !lost {
            return e;
        }
        self.link.mark_lost(self.gen, raw_text(&e));
        // The VfsError inside carries the message for the user (VfsError::text).
        std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            VfsError::ConnectionLost(txt!("the connection to the server was interrupted: {error}", error = e.to_string())),
        )
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Guarded<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, buf) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(self.map(e))),
            other => other,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Guarded<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match std::pin::Pin::new(&mut self.inner).poll_write(cx, buf) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(self.map(e))),
            other => other,
        }
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match std::pin::Pin::new(&mut self.inner).poll_flush(cx) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(self.map(e))),
            other => other,
        }
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match std::pin::Pin::new(&mut self.inner).poll_shutdown(cx) {
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(self.map(e))),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_sftp::client::error::Error as E;
    use russh_sftp::protocol::{Status, StatusCode};

    #[test]
    fn backoff_sequence() {
        let secs: Vec<u64> = (1..=9).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30, 30]);
        assert_eq!(backoff(0).as_secs(), 1);
        assert_eq!(backoff(u32::MAX).as_secs(), 30);
    }

    fn status(code: StatusCode, msg: &str) -> E {
        E::Status(Status { id: 1, status_code: code, error_message: msg.into(), language_tag: "en".into() })
    }

    #[test]
    fn transport_errors_are_connection_loss() {
        for e in [
            E::Timeout,
            E::IO("Broken pipe (os error 32)".into()),
            E::UnexpectedBehavior("session closed".into()),
            E::UnexpectedBehavior("sender dropped".into()),
            E::UnexpectedBehavior("RecvError: channel closed".into()),
            E::UnexpectedBehavior("SendError: channel closed".into()),
        ] {
            let d = format!("{e:?}");
            assert!(classify_sftp(e).is_connection_lost(), "{d}");
        }
        // Answers of a working server are file errors.
        for e in [
            status(StatusCode::NoSuchFile, "No such file"),
            status(StatusCode::PermissionDenied, "Permission denied"),
            status(StatusCode::Failure, "Failure"),
            status(StatusCode::ConnectionLost, "the server says so"),
            E::UnexpectedPacket,
            E::Limited("packet exceeds server limit".into()),
            E::UnexpectedBehavior("Duplicate version".into()),
        ] {
            let d = format!("{e:?}");
            assert!(!classify_sftp(e).is_connection_lost(), "{d}");
        }
        assert!(classify_sftp(status(StatusCode::NoSuchFile, "No such file")).is_not_found());
        assert!(!classify_sftp(status(StatusCode::PermissionDenied, "Permission denied")).is_not_found());
    }

    /// OpenSSH sends the name of the code as the message: it is said once
    /// (an F4 upload into a read-only folder said "Permission denied: Permission denied").
    #[test]
    fn a_status_is_not_said_twice() {
        let text = |e| classify_sftp(e).text().to_string();
        assert_eq!(text(status(StatusCode::PermissionDenied, "Permission denied")), "Permission denied");
        assert_eq!(text(status(StatusCode::NoSuchFile, "No such file")), "No such file");
        assert_eq!(text(status(StatusCode::Failure, "")), "Failure");
        assert_eq!(text(status(StatusCode::Failure, "disk quota exceeded")), "Failure: disk quota exceeded");
    }

    #[test]
    fn messages() {
        for m in ["session closed", "Timeout", "early eof", "EOF", "Connection reset by peer", "channel closed"] {
            assert!(message_means_lost(m), "{m}");
        }
        for m in ["No such file", "Permission denied", "Failure", "neofetch.conf"] {
            assert!(!message_means_lost(m), "{m}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn driver_backs_off_needs_user_and_stops() {
        // Network down three times, then the server wants a password.
        let link = Link::new_lost("u@h", "lost");
        assert!(link.claim_driver());
        assert!(!link.claim_driver(), "only one driver");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        let started = tokio::time::Instant::now();
        drive_reconnect(link.clone(), move |n| {
            c.lock().unwrap().push((n, started.elapsed().as_secs()));
            async move {
                if n <= 3 {
                    Attempt::Retry(raw_text("Connection refused"))
                } else {
                    Attempt::NeedsUser(NeedUser::Credentials, raw_text("password"))
                }
            }
        })
        .await;
        // Waits 1, 2, 4, 8 s before attempts 1–4.
        assert_eq!(*calls.lock().unwrap(), [(1, 1), (2, 3), (3, 7), (4, 15)]);
        assert_eq!(link.state(), LinkState::NeedsUser { reason: NeedUser::Credentials, error: raw_text("password") });
        assert!(link.state().text().to_string().contains("login details"));

        // "Connect now" restarts at once; "Cancel" stops during the pause.
        assert!(link.connect_now());
        assert!(link.claim_driver());
        let l2 = link.clone();
        let h = tokio::spawn(drive_reconnect(link.clone(), |_| async { Attempt::Retry(raw_text("refused")) }));
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(matches!(l2.state(), LinkState::Reconnecting { attempt: 1, next_at: Some(_), .. }));
        l2.stop();
        h.await.unwrap();
        assert!(matches!(link.state(), LinkState::Stopped { .. }));
        assert!(!link.claim_driver(), "stopped: no new driver");
        assert!(!link.wait_up(&AtomicBool::new(false)).await, "jobs give up when stopped");
    }

    #[tokio::test(start_paused = true)]
    async fn connect_now_skips_the_pause() {
        let link = Link::new_lost("u@h", "x");
        assert!(link.claim_driver());
        let started = tokio::time::Instant::now();
        let l = link.clone();
        let h = tokio::spawn(drive_reconnect(link.clone(), move |n| {
            let t = started.elapsed();
            async move {
                if n == 1 {
                    Attempt::Retry(raw_text(t.as_millis()))
                } else {
                    Attempt::NeedsUser(NeedUser::Unlock, raw_text(t.as_millis()))
                }
            }
        }));
        tokio::time::sleep(Duration::from_millis(1100)).await; // attempt 1 done, now waiting 2 s
        assert!(!l.connect_now(), "driver already running: just wake it");
        h.await.unwrap();
        // Attempt 2 started right away, not after the 2 s pause.
        match link.state() {
            LinkState::NeedsUser { error, .. } => assert!(error.to_string().parse::<u64>().unwrap() < 1500, "{error}"),
            s => panic!("{s:?}"),
        }
    }
}
