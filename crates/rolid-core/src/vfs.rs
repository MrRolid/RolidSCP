//! Virtual filesystem trait implemented by local and remote backends.

use std::time::SystemTime;

use async_trait::async_trait;
use rolid_text::{txt, Text};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid file name: {0:?}")]
    InvalidName(String),
    #[error("{0}")]
    Backend(Text),
    /// The connection to the server is gone (not a problem of the file itself).
    #[error("{0}")]
    ConnectionLost(Text),
}

pub type VfsResult<T> = Result<T, VfsError>;

impl VfsError {
    /// A backend error whose text comes from elsewhere (a server, a library)
    /// and is shown as it is.
    pub fn raw(e: impl std::fmt::Display) -> Self {
        VfsError::Backend(raw_text(e))
    }

    /// The operation was canceled by the user.
    pub fn canceled() -> Self {
        VfsError::Backend(txt!("canceled by the user"))
    }

    /// The error made by [`VfsError::canceled`].
    pub fn is_canceled(&self) -> bool {
        matches!(self, VfsError::Backend(t) if t.id == "canceled by the user")
    }
    /// The message for the user (English template, translated by the UI).
    /// An I/O error carries the system's own text.
    pub fn text(&self) -> Text {
        match self {
            // A backend may wrap its own VfsError in an io::Error (readers/writers).
            VfsError::Io(e) => match e.get_ref().and_then(|i| i.downcast_ref::<VfsError>()) {
                Some(inner) => inner.text(),
                None => raw_text(e),
            },
            VfsError::InvalidName(name) => txt!("Invalid file name: {name}", name = format!("{name:?}")),
            VfsError::Backend(t) | VfsError::ConnectionLost(t) => t.clone(),
        }
    }
    /// The file or folder does not exist (locally or on the server).
    pub fn is_not_found(&self) -> bool {
        matches!(self, VfsError::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
    }
    /// The operation failed because the connection was lost (network failure,
    /// server restart…), not because of the file: it can be repeated once the
    /// connection is back.
    pub fn is_connection_lost(&self) -> bool {
        match self {
            VfsError::ConnectionLost(_) => true,
            VfsError::Io(e) => io_is_connection_lost(e),
            _ => false,
        }
    }
}

/// A message that is not translated: the text of a system / server / library
/// error, shown as it is (the template is just `{error}`).
pub fn raw_text(e: impl std::fmt::Display) -> Text {
    txt!("{error}", error = e.to_string())
}

/// I/O error kinds that mean a broken connection.
pub fn io_is_connection_lost(e: &std::io::Error) -> bool {
    use std::io::ErrorKind as K;
    matches!(e.kind(), K::ConnectionAborted | K::ConnectionReset | K::BrokenPipe | K::NotConnected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// For symlinks: whether the target is a directory.
    pub link_to_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, if known.
    pub mode: Option<u32>,
}

impl Entry {
    /// True if navigating into this entry makes sense.
    pub fn is_dir_like(&self) -> bool {
        self.kind == EntryKind::Dir || (self.kind == EntryKind::Symlink && self.link_to_dir)
    }
}

pub type Reader = Box<dyn AsyncRead + Send + Unpin>;
pub type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// Disk space of a filesystem, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    /// Free for an ordinary user.
    pub avail: u64,
    pub total: u64,
}

/// A filesystem the panels can browse. Paths are backend-native strings;
/// use `join`/`parent` from the backend to manipulate them.
#[async_trait]
pub trait Vfs: Send + Sync {
    /// Short label, e.g. `local` or `user@host`.
    fn label(&self) -> String;
    fn is_remote(&self) -> bool;
    /// Paths are real paths on this computer's disks (archives can be read and
    /// written directly, without a temporary copy).
    fn is_local_disk(&self) -> bool {
        false
    }
    /// Nothing can be written here (the inside of an archive).
    fn read_only(&self) -> bool {
        false
    }

    fn join(&self, dir: &str, name: &str) -> String;
    /// Parent directory, or `None` at the root.
    fn parent(&self, path: &str) -> Option<String>;

    async fn home(&self) -> VfsResult<String>;
    async fn canonicalize(&self, path: &str) -> VfsResult<String>;
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>>;
    async fn stat(&self, path: &str) -> VfsResult<Entry>;
    async fn mkdir(&self, path: &str) -> VfsResult<()>;
    async fn remove_file(&self, path: &str) -> VfsResult<()>;
    async fn remove_dir(&self, path: &str) -> VfsResult<()>;
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()>;
    /// Rename `from` onto `to` in one atomic step, replacing an existing `to`.
    /// `Ok(false)` = this backend (or server) cannot do that; nothing was changed.
    async fn rename_replace(&self, _from: &str, _to: &str) -> VfsResult<bool> {
        Ok(false)
    }
    /// Open a file for reading, positioned at `offset`.
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader>;
    /// Open (create) a file for writing at `offset`. `offset == 0` truncates;
    /// otherwise existing content up to `offset` is kept (resume).
    async fn open_write_at(&self, path: &str, offset: u64) -> VfsResult<Writer>;
    /// Set permission bits and/or modification time (best effort).
    async fn set_attrs(
        &self,
        path: &str,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
    ) -> VfsResult<()>;

    async fn open_read(&self, path: &str) -> VfsResult<Reader> {
        self.open_read_at(path, 0).await
    }
    /// Free and total space of the filesystem holding `path`.
    /// `Ok(None)` = this backend (or server) cannot tell.
    async fn space(&self, _path: &str) -> VfsResult<Option<Space>> {
        Ok(None)
    }
    async fn open_write(&self, path: &str) -> VfsResult<Writer> {
        self.open_write_at(path, 0).await
    }
    /// Reading at any position (the viewer), when the backend has its own way;
    /// `Ok(None)` = use [`open_random`] (reopening at the position).
    async fn open_random_native(&self, _path: &str) -> VfsResult<Option<Box<dyn RandomFile>>> {
        Ok(None)
    }
}

/// A file read at any position (the viewer reads blocks of big files).
#[async_trait]
pub trait RandomFile: Send {
    /// Up to `len` bytes at `offset`; fewer only at the end of the file.
    async fn read_at(&mut self, offset: u64, len: usize) -> VfsResult<Vec<u8>>;
}

/// Open `path` of `fs` for reading at any position: the backend's own way, or
/// a stream reopened whenever the position jumps (continuing sequential
/// reads on the open one).
pub async fn open_random(fs: &std::sync::Arc<dyn Vfs>, path: &str) -> VfsResult<Box<dyn RandomFile>> {
    if let Some(f) = fs.open_random_native(path).await? {
        return Ok(f);
    }
    let first = fs.open_read_at(path, 0).await?;
    Ok(Box::new(Reopen { fs: fs.clone(), path: path.to_string(), cur: Some((0, first)) }))
}

struct Reopen {
    fs: std::sync::Arc<dyn Vfs>,
    path: String,
    /// The open stream and its position.
    cur: Option<(u64, Reader)>,
}

#[async_trait]
impl RandomFile for Reopen {
    async fn read_at(&mut self, offset: u64, len: usize) -> VfsResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let mut r = match self.cur.take() {
            Some((pos, r)) if pos == offset => r,
            _ => self.fs.open_read_at(&self.path, offset).await?,
        };
        let mut buf = vec![0u8; len];
        let mut got = 0;
        while got < len {
            let n = r.read(&mut buf[got..]).await?;
            if n == 0 {
                break;
            }
            got += n;
        }
        buf.truncate(got);
        if got == len {
            self.cur = Some((offset + got as u64, r));
        }
        Ok(buf)
    }
}

/// Reject names coming from listings that could escape the target directory
/// (path traversal) when used as a destination name.
pub fn validate_name(name: &str) -> VfsResult<()> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || (cfg!(windows) && name.contains(':'));
    if bad {
        Err(VfsError::InvalidName(name.to_owned()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_loss_is_told_apart_from_file_errors() {
        use std::io::{Error, ErrorKind};
        assert!(VfsError::ConnectionLost(Text::new("x")).is_connection_lost());
        for k in [ErrorKind::ConnectionAborted, ErrorKind::ConnectionReset, ErrorKind::BrokenPipe, ErrorKind::NotConnected] {
            assert!(VfsError::Io(Error::new(k, "x")).is_connection_lost(), "{k:?}");
        }
        for k in [ErrorKind::NotFound, ErrorKind::PermissionDenied, ErrorKind::Other, ErrorKind::StorageFull] {
            assert!(!VfsError::Io(Error::new(k, "x")).is_connection_lost(), "{k:?}");
        }
        assert!(!VfsError::Backend(Text::new("Permission denied")).is_connection_lost());
        assert!(!VfsError::InvalidName("..".into()).is_connection_lost());
    }

    #[test]
    fn rejects_traversal() {
        for n in ["", ".", "..", "../x", "a/b", "a\\b", "x\0"] {
            assert!(validate_name(n).is_err(), "{n:?}");
        }
        assert!(validate_name("..hidden").is_ok());
        assert!(validate_name("file.txt").is_ok());
    }
}
