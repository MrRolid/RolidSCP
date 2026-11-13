//! Archives: packing (zip, 7z, tar.gz, tar.zst), listing, safe extraction and
//! integrity tests. Everything here is blocking (run it on a blocking thread)
//! and works on local files only: remote archives are downloaded first.
//!
//! Safety of extraction: every name in an archive goes through [`safe_rel`]
//! (no `..`, no absolute paths, no drive letters); links pointing outside the
//! target are not created, devices and FIFOs are skipped, setuid / setgid bits
//! are dropped and the unpacked size, the compression ratio and the number of
//! items are limited ([`Limits`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use rolid_text::{n, nc, txt, Text};
use secrecy::SecretString;

mod pack;
mod par;
mod read;

pub use pack::{pack, pack_with, PackOpts, PackReport};
pub use read::{cleanup, extract, list, test, ExtractReport, Select, TestReport};

/// Archive formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Zip,
    SevenZ,
    Tar,
    TarGz,
    TarZst,
}

impl Format {
    /// Formats offered for packing, in the order of the dialog.
    pub const PACK: [Format; 4] = [Format::Zip, Format::SevenZ, Format::TarGz, Format::TarZst];

    /// Format by the file name (extension), `None` = not an archive we read.
    pub fn from_name(name: &str) -> Option<Format> {
        let l = name.to_ascii_lowercase();
        let ends = |s: &str| l.len() > s.len() && l.ends_with(s);
        Some(if ends(".zip") {
            Format::Zip
        } else if ends(".7z") {
            Format::SevenZ
        } else if ends(".tar.gz") || ends(".tgz") {
            Format::TarGz
        } else if ends(".tar.zst") || ends(".tzst") {
            Format::TarZst
        } else if ends(".tar") {
            Format::Tar
        } else {
            return None;
        })
    }

    /// Extension including the dot.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Zip => ".zip",
            Format::SevenZ => ".7z",
            Format::Tar => ".tar",
            Format::TarGz => ".tar.gz",
            Format::TarZst => ".tar.zst",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarZst => "tar.zst",
        }
    }

    /// Settings key.
    pub fn key(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarZst => "tar.zst",
        }
    }

    pub fn from_key(k: &str) -> Option<Format> {
        [Format::Zip, Format::SevenZ, Format::Tar, Format::TarGz, Format::TarZst].into_iter().find(|f| f.key() == k)
    }

    /// AES-256 encryption is possible (zip, 7z).
    pub fn can_encrypt(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ)
    }

    /// `name` without this format's extension (case-insensitive).
    pub fn strip_ext(self, name: &str) -> String {
        let exts: &[&str] = match self {
            Format::TarGz => &[".tar.gz", ".tgz"],
            Format::TarZst => &[".tar.zst", ".tzst"],
            f => &[f.ext()],
        };
        let l = name.to_ascii_lowercase();
        for e in exts {
            if l.len() > e.len() && l.ends_with(e) {
                return name[..name.len() - e.len()].to_string();
            }
        }
        name.to_string()
    }
}

/// Name without any archive extension we know.
pub fn strip_archive_ext(name: &str) -> String {
    match Format::from_name(name) {
        Some(f) => f.strip_ext(name),
        None => name.to_string(),
    }
}

/// Compression level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Level {
    /// No compression (zip Stored, 7z Copy).
    Store,
    Fast,
    #[default]
    Normal,
    Max,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Store, Level::Fast, Level::Normal, Level::Max];

    pub fn label(self) -> &'static str {
        match self {
            Level::Store => nc!("compression", "No compression (store)"),
            Level::Fast => nc!("compression", "Fast"),
            Level::Normal => nc!("compression", "Normal"),
            Level::Max => nc!("compression", "Maximum"),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Level::Store => "store",
            Level::Fast => "fast",
            Level::Normal => "normal",
            Level::Max => "max",
        }
    }

    pub fn from_key(k: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.key() == k)
    }
}

impl Format {
    /// Levels offered for this format (tar has none).
    pub fn levels(self) -> &'static [Level] {
        match self {
            Format::Zip | Format::SevenZ => &Level::ALL,
            Format::TarGz | Format::TarZst => &[Level::Fast, Level::Normal, Level::Max],
            Format::Tar => &[],
        }
    }
}

/// The algorithm's own level for a format and level: Deflate 1 / 6 / 9 (zip,
/// tar.gz), LZMA2 preset 0 / 5 / 9 (7z), Zstandard 1 / 3 / 19 (tar.zst).
/// "Fast" is the lowest level that still compresses. `None` = no
/// compression (zip Stored, 7z Copy, tar). Store is not offered for tar.gz /
/// tar.zst; there it falls back to the fast level.
pub fn level_value(format: Format, level: Level) -> Option<i32> {
    use Level::*;
    match (format, level) {
        (Format::Tar, _) | (Format::Zip | Format::SevenZ, Store) => None,
        (Format::Zip | Format::TarGz, Store | Fast) => Some(1),
        (Format::Zip | Format::TarGz, Normal) => Some(6),
        (Format::Zip | Format::TarGz, Max) => Some(9),
        (Format::SevenZ, Fast) => Some(0),
        (Format::SevenZ, Normal) => Some(5),
        (Format::SevenZ, Max) => Some(9),
        (Format::TarZst, Store | Fast) => Some(1),
        (Format::TarZst, Normal) => Some(3),
        (Format::TarZst, Max) => Some(19),
    }
}

/// Extensions of files that are compressed already (zip stores them as they are).
pub const COMPRESSED_EXT: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "mp3", "m4a", "aac", "ogg", "opus", "flac", "jpg", "jpeg", "png", "gif",
    "webp", "heic", "avif", "zip", "7z", "rar", "gz", "tgz", "bz2", "xz", "zst", "docx", "xlsx", "pptx", "odt", "ods",
    "odp", "epub", "apk", "jar",
];

/// The file is compressed already (by its extension, any case).
pub fn already_compressed(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.').is_some_and(|(stem, e)| !stem.is_empty() && COMPRESSED_EXT.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Compression threads by default: all cores but one (the UI stays smooth).
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1)).max(1)
}

/// Code page for zip names stored without the UTF-8 flag that are not valid
/// UTF-8 (old Windows / DOS archivers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Czech DOS (Latin 2), what Czech Windows archivers write.
    #[default]
    Cp852,
    Cp1250,
    Cp437,
}

impl Encoding {
    pub const ALL: [Encoding; 3] = [Encoding::Cp852, Encoding::Cp1250, Encoding::Cp437];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Cp852 => "CP852 (DOS Latin 2)",
            Encoding::Cp1250 => "CP1250 (Windows)",
            Encoding::Cp437 => "CP437 (DOS USA)",
        }
    }

    pub fn decode(self, raw: &[u8]) -> String {
        match self {
            Encoding::Cp852 => oem_cp::decode_string_complete_table(raw, &oem_cp::code_table::DECODING_TABLE_CP852),
            Encoding::Cp437 => oem_cp::decode_string_complete_table(raw, &oem_cp::code_table::DECODING_TABLE_CP437),
            Encoding::Cp1250 => encoding_rs::WINDOWS_1250.decode_without_bom_handling(raw).0.into_owned(),
        }
    }
}

/// Name of a zip entry: UTF-8 if it is valid UTF-8, otherwise in `enc`.
pub fn decode_name(raw: &[u8], enc: Encoding) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => enc.decode(raw),
    }
}

/// The `Display` is English (the log); [`Error::text`] is for the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("The operation was canceled.")]
    Canceled,
    #[error("The archive is password protected.")]
    NeedPassword,
    #[error("Wrong password.")]
    BadPassword,
    #[error("The archive is damaged: {0}")]
    Corrupt(Text),
    #[error("Unsupported archive: {0}")]
    Unsupported(Text),
    /// A bomb limit was hit (size, count, ratio).
    #[error("{0}")]
    Limit(Text),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn is_cancel(&self) -> bool {
        matches!(self, Error::Canceled)
    }

    /// The message for the user (English template, translated by the UI).
    pub fn text(&self) -> Text {
        match self {
            Error::Io(e) => raw(e),
            Error::Canceled => txt!("The operation was canceled."),
            Error::NeedPassword => txt!("The archive is password protected."),
            Error::BadPassword => txt!("Wrong password."),
            Error::Corrupt(why) => txt!("The archive is damaged: {error}", error = why),
            Error::Unsupported(why) => txt!("Unsupported archive: {error}", error = why),
            Error::Limit(t) => t.clone(),
        }
    }
}

/// A message from a library or the system, shown as it is (not translated).
pub(crate) fn raw(e: impl std::fmt::Display) -> Text {
    txt!("{error}", error = e.to_string())
}

/// Name of the pseudo item in [`ExtractReport::failures`] for the damaged
/// rest of an archive (English, marked with `n!`; the UI translates it).
pub const REST_OF_ARCHIVE: &str = n!("(rest of the archive)");
/// Name of the pseudo item in [`TestReport::bad`] when the damage is not in
/// a named item (English, marked with `n!`; the UI translates it).
pub const WHOLE_ARCHIVE: &str = n!("(archive)");

/// Progress events.
#[derive(Debug)]
pub enum Tick<'a> {
    /// What the operation has in all (items, uncompressed bytes).
    Total { items: u64, bytes: u64 },
    /// Working on this item (path inside the archive).
    Item(&'a str),
    /// This many more bytes (uncompressed) done.
    Bytes(u64),
    /// One more item finished.
    Done,
    /// Bytes of the archive file read so far, of its size (extracting and
    /// testing: the progress and remaining time, the unpacked total is unknown).
    Read { done: u64, total: u64 },
}

/// Cancellation and progress of one operation.
pub struct Ctl<'a> {
    pub cancel: &'a AtomicBool,
    pub on: &'a (dyn Fn(Tick<'_>) + Sync),
}

impl Ctl<'_> {
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Error::Canceled)
        } else {
            Ok(())
        }
    }

    pub fn tick(&self, t: Tick<'_>) {
        (self.on)(t)
    }
}

/// How to read an archive.
#[derive(Default)]
pub struct ReadOpts {
    pub password: Option<SecretString>,
    pub encoding: Encoding,
    /// Stop listing above this many items.
    pub max_entries: u64,
}

impl ReadOpts {
    pub fn new() -> ReadOpts {
        ReadOpts { max_entries: Limits::default().max_entries, ..Default::default() }
    }
}

/// Limits against archive bombs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes written in all (the caller lowers it to the free space − 1 GB).
    pub max_total: u64,
    /// Items in all.
    pub max_entries: u64,
    /// Unpacked : packed ratio; `None` = the user allowed any ratio.
    pub max_ratio: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_total: 20_000_000_000, max_entries: 1_000_000, max_ratio: Some(1000) }
    }
}

/// What to do with a file that already exists in the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Exist {
    #[default]
    Overwrite,
    Skip,
    /// Overwrite if the archived file is newer.
    Newer,
}

/// Kind of an archive item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Hardlink,
    /// Device, FIFO, … (tar): never created.
    Special,
}

/// One item of an archive (with a safe path).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Normalized path inside the archive ("a/b/c.txt"), checked by [`safe_rel`].
    pub path: String,
    pub kind: Kind,
    pub size: u64,
    /// Compressed size, when the format stores it per item.
    pub packed: Option<u64>,
    pub mode: Option<u32>,
    pub mtime: Option<SystemTime>,
    /// Target of a symlink / hardlink (as stored; `None` if it needs decoding).
    pub link: Option<String>,
    pub encrypted: bool,
}

/// An item the listing hid because its path is not safe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub name: String,
    pub reason: Text,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub items: Vec<Item>,
    pub rejected: Vec<Rejected>,
    /// Raw names that are neither UTF-8 nor flagged as such (first few): the
    /// user should pick an encoding.
    pub raw_names: Vec<Vec<u8>>,
    /// Some item is encrypted.
    pub encrypted: bool,
    /// Sum of the declared unpacked sizes.
    pub total_size: u64,
    /// Size of the archive file.
    pub archive_size: u64,
}

impl Listing {
    /// Names at the archive root (explicit or implied by deeper paths).
    pub fn root_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.items.iter().map(|i| i.path.split('/').next().unwrap_or("").to_string()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Declared unpacked : packed ratio (rounded down; 0 for an empty archive).
    pub fn ratio(&self) -> u64 {
        self.total_size / self.archive_size.max(1)
    }
}

/// Why a name in an archive is not safe to extract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsafe {
    Empty,
    Absolute,
    Drive,
    Parent,
    Nul,
}

impl Unsafe {
    /// The reason for the user.
    pub fn text(self) -> Text {
        match self {
            Unsafe::Empty => txt!("empty name"),
            Unsafe::Absolute => txt!("absolute path"),
            Unsafe::Drive => txt!("path with a drive letter"),
            Unsafe::Parent => txt!("the path leads outside (..)"),
            Unsafe::Nul => txt!("invalid character in the name"),
        }
    }
}

/// Normalize a name from an archive to a relative path that stays inside the
/// target: backslashes count as separators, `.` and empty parts are dropped,
/// `..`, absolute paths and drive letters are refused.
pub fn safe_rel(name: &str) -> std::result::Result<String, Unsafe> {
    if name.contains('\0') {
        return Err(Unsafe::Nul);
    }
    let n = name.replace('\\', "/");
    if n.starts_with('/') {
        return Err(Unsafe::Absolute);
    }
    let b = n.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Err(Unsafe::Drive);
    }
    let mut parts = Vec::new();
    for p in n.split('/') {
        match p {
            "" | "." => {}
            ".." => return Err(Unsafe::Parent),
            p => {
                // "C:" in the middle would be a drive / stream on Windows.
                if p.contains(':') && cfg!(windows) {
                    return Err(Unsafe::Drive);
                }
                parts.push(p);
            }
        }
    }
    if parts.is_empty() {
        return Err(Unsafe::Empty);
    }
    Ok(parts.join("/"))
}

/// A link at archive path `link` (already safe) pointing to `target` stays
/// inside the extraction root (lexically; we never extract through links).
pub fn link_inside(link: &str, target: &str) -> bool {
    let t = target.replace('\\', "/");
    if t.is_empty() || t.starts_with('/') || t.contains('\0') {
        return false;
    }
    let tb = t.as_bytes();
    if tb.len() >= 2 && tb[0].is_ascii_alphabetic() && tb[1] == b':' {
        return false;
    }
    let mut stack: Vec<&str> = link.split('/').collect();
    stack.pop(); // the link itself
    for p in t.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            p => stack.push(p),
        }
    }
    true
}

/// Permission bits we restore: setuid and setgid dropped.
pub fn safe_mode(mode: u32) -> u32 {
    mode & 0o7777 & !0o6000
}

/// `<path>.filepart` next to `path`.
pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".filepart");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests;
