//! Writing a file to the server with many writes on the way, knowing how far
//! the server confirmed it.
//!
//! russh-sftp's own file writer keeps up to 64 writes on the way but does not
//! say which of them the server already confirmed. After a broken connection
//! the transfer then had to write the last 4 MB of the part file again (a
//! server may apply writes out of order and leave a hole before its end). On
//! a line that is up only for seconds between outages that ate everything it
//! moved. [`AckWriter`] counts the confirmed bytes (in order: everything
//! before the count is on the server), so a continuation writes again only
//! what was not confirmed.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::RawSftpSession;
use tokio::io::AsyncWrite;

type Pending = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

/// Bytes in one write request: with the request's own 25 bytes and the handle
/// it fits one SSH packet of 32 kB (what every server accepts).
fn chunk_len(handle: &str) -> usize {
    32 * 1024 - 25 - handle.len()
}

fn sftp_io(e: SftpError) -> io::Error {
    match e {
        // A refusal of the server (disk full, permission): its own message.
        SftpError::Status(st) => io::Error::other(st.error_message),
        e => e.into(),
    }
}

/// Writer of one open remote file (see the module docs).
pub(crate) struct AckWriter {
    raw: Arc<RawSftpSession>,
    handle: String,
    /// Where the next write goes.
    pos: u64,
    /// Writes on the way: (end of the write, its answer), oldest first.
    acks: VecDeque<(u64, Pending)>,
    max_in_flight: usize,
    /// Everything before this is confirmed by the server.
    confirmed: Arc<AtomicU64>,
    /// A write was refused or lost: nothing more is written.
    broken: bool,
    closing: Option<Pending>,
    closed: bool,
}

impl AckWriter {
    /// `handle` is open for writing on `raw`; writing starts at `offset`
    /// (`confirmed` is set to it).
    pub(crate) fn new(raw: Arc<RawSftpSession>, handle: String, offset: u64, max_in_flight: usize, confirmed: Arc<AtomicU64>) -> Self {
        confirmed.store(offset, Ordering::Relaxed);
        Self {
            raw,
            handle,
            pos: offset,
            acks: VecDeque::with_capacity(max_in_flight),
            max_in_flight: max_in_flight.max(1),
            confirmed,
            broken: false,
            closing: None,
            closed: false,
        }
    }

    fn broken_error() -> io::Error {
        io::Error::other("an earlier write to the server failed")
    }

    /// The answer to the oldest write: `Ready(None)` when none is on the way.
    fn poll_oldest(&mut self, cx: &mut Context<'_>) -> Poll<Option<io::Result<()>>> {
        let Some((end, answer)) = self.acks.front_mut() else {
            return Poll::Ready(None);
        };
        let r = ready!(answer.as_mut().poll(cx));
        let end = *end;
        self.acks.pop_front();
        match r {
            Ok(()) => {
                self.confirmed.fetch_max(end, Ordering::Relaxed);
                Poll::Ready(Some(Ok(())))
            }
            Err(e) => {
                self.broken = true;
                self.acks.clear();
                Poll::Ready(Some(Err(e)))
            }
        }
    }

    fn poll_all(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.broken {
            return Poll::Ready(Err(Self::broken_error()));
        }
        while let Some(r) = ready!(self.poll_oldest(cx)) {
            r?;
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for AckWriter {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let me = self.get_mut();
        if me.broken || me.closed || me.closing.is_some() {
            return Poll::Ready(Err(Self::broken_error()));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        // Collect the answers that came; wait while the pipeline is full.
        loop {
            match me.poll_oldest(cx) {
                Poll::Ready(Some(Ok(()))) => continue,
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(None) => break,
                Poll::Pending if me.acks.len() >= me.max_in_flight => return Poll::Pending,
                Poll::Pending => break,
            }
        }
        let len = buf.len().min(chunk_len(&me.handle));
        let (raw, handle, offset, data) = (me.raw.clone(), me.handle.clone(), me.pos, buf[..len].to_vec());
        let mut answer: Pending = Box::pin(async move { raw.write(handle, offset, data).await.map(|_| ()).map_err(sftp_io) });
        // The request goes out on the first poll: now, so they leave in order.
        if let Poll::Ready(r) = answer.as_mut().poll(cx) {
            if let Err(e) = r {
                me.broken = true;
                return Poll::Ready(Err(e));
            }
            answer = Box::pin(std::future::ready(Ok(())));
        }
        me.pos += len as u64;
        me.acks.push_back((me.pos, answer));
        Poll::Ready(Ok(len))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().poll_all(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        if me.closed {
            return Poll::Ready(Ok(()));
        }
        if me.closing.is_none() {
            ready!(me.poll_all(cx))?;
            let (raw, handle) = (me.raw.clone(), me.handle.clone());
            me.closing = Some(Box::pin(async move { raw.close(handle).await.map(|_| ()).map_err(sftp_io) }));
        }
        let r = ready!(me.closing.as_mut().expect("closing").as_mut().poll(cx));
        me.closing = None;
        me.closed = true;
        Poll::Ready(r)
    }
}

impl Drop for AckWriter {
    fn drop(&mut self) {
        if self.closed || self.closing.is_some() {
            return;
        }
        // Not closed (an error, a cancel): close the handle in the background.
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let (raw, handle) = (self.raw.clone(), std::mem::take(&mut self.handle));
            rt.spawn(async move {
                let _ = raw.close(handle).await;
            });
        }
    }
}

/// How far the last writer of each file confirmed it (see
/// [`rolid_core::Vfs::write_confirmed`]). Kept for the files written last.
#[derive(Default)]
pub(crate) struct Confirmed {
    files: std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicU64>>>,
}

impl Confirmed {
    /// Most files are finished and never asked about again: past this many,
    /// those without a writer are forgotten.
    const KEEP: usize = 256;

    /// A fresh count for a new writer of `path`.
    pub(crate) fn start(&self, path: &str) -> Arc<AtomicU64> {
        let mut files = self.files.lock().unwrap();
        if files.len() >= Self::KEEP {
            files.retain(|_, c| Arc::strong_count(c) > 1);
        }
        let c = Arc::new(AtomicU64::new(0));
        files.insert(path.to_owned(), c.clone());
        c
    }

    pub(crate) fn get(&self, path: &str) -> Option<u64> {
        self.files.lock().unwrap().get(path).map(|c| c.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_kept_for_recent_files() {
        let c = Confirmed::default();
        let live = c.start("/a.filepart");
        live.store(7, Ordering::Relaxed);
        for i in 0..Confirmed::KEEP + 10 {
            c.start(&format!("/f{i}"));
        }
        assert_eq!(c.get("/a.filepart"), Some(7), "a file still being written is kept");
        assert!(c.files.lock().unwrap().len() <= Confirmed::KEEP + 1);
        assert_eq!(c.get("/nothing"), None);
    }

    #[test]
    fn a_write_fits_one_packet() {
        assert!(chunk_len("0123456789abcdef") + 25 + 16 <= 32 * 1024);
    }
}
