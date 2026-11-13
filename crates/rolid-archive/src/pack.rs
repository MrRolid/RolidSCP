//! Packing local files and folders into a new archive.

use std::fs::{self, File, Metadata};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use secrecy::{ExposeSecret, SecretString};

use rolid_text::{txt, Text};

use crate::par::ordered;
use crate::{already_compressed, level_value, part_path, raw, Ctl, Error, Format, Level, Result, Tick};

/// How to pack.
#[derive(Clone, Debug)]
pub struct PackOpts {
    pub format: Format,
    pub level: Level,
    pub password: Option<SecretString>,
    /// Compression threads (1 = everything on the calling thread).
    pub threads: usize,
    /// zip: store already-compressed files (by extension) without compression.
    pub store_compressed: bool,
}

impl PackOpts {
    pub fn new(format: Format, level: Level) -> PackOpts {
        PackOpts { format, level, password: None, threads: crate::default_threads(), store_compressed: true }
    }
}

/// Result of a finished packing.
#[derive(Debug, Default)]
pub struct PackReport {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Items left out (unreadable, special files): path and reason.
    pub failures: Vec<(String, Text)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SrcKind {
    File,
    Dir,
    Symlink,
}

struct Src {
    abs: PathBuf,
    /// Path inside the archive.
    name: String,
    kind: SrcKind,
    size: u64,
    /// Permission bits (without the file type).
    mode: u32,
    mtime: Option<SystemTime>,
    target: String,
}

const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

/// Files up to this size are compressed on worker threads (whole, in memory);
/// bigger ones on the writing thread (7z: with multi-threaded LZMA2).
#[cfg(not(test))]
const PAR_MAX: u64 = 32 << 20;
/// Lower in tests, so the small test files take both paths.
#[cfg(test)]
const PAR_MAX: u64 = 200_000;

/// [`pack_with`] with the default threads, already-compressed files stored.
pub fn pack(
    sources: &[PathBuf],
    out: &Path,
    format: Format,
    level: Level,
    password: Option<&SecretString>,
    ctl: &Ctl,
) -> Result<PackReport> {
    let mut o = PackOpts::new(format, level);
    o.password = password.cloned();
    pack_with(sources, out, &o, ctl)
}

/// Pack `sources` (local paths; each goes to the archive root under its own
/// name, folders recursively) into `out`. Symlinks are stored as links, not
/// followed; permissions and times are kept. The archive is written to
/// `out.filepart` and renamed at the end; on an error or cancel the partial
/// file is deleted. `out` is replaced if it exists (the caller decides that).
pub fn pack_with(sources: &[PathBuf], out: &Path, o: &PackOpts, ctl: &Ctl) -> Result<PackReport> {
    let format = o.format;
    let password = o.password.as_ref().filter(|p| !p.expose_secret().is_empty());
    if password.is_some() && !format.can_encrypt() {
        return Err(Error::Unsupported(txt!("format {format} cannot be encrypted", format = format.label())));
    }
    let mut rep = PackReport::default();
    let mut list = Vec::new();
    for s in sources {
        ctl.check()?;
        let name = s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.is_empty() {
            rep.failures.push((s.display().to_string(), txt!("the root of a disk cannot be packed")));
            continue;
        }
        collect(s, &name, &mut list, &mut rep.failures, ctl)?;
    }
    let bytes: u64 = list.iter().filter(|s| s.kind == SrcKind::File).map(|s| s.size).sum();
    ctl.tick(Tick::Total { items: list.len() as u64, bytes });

    let threads = o.threads.max(1);
    let lvl = level_value(format, o.level);
    let part = part_path(out);
    let result = File::create(&part).map_err(Error::from).and_then(|f| {
        let w = BufWriter::with_capacity(1 << 20, f);
        match format {
            Format::Zip => write_zip(w, &list, o, password, ctl, &mut rep),
            Format::SevenZ => write_7z(w, &list, lvl, threads, password, ctl, &mut rep),
            Format::Tar => write_tar(w, &list, ctl, &mut rep).and_then(finish_buf),
            Format::TarGz => {
                let enc = flate2::write::GzEncoder::new(w, flate2::Compression::new(lvl.unwrap_or(1) as u32));
                write_tar(enc, &list, ctl, &mut rep).and_then(|e| Ok(e.finish()?)).and_then(finish_buf)
            }
            Format::TarZst => {
                let mut enc = zstd::Encoder::new(w, lvl.unwrap_or(1))?;
                enc.include_checksum(true)?;
                if threads > 1 {
                    enc.multithread(threads as u32)?;
                }
                write_tar(enc, &list, ctl, &mut rep).and_then(|e| Ok(e.finish()?)).and_then(finish_buf)
            }
        }
    });
    let result = result.and_then(|()| Ok(fs::rename(&part, out)?));
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(if ctl.check().is_err() { Error::Canceled } else { e });
    }
    Ok(rep)
}

/// Memory the compression may take at once: a quarter of the RAM, at least 1 GiB.
fn mem_budget() -> u64 {
    let total = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines()
                .find_map(|l| l.strip_prefix("MemTotal:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        })
        .map_or(4 << 30, |kb| kb * 1024);
    (total / 4).max(1 << 30)
}

/// What a worker reports for one file.
enum Skip {
    /// Left out (could not be opened): the reason.
    Left(Text),
    Fatal(Error),
}

fn finish_buf(mut w: BufWriter<File>) -> Result<()> {
    w.flush()?;
    let f = w.into_inner().map_err(|e| e.into_error())?;
    f.sync_all()?;
    Ok(())
}

fn collect(abs: &Path, name: &str, out: &mut Vec<Src>, fails: &mut Vec<(String, Text)>, ctl: &Ctl) -> Result<()> {
    ctl.check()?;
    let meta = match fs::symlink_metadata(abs) {
        Ok(m) => m,
        Err(e) => {
            fails.push((name.to_string(), raw(e)));
            return Ok(());
        }
    };
    let ft = meta.file_type();
    let base = |kind, size, target| Src {
        abs: abs.to_path_buf(),
        name: name.to_string(),
        kind,
        size,
        mode: meta.permissions().mode() & 0o7777,
        mtime: meta.modified().ok(),
        target,
    };
    if ft.is_symlink() {
        match fs::read_link(abs) {
            Ok(t) => out.push(base(SrcKind::Symlink, 0, t.to_string_lossy().into_owned())),
            Err(e) => fails.push((name.to_string(), raw(e))),
        }
    } else if ft.is_dir() {
        out.push(base(SrcKind::Dir, 0, String::new()));
        let mut kids: Vec<_> = match fs::read_dir(abs) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
            Err(e) => {
                fails.push((name.to_string(), raw(e)));
                return Ok(());
            }
        };
        kids.sort();
        for k in kids {
            let child = format!("{name}/{}", k.to_string_lossy());
            collect(&abs.join(&k), &child, out, fails, ctl)?;
        }
    } else if ft.is_file() {
        out.push(base(SrcKind::File, meta.len(), String::new()));
    } else {
        fails.push((name.to_string(), txt!("special files (device, pipe, socket) are not packed")));
    }
    Ok(())
}

/// Reader that stops on cancel (or when another thread failed) and reports
/// the bytes it passes on.
struct Feed<'a, 'c, R> {
    r: R,
    ctl: &'a Ctl<'c>,
    n: u64,
    abort: Option<&'a AtomicBool>,
}

impl<'a, 'c, R> Feed<'a, 'c, R> {
    fn new(r: R, ctl: &'a Ctl<'c>) -> Self {
        Feed { r, ctl, n: 0, abort: None }
    }
}

impl<R: Read> Read for Feed<'_, '_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ctl.check().is_err() || self.abort.is_some_and(|a| a.load(Ordering::Relaxed)) {
            return Err(io::Error::other("canceled"));
        }
        let n = self.r.read(buf)?;
        self.n += n as u64;
        self.ctl.tick(Tick::Bytes(n as u64));
        Ok(n)
    }
}

/// Open a source file; a failure is recorded and the file left out.
fn open_src(s: &Src, rep: &mut PackReport) -> Option<(File, Metadata)> {
    match open_file(s) {
        Ok(x) => Some(x),
        Err(e) => {
            rep.failures.push((s.name.clone(), e));
            None
        }
    }
}

fn open_file(s: &Src) -> std::result::Result<(File, Metadata), Text> {
    File::open(&s.abs).and_then(|f| f.metadata().map(|m| (f, m))).map_err(raw)
}

/// A worker's result for a file: record a left-out file, pass a fatal error on.
fn took<T>(r: std::result::Result<T, Skip>, s: &Src, ctl: &Ctl, rep: &mut PackReport) -> Result<Option<T>> {
    match r {
        Ok(x) => Ok(Some(x)),
        Err(Skip::Left(e)) => {
            rep.failures.push((s.name.clone(), e));
            Ok(None)
        }
        Err(Skip::Fatal(_)) if ctl.check().is_err() => Err(Error::Canceled),
        Err(Skip::Fatal(e)) => Err(e),
    }
}

fn zip_time(t: Option<SystemTime>) -> zip::DateTime {
    use chrono::{Datelike, Timelike};
    t.and_then(|t| {
        let l: chrono::DateTime<chrono::Local> = t.into();
        zip::DateTime::from_date_and_time(
            u16::try_from(l.year()).ok()?,
            l.month() as u8,
            l.day() as u8,
            l.hour() as u8,
            l.minute() as u8,
            l.second().min(59) as u8,
        )
        .ok()
    })
    .unwrap_or_default()
}

fn write_zip(
    w: BufWriter<File>,
    list: &[Src],
    o: &PackOpts,
    password: Option<&SecretString>,
    ctl: &Ctl,
    rep: &mut PackReport,
) -> Result<()> {
    use zip::write::{FileOptions, ZipFileBuilder};
    use zip::{AesMode, CompressionMethod, ZipWriter};
    let lvl = level_value(Format::Zip, o.level);
    let opts = |s: &Src| -> zip::write::SimpleFileOptions {
        let store = lvl.is_none() || (o.store_compressed && s.kind == SrcKind::File && already_compressed(&s.name));
        let o: FileOptions<()> = FileOptions::default();
        let o = if store {
            o.compression_method(CompressionMethod::Stored)
        } else {
            o.compression_method(CompressionMethod::Deflated).compression_level(lvl.map(i64::from))
        };
        o.unix_permissions(s.mode).last_modified_time(zip_time(s.mtime))
    };
    // Encrypted entries can only be written by the ZipWriter itself.
    let cost = |s: &Src| (password.is_none() && s.kind == SrcKind::File && s.size <= PAR_MAX).then_some(s.size + (64 << 10));
    let work = |s: &Src, abort: &AtomicBool| -> std::result::Result<(zip::write::PreparedZipFile, u64), Skip> {
        let (f, _) = open_file(s).map_err(Skip::Left)?;
        let mut b = ZipFileBuilder::new(&s.name, opts(s)).map_err(|e| Skip::Fatal(zip_err(e)))?;
        let mut feed = Feed { r: f, ctl, n: 0, abort: Some(abort) };
        io::copy(&mut feed, &mut b).map_err(|e| Skip::Fatal(e.into()))?;
        let n = feed.n;
        Ok((b.finish().map_err(|e| Skip::Fatal(zip_err(e)))?, n))
    };
    let mut z = ZipWriter::new(w);
    ordered(list, o.threads, 256 << 20, cost, work, ctl.cancel, |s, done| {
        ctl.check()?;
        ctl.tick(Tick::Item(&s.name));
        if let Some(r) = done {
            if let Some((p, n)) = took(r, s, ctl, rep)? {
                z.add_prepared_file(p).map_err(zip_err)?;
                rep.bytes += n;
                rep.files += 1;
            }
            ctl.tick(Tick::Done);
            return Ok(());
        }
        let o = opts(s);
        match s.kind {
            SrcKind::Dir => {
                z.add_directory(format!("{}/", s.name), o).map_err(zip_err)?;
                rep.dirs += 1;
            }
            SrcKind::Symlink => {
                z.add_symlink(&s.name, &s.target, o).map_err(zip_err)?;
                rep.files += 1;
            }
            SrcKind::File => {
                let Some((f, m)) = open_src(s, rep) else { return Ok(()) };
                let mut o = o.large_file(m.len() >= 0xFFFF_0000);
                if let Some(pw) = password {
                    o = o.with_aes_encryption(AesMode::Aes256, pw.expose_secret());
                }
                z.start_file(&s.name, o).map_err(zip_err)?;
                let mut feed = Feed::new(f, ctl);
                io::copy(&mut feed, &mut z)?;
                rep.bytes += feed.n;
                rep.files += 1;
            }
        }
        ctl.tick(Tick::Done);
        Ok(())
    })?;
    let w = z.finish().map_err(zip_err)?;
    finish_buf(w)
}

pub(crate) fn zip_err(e: zip::result::ZipError) -> Error {
    use zip::result::ZipError as Z;
    match e {
        Z::Io(e) => Error::Io(e),
        Z::InvalidPassword => Error::BadPassword,
        Z::UnsupportedArchive(m) if m.contains("assword") => Error::NeedPassword,
        Z::UnsupportedArchive(m) => Error::Unsupported(raw(m)),
        e => Error::Corrupt(raw(e)),
    }
}

/// 100 ns ticks since 1601 (7z / NTFS time).
fn nt_time(t: SystemTime) -> u64 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() + EPOCH_DIFF) * 10_000_000 + u64::from(d.subsec_nanos() / 100),
        Err(e) => (EPOCH_DIFF * 10_000_000).saturating_sub(e.duration().as_nanos() as u64 / 100),
    }
}

/// Windows attributes with the unix mode in the upper half (p7zip convention).
pub(crate) const UNIX_EXT: u32 = 0x8000;

/// LZMA2 dictionary of each preset (lzma-rust2 / xz).
const LZMA2_DICT: [u32; 10] = [1 << 18, 1 << 20, 1 << 21, 1 << 22, 1 << 22, 1 << 23, 1 << 23, 1 << 24, 1 << 25, 1 << 26];

/// Rough memory of one LZMA2 encoder with dictionary `dict`.
fn lzma_mem(dict: u64) -> u64 {
    dict * 12 + (4 << 20)
}

/// The dictionary for a file of `size` bytes: no bigger than the file (saves
/// memory and time), at most the preset's.
fn dict_for(preset: i32, size: u64) -> u32 {
    let full = LZMA2_DICT[preset.clamp(0, 9) as usize];
    size.max(1 << 16).next_power_of_two().min(u64::from(full)) as u32
}

#[allow(clippy::too_many_arguments)]
fn write_7z(
    w: BufWriter<File>,
    list: &[Src],
    lvl: Option<i32>,
    threads: usize,
    password: Option<&SecretString>,
    ctl: &Ctl,
    rep: &mut PackReport,
) -> Result<()> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{prepare_block, ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password, SourceReader};
    use std::sync::Arc;
    // Key derivation as strong as 7-Zip's (2^19 SHA-256 rounds; the library
    // default is 2^8). One salt (the key is derived once), but a fresh IV for
    // every item: each item is its own CBC stream.
    let aes = password.map(|pw| {
        let mut a = AesEncoderOptions::new(Password::new(pw.expose_secret()));
        a.num_cycles_power = 19;
        a
    });
    let budget = mem_budget();
    // Methods for one stream: a file of `size` bytes (`None` = the header),
    // compressed by `mt` threads.
    let methods = |size: Option<u64>, mt: u32| -> Result<Vec<EncoderConfiguration>> {
        let mut m = Vec::new();
        if let Some(a) = &aes {
            let mut a = a.clone();
            getrandom::fill(&mut a.iv).map_err(|e| Error::Io(io::Error::other(e.to_string())))?;
            m.push(a.into());
        }
        match lvl {
            None => m.push(EncoderConfiguration::new(EncoderMethod::COPY)),
            Some(l) => {
                let dict = size.map_or(LZMA2_DICT[l.clamp(0, 9) as usize], |s| dict_for(l, s));
                let mut o = if mt > 1 {
                    Lzma2Options::from_level_mt(l as u32, mt, (u64::from(dict) * 2).max(4 << 20))
                } else {
                    Lzma2Options::from_level(l as u32)
                };
                o.set_dictionary_size(dict);
                m.push(o.into());
            }
        }
        Ok(m)
    };
    // Threads for one big file: as many as the memory allows.
    let big_threads = |size: u64| -> u32 {
        let Some(l) = lvl else { return 1 };
        let dict = u64::from(dict_for(l, size));
        let per = lzma_mem(dict) + dict * 4;
        (budget / per).clamp(1, threads as u64) as u32
    };
    let entry = |s: &Src, mode: u32| -> ArchiveEntry {
        let mut e = match s.kind {
            SrcKind::Dir => ArchiveEntry::new_directory(&s.name),
            SrcKind::File | SrcKind::Symlink => ArchiveEntry::new_file(&s.name),
        };
        if let Some(t) = s.mtime {
            e.has_last_modified_date = true;
            e.last_modified_date = nt_time(t).into();
        }
        e.has_windows_attributes = true;
        e.windows_attributes = match s.kind {
            SrcKind::Dir => UNIX_EXT | S_IFDIR | 0x10 | ((S_IFDIR | s.mode) << 16),
            SrcKind::Symlink => UNIX_EXT | 0x20 | ((S_IFLNK | 0o777) << 16),
            SrcKind::File => UNIX_EXT | 0x20 | ((S_IFREG | mode) << 16),
        };
        e
    };
    let cost = |s: &Src| {
        (s.kind == SrcKind::File && s.size > 0 && s.size <= PAR_MAX)
            .then(|| s.size + lvl.map_or(0, |l| lzma_mem(u64::from(dict_for(l, s.size)))))
    };
    let work = |s: &Src, abort: &AtomicBool| -> std::result::Result<(sevenz_rust2::PreparedBlock, u64), Skip> {
        let (f, m) = open_file(s).map_err(Skip::Left)?;
        let e = entry(s, m.mode() & 0o7777);
        let ms = methods(Some(m.len()), 1).map_err(Skip::Fatal)?;
        let mut feed = Feed { r: f, ctl, n: 0, abort: Some(abort) };
        let b = prepare_block(Arc::new(ms), vec![e], vec![SourceReader::new(&mut feed)]).map_err(|e| Skip::Fatal(sz_err(e)))?;
        Ok((b, feed.n))
    };
    let mut z = ArchiveWriter::new(w).map_err(sz_err)?;
    // Names are encrypted too (asks for the password before listing).
    z.set_encrypt_header(password.is_some());
    ordered(list, threads, budget, cost, work, ctl.cancel, |s, done| {
        ctl.check()?;
        ctl.tick(Tick::Item(&s.name));
        if let Some(r) = done {
            if let Some((b, n)) = took(r, s, ctl, rep)? {
                z.push_prepared_block(b).map_err(sz_err)?;
                rep.bytes += n;
                rep.files += 1;
            }
            ctl.tick(Tick::Done);
            return Ok(());
        }
        match s.kind {
            SrcKind::Dir => {
                z.push_archive_entry::<&[u8]>(entry(s, 0), None).map_err(sz_err)?;
                rep.dirs += 1;
            }
            SrcKind::Symlink => {
                z.set_content_methods(methods(Some(s.target.len() as u64), 1)?);
                z.push_archive_entry(entry(s, 0), Some(s.target.as_bytes())).map_err(sz_err)?;
                rep.files += 1;
            }
            SrcKind::File => {
                let Some((f, m)) = open_src(s, rep) else { return Ok(()) };
                let e = entry(s, m.mode() & 0o7777);
                if m.len() == 0 {
                    z.push_archive_entry::<&[u8]>(e, None).map_err(sz_err)?;
                } else {
                    z.set_content_methods(methods(Some(m.len()), big_threads(m.len()))?);
                    let mut feed = Feed::new(f, ctl);
                    z.push_archive_entry(e, Some(&mut feed)).map_err(sz_err)?;
                    rep.bytes += feed.n;
                }
                rep.files += 1;
            }
        }
        ctl.tick(Tick::Done);
        Ok(())
    })?;
    // The header gets its own IV as well.
    z.set_content_methods(methods(None, 1)?);
    let w = z.finish()?;
    finish_buf(w)
}

pub(crate) fn sz_err(e: sevenz_rust2::Error) -> Error {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired => Error::NeedPassword,
        E::MaybeBadPassword(_) => Error::BadPassword,
        E::Io(e, _) | E::FileOpen(e, _) => Error::Io(e),
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => {
            Error::Corrupt(txt!("checksum mismatch"))
        }
        E::Unsupported(m) => Error::Unsupported(raw(m)),
        E::UnsupportedCompressionMethod(m) => Error::Unsupported(raw(m)),
        E::BadSignature(_) => Error::Corrupt(txt!("it is not a 7z archive")),
        e => Error::Corrupt(raw(e)),
    }
}

fn write_tar<W: Write>(w: W, list: &[Src], ctl: &Ctl, rep: &mut PackReport) -> Result<W> {
    use tar::{EntryType, Header, HeaderMode};
    let mut b = tar::Builder::new(w);
    b.follow_symlinks(false);
    for s in list {
        ctl.check()?;
        ctl.tick(Tick::Item(&s.name));
        match s.kind {
            SrcKind::Dir | SrcKind::Symlink => {
                let meta = match fs::symlink_metadata(&s.abs) {
                    Ok(m) => m,
                    Err(e) => {
                        rep.failures.push((s.name.clone(), raw(e)));
                        continue;
                    }
                };
                let mut h = Header::new_gnu();
                h.set_metadata_in_mode(&meta, HeaderMode::Complete);
                h.set_size(0);
                if s.kind == SrcKind::Dir {
                    h.set_entry_type(EntryType::Directory);
                    b.append_data(&mut h, format!("{}/", s.name), io::empty())?;
                    rep.dirs += 1;
                } else {
                    h.set_entry_type(EntryType::Symlink);
                    b.append_link(&mut h, &s.name, &s.target)?;
                    rep.files += 1;
                }
            }
            SrcKind::File => {
                let Some((f, m)) = open_src(s, rep) else { continue };
                let mut h = Header::new_gnu();
                h.set_metadata_in_mode(&m, HeaderMode::Complete);
                h.set_entry_type(EntryType::Regular);
                let mut feed = Feed::new(f.take(m.len()), ctl);
                b.append_data(&mut h, &s.name, &mut feed)?;
                rep.bytes += feed.n;
                rep.files += 1;
            }
        }
        ctl.tick(Tick::Done);
    }
    Ok(b.into_inner()?)
}
