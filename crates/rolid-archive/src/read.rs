//! Reading archives: listing, safe extraction, integrity test.

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rolid_text::{txt, txtn, Text};
use secrecy::ExposeSecret;

use crate::pack::{sz_err, zip_err, UNIX_EXT};
use crate::{
    decode_name, link_inside, part_path, raw, safe_mode, safe_rel, Ctl, Error, Exist, Format, Item, Kind, Limits,
    Listing, ReadOpts, Rejected, Result, Tick, Unsafe, REST_OF_ARCHIVE, WHOLE_ARCHIVE,
};

/// Longest symlink target we read from an archive.
const MAX_LINK: u64 = 4096;
/// The ratio is checked once this much has been written (tiny files of zeros
/// compress extremely well and are harmless).
const RATIO_FROM: u64 = 64 << 20;

/// One entry as the format gives it.
struct Raw {
    name: String,
    /// Name bytes that are not valid UTF-8 (decoded with the chosen encoding).
    non_utf8: Option<Vec<u8>>,
    kind: Kind,
    size: u64,
    packed: Option<u64>,
    mode: Option<u32>,
    mtime: Option<SystemTime>,
    link: Option<String>,
    encrypted: bool,
}

impl Raw {
    fn item(&self, path: String) -> Item {
        Item {
            path,
            kind: self.kind,
            size: self.size,
            packed: self.packed,
            mode: self.mode,
            mtime: self.mtime,
            link: self.link.clone(),
            encrypted: self.encrypted,
        }
    }
}

/// File reader that counts the compressed bytes read.
struct Counted {
    f: BufReader<File>,
    n: Arc<AtomicU64>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.f.read(buf)?;
        self.n.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.f.seek(pos)
    }
}

/// What the visitor wants after an entry.
#[derive(PartialEq, Eq)]
enum Flow {
    Next,
    Stop,
}

/// Visitor: the entry and (when data is wanted) a reader of its content.
type Visit<'v> = dyn FnMut(&Raw, Option<&mut dyn Read>) -> Result<Flow> + 'v;

/// Go through the entries of an archive. `data` = give the visitor readers.
fn walk(path: &Path, format: Format, opts: &ReadOpts, data: bool, counter: Arc<AtomicU64>, visit: &mut Visit) -> Result<()> {
    let f = File::open(path)?;
    let src = Counted { f: BufReader::with_capacity(256 << 10, f), n: counter };
    match format {
        Format::Zip => walk_zip(src, opts, data, visit),
        Format::SevenZ => walk_7z(src, opts, data, visit),
        Format::Tar => walk_tar(src, opts, data, visit),
        Format::TarGz => walk_tar(flate2::read::MultiGzDecoder::new(src), opts, data, visit),
        Format::TarZst => walk_tar(zstd::Decoder::new(src)?, opts, data, visit),
    }
}

fn local_time(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<SystemTime> {
    use chrono::TimeZone;
    let t = chrono::Local.with_ymd_and_hms(y, mo, d, h, mi, s).earliest()?;
    Some(t.into())
}

fn unix_time(secs: u64) -> Option<SystemTime> {
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

fn split_name(raw: &[u8], opts: &ReadOpts) -> (String, Option<Vec<u8>>) {
    match std::str::from_utf8(raw) {
        Ok(s) => (s.to_string(), None),
        Err(_) => (decode_name(raw, opts.encoding), Some(raw.to_vec())),
    }
}

fn walk_zip<R: Read + Seek>(src: R, opts: &ReadOpts, data: bool, visit: &mut Visit) -> Result<()> {
    let mut z = zip::ZipArchive::new(src).map_err(zip_err)?;
    for i in 0..z.len() {
        let mut raw = {
            let e = z.by_index_data(i).map_err(zip_err)?;
            let (name, non_utf8) = split_name(e.name_raw(), opts);
            let mode = e.unix_mode();
            let kind = if e.is_dir() {
                Kind::Dir
            } else if e.is_symlink() {
                Kind::Symlink
            } else {
                match mode.map(|m| m & 0o170000) {
                    Some(0o020000 | 0o060000 | 0o010000 | 0o140000) => Kind::Special,
                    _ => Kind::File,
                }
            };
            let mtime = e.last_modified().and_then(|t| {
                local_time(
                    t.year().into(),
                    t.month().into(),
                    t.day().into(),
                    t.hour().into(),
                    t.minute().into(),
                    t.second().into(),
                )
            });
            Raw {
                name,
                non_utf8,
                kind,
                size: e.size(),
                packed: Some(e.compressed_size()),
                mode: mode.map(|m| m & 0o7777),
                mtime,
                link: None,
                encrypted: e.encrypted(),
            }
        };
        let need_data = data || (raw.kind == Kind::Symlink && !raw.encrypted);
        if !need_data {
            if visit(&raw, None)? == Flow::Stop {
                return Ok(());
            }
            continue;
        }
        let mut file = if raw.encrypted {
            let Some(pw) = &opts.password else { return Err(Error::NeedPassword) };
            z.by_index_decrypt(i, pw.expose_secret().as_bytes()).map_err(zip_err)?
        } else {
            z.by_index(i).map_err(zip_err)?
        };
        if raw.kind == Kind::Symlink {
            let mut t = Vec::new();
            (&mut file).take(MAX_LINK).read_to_end(&mut t)?;
            raw.link = Some(String::from_utf8_lossy(&t).into_owned());
            let flow = if data { visit(&raw, Some(&mut io::empty()))? } else { visit(&raw, None)? };
            if flow == Flow::Stop {
                return Ok(());
            }
            continue;
        }
        if visit(&raw, Some(&mut file))? == Flow::Stop {
            return Ok(());
        }
    }
    Ok(())
}

fn sevenz_raw(e: &sevenz_rust2::ArchiveEntry, encrypted: bool) -> Raw {
    let unix = (e.has_windows_attributes && e.windows_attributes & UNIX_EXT != 0).then_some(e.windows_attributes >> 16);
    let kind = if e.is_directory {
        Kind::Dir
    } else {
        match unix.map(|m| m & 0o170000) {
            Some(0o120000) => Kind::Symlink,
            Some(0o020000 | 0o060000 | 0o010000 | 0o140000) => Kind::Special,
            _ => Kind::File,
        }
    };
    let mtime = e.has_last_modified_date.then(|| {
        let t = u64::from(e.last_modified_date);
        let secs = (t / 10_000_000).checked_sub(11_644_473_600)?;
        SystemTime::UNIX_EPOCH.checked_add(Duration::new(secs, ((t % 10_000_000) * 100) as u32))
    });
    Raw {
        name: e.name.clone(),
        non_utf8: None,
        kind,
        size: e.size,
        packed: None,
        mode: unix.map(|m| m & 0o7777),
        mtime: mtime.flatten(),
        link: None,
        encrypted,
    }
}

fn walk_7z<R: Read + Seek>(src: R, opts: &ReadOpts, data: bool, visit: &mut Visit) -> Result<()> {
    use sevenz_rust2::{ArchiveReader, Password};
    let pw = match &opts.password {
        Some(p) => Password::new(p.expose_secret()),
        None => Password::empty(),
    };
    let mut r = ArchiveReader::new(src, pw).map_err(|e| match sz_err(e) {
        // With encrypted names a wrong password shows up as a broken header.
        Error::Corrupt(_) if opts.password.is_some() => Error::BadPassword,
        e => e,
    })?;
    let enc: Vec<bool> = (0..r.archive().files.len()).map(|i| r.archive().is_file_encrypted(i)).collect();
    if !data {
        let any_link = r.archive().files.iter().any(|e| sevenz_raw(e, false).kind == Kind::Symlink);
        if !any_link {
            for (i, e) in r.archive().files.iter().enumerate() {
                if e.is_anti_item {
                    continue;
                }
                if visit(&sevenz_raw(e, enc[i]), None)? == Flow::Stop {
                    break;
                }
            }
            return Ok(());
        }
    }
    // Index of each entry by name order is not exposed in the callback: keep
    // our own counter (entries come in archive order).
    let names: Vec<String> = r.archive().files.iter().map(|e| e.name.clone()).collect();
    let mut idx = 0usize;
    let mut cur_enc = false;
    let mut stop = false;
    let mut inner: Option<Error> = None;
    let res = r.for_each_entries(|e, rd| {
        while idx < names.len() && names[idx] != e.name {
            idx += 1;
        }
        let encrypted = enc.get(idx).copied().unwrap_or(false);
        cur_enc = encrypted;
        idx += 1;
        if e.is_anti_item || stop {
            return Ok(true);
        }
        let mut raw = sevenz_raw(e, encrypted);
        let res = if raw.kind == Kind::Symlink {
            let mut t = Vec::new();
            if let Err(err) = rd.take(MAX_LINK).read_to_end(&mut t) {
                return Err(sevenz_rust2::Error::Io(err, "".into()));
            }
            raw.link = Some(String::from_utf8_lossy(&t).into_owned());
            if data {
                visit(&raw, Some(&mut io::empty()))
            } else {
                visit(&raw, None)
            }
        } else if data {
            visit(&raw, Some(rd))
        } else {
            visit(&raw, None)
        };
        match res {
            Ok(Flow::Next) => Ok(true),
            Ok(Flow::Stop) => {
                stop = true;
                Ok(true)
            }
            Err(err) => {
                inner = Some(err);
                Err(sevenz_rust2::Error::Other("interrupted".into()))
            }
        }
    });
    if let Some(e) = inner {
        return Err(e);
    }
    res.map_err(|e| match sz_err(e) {
        Error::Io(_) | Error::Corrupt(_) if cur_enc => Error::BadPassword,
        Error::Io(io) => tar_io(io),
        e => e,
    })
}

fn walk_tar<R: Read>(src: R, opts: &ReadOpts, data: bool, visit: &mut Visit) -> Result<()> {
    use tar::EntryType as T;
    let mut a = tar::Archive::new(src);
    for e in a.entries().map_err(tar_err)? {
        let mut e = e.map_err(tar_err)?;
        let h = e.header();
        let kind = match h.entry_type() {
            T::Regular | T::Continuous | T::GNUSparse => Kind::File,
            T::Directory => Kind::Dir,
            T::Symlink => Kind::Symlink,
            T::Link => Kind::Hardlink,
            _ => Kind::Special,
        };
        let size = e.size();
        let mode = h.mode().ok().map(|m| m & 0o7777);
        let mtime = h.mtime().ok().and_then(unix_time);
        let (name, non_utf8) = split_name(&e.path_bytes(), opts);
        let link = e.link_name_bytes().map(|b| String::from_utf8_lossy(&b).into_owned());
        let raw = Raw { name, non_utf8, kind, size, packed: None, mode, mtime, link, encrypted: false };
        let flow = if data { visit(&raw, Some(&mut e))? } else { visit(&raw, None)? };
        if flow == Flow::Stop {
            return Ok(());
        }
    }
    if data {
        // Read up to the end of the compressed stream: gzip / zstd check sums
        // sit at the very end.
        io::copy(&mut a.into_inner(), &mut io::sink()).map_err(tar_io)?;
    }
    Ok(())
}

fn tar_io(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof | io::ErrorKind::Other => {
            Error::Corrupt(raw(e))
        }
        _ => Error::Io(e),
    }
}

fn tar_err(e: io::Error) -> Error {
    tar_io(e)
}

/// List an archive. Unsafe names are left out and listed in `rejected`.
pub fn list(path: &Path, format: Format, opts: &ReadOpts) -> Result<Listing> {
    let mut l = Listing { archive_size: fs::metadata(path)?.len(), ..Default::default() };
    let max = if opts.max_entries == 0 { u64::MAX } else { opts.max_entries };
    let mut count = 0u64;
    walk(path, format, opts, false, Arc::new(AtomicU64::new(0)), &mut |r: &Raw, _| {
        count += 1;
        if count > max {
            return Err(Error::Limit(txtn!(
                "The archive has more than {n} item.",
                "The archive has more than {n} items.",
                max
            )));
        }
        if let Some(b) = &r.non_utf8 {
            if l.raw_names.len() < 8 {
                l.raw_names.push(b.clone());
            }
        }
        l.encrypted |= r.encrypted;
        match safe_rel(&r.name) {
            Ok(p) => {
                l.total_size = l.total_size.saturating_add(r.size);
                l.items.push(r.item(p));
            }
            Err(u) => l.rejected.push(Rejected { name: r.name.clone(), reason: u.text() }),
        }
        Ok(Flow::Next)
    })?;
    Ok(l)
}

/// Part of an archive to extract: items under `base` (a folder inside the
/// archive, "" = root); they land in the target without the `base` prefix.
#[derive(Clone, Debug, Default)]
pub struct Select {
    pub base: String,
    /// Paths relative to the archive root (folders include their content).
    pub items: Vec<String>,
}

impl Select {
    /// Target-relative path of archive path `p`, `None` = not selected.
    pub fn map(&self, p: &str) -> Option<String> {
        if !self.items.iter().any(|i| p == i || p.strip_prefix(i.as_str()).is_some_and(|r| r.starts_with('/'))) {
            return None;
        }
        if self.base.is_empty() {
            return Some(p.to_string());
        }
        p.strip_prefix(self.base.as_str()).and_then(|r| r.strip_prefix('/')).map(str::to_string)
    }
}

#[derive(Debug, Default)]
pub struct ExtractReport {
    /// Files, links and folders this operation created (removed again on cancel).
    pub created: Vec<PathBuf>,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Existing files left as they were.
    pub skipped: u64,
    /// Items not extracted: name (or [`REST_OF_ARCHIVE`]) and reason.
    pub failures: Vec<(String, Text)>,
}

/// Writer that enforces the limits on what is really written.
struct Guard<'a> {
    written: u64,
    limits: Limits,
    consumed: Arc<AtomicU64>,
    /// Size of the archive file.
    size: u64,
    ctl: &'a Ctl<'a>,
}

impl Guard<'_> {
    fn add(&mut self, n: u64) -> Result<()> {
        self.written += n;
        if self.written > self.limits.max_total {
            return Err(Error::Limit(txt!(
                "The extraction would write more than {size} – extraction stopped.",
                size = human(self.limits.max_total)
            )));
        }
        if let Some(r) = self.limits.max_ratio {
            let c = self.consumed.load(Ordering::Relaxed).max(1);
            if self.written > RATIO_FROM && self.written / c > r {
                return Err(Error::Limit(txt!(
                    "The compression ratio exceeded {ratio} : 1 (possibly an archive bomb) – extraction stopped.",
                    ratio = r
                )));
            }
        }
        self.ctl.tick(Tick::Bytes(n));
        self.ctl.tick(Tick::Read { done: self.consumed.load(Ordering::Relaxed), total: self.size });
        Ok(())
    }
}

fn human(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{:.1} MB", b as f64 / 1e6)
    }
}

/// Make the folders on the way to `rel` inside `dest` (never through links).
fn make_parents(dest: &Path, rel: &str, created: &mut Vec<PathBuf>) -> std::result::Result<PathBuf, Text> {
    let parts: Vec<&str> = rel.split('/').collect();
    let mut p = dest.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        p.push(part);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(m) if m.file_type().is_symlink() => return Err(txt!("the path goes through a link")),
            Ok(_) => return Err(txt!("there is a file instead of a folder in the path")),
            Err(_) => {
                fs::create_dir(&p).map_err(raw)?;
                created.push(p.clone());
            }
        }
    }
    Ok(p.join(parts[parts.len() - 1]))
}

/// Extract (all or `select`) into the existing folder `dest`.
///
/// On cancel, a limit, a wrong password or a write error everything created so
/// far is removed again and the error returned. A damaged archive keeps what
/// was extracted and lists the rest in `failures`.
#[allow(clippy::too_many_arguments)]
pub fn extract(
    path: &Path,
    format: Format,
    dest: &Path,
    opts: &ReadOpts,
    limits: Limits,
    exist: Exist,
    select: Option<&Select>,
    ctl: &Ctl,
) -> Result<ExtractReport> {
    let mut rep = ExtractReport::default();
    let consumed = Arc::new(AtomicU64::new(0));
    let size = fs::metadata(path).map_or(0, |m| m.len());
    let mut guard = Guard { written: 0, limits, consumed: consumed.clone(), size, ctl };
    let mut dir_modes: Vec<(PathBuf, u32)> = Vec::new();
    let mut count = 0u64;
    // A `.filepart` being written (removed on an error).
    let mut current: Option<PathBuf> = None;
    let res = walk(path, format, opts, true, consumed, &mut |r: &Raw, data| {
        ctl.check()?;
        count += 1;
        if count > limits.max_entries {
            return Err(Error::Limit(txtn!(
                "The archive has more than {n} item – extraction stopped.",
                "The archive has more than {n} items – extraction stopped.",
                limits.max_entries
            )));
        }
        let rel = match safe_rel(&r.name) {
            Ok(p) => p,
            Err(u) => {
                rep.failures.push((r.name.clone(), u.text()));
                return Ok(Flow::Next);
            }
        };
        let rel = match select {
            Some(s) => match s.map(&rel) {
                Some(x) if !x.is_empty() => x,
                _ => return Ok(Flow::Next),
            },
            None => rel,
        };
        ctl.tick(Tick::Item(&rel));
        let target = match make_parents(dest, &rel, &mut rep.created) {
            Ok(t) => t,
            Err(e) => {
                rep.failures.push((rel, e));
                return Ok(Flow::Next);
            }
        };
        let existing = fs::symlink_metadata(&target).ok();
        match r.kind {
            Kind::Dir => {
                match existing {
                    Some(m) if m.file_type().is_dir() => {}
                    Some(_) => {
                        rep.failures.push((rel, txt!("a file or a link is already in place of the folder")));
                        return Ok(Flow::Next);
                    }
                    None => {
                        fs::create_dir(&target)?;
                        rep.created.push(target.clone());
                        if let Some(m) = r.mode {
                            dir_modes.push((target, safe_mode(m)));
                        }
                    }
                }
                rep.dirs += 1;
            }
            Kind::Special => {
                rep.failures.push((rel, txt!("devices, pipes and sockets are not created (skipped)")));
            }
            Kind::File | Kind::Symlink | Kind::Hardlink => {
                if let Some(m) = &existing {
                    if m.file_type().is_dir() {
                        rep.failures.push((rel, txt!("a folder is already in place of the file")));
                        return Ok(Flow::Next);
                    }
                    let skip = match exist {
                        Exist::Overwrite => false,
                        Exist::Skip => true,
                        Exist::Newer => match (r.mtime, m.modified().ok()) {
                            (Some(a), Some(b)) => a <= b,
                            _ => true,
                        },
                    };
                    if skip {
                        rep.skipped += 1;
                        return Ok(Flow::Next);
                    }
                }
                match r.kind {
                    Kind::Symlink => {
                        let t = r.link.clone().unwrap_or_default();
                        if !link_inside(&rel, &t) {
                            rep.failures.push((rel, txt!("the link points outside the target folder ({target})", target = &t)));
                            return Ok(Flow::Next);
                        }
                        if existing.is_some() {
                            fs::remove_file(&target)?;
                        }
                        std::os::unix::fs::symlink(&t, &target)?;
                        if existing.is_none() {
                            rep.created.push(target);
                        }
                        rep.files += 1;
                    }
                    Kind::Hardlink => {
                        let src = r.link.as_deref().ok_or(Unsafe::Empty).and_then(safe_rel).ok();
                        let src = src.and_then(|s| match select {
                            Some(sel) => sel.map(&s),
                            None => Some(s),
                        });
                        let src = src.map(|s| dest.join(s));
                        let ok = src.as_ref().and_then(|s| fs::symlink_metadata(s).ok()).is_some_and(|m| m.is_file());
                        let Some(src) = src.filter(|_| ok) else {
                            rep.failures.push((rel, txt!("the hard link points outside the target folder")));
                            return Ok(Flow::Next);
                        };
                        if existing.is_some() {
                            fs::remove_file(&target)?;
                        }
                        fs::hard_link(&src, &target)?;
                        if existing.is_none() {
                            rep.created.push(target);
                        }
                        rep.files += 1;
                    }
                    _ => {
                        let part = part_path(&target);
                        current = Some(part.clone());
                        let mut out = File::create(&part)?;
                        let mut buf = vec![0u8; 256 << 10];
                        if let Some(d) = data {
                            loop {
                                ctl.check()?;
                                let n = match d.read(&mut buf) {
                                    Ok(n) => n,
                                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                                    Err(e) => return Err(data_err(e, r.encrypted)),
                                };
                                if n == 0 {
                                    break;
                                }
                                guard.add(n as u64)?;
                                out.write_all(&buf[..n])?;
                                rep.bytes += n as u64;
                            }
                        }
                        if let Some(m) = r.mode {
                            out.set_permissions(fs::Permissions::from_mode(safe_mode(m)))?;
                        }
                        if let Some(t) = r.mtime {
                            let _ = out.set_modified(t);
                        }
                        drop(out);
                        fs::rename(&part, &target)?;
                        current = None;
                        if existing.is_none() {
                            rep.created.push(target);
                        }
                        rep.files += 1;
                    }
                }
            }
        }
        ctl.tick(Tick::Done);
        Ok(Flow::Next)
    });
    if let Some(p) = current.take() {
        let _ = fs::remove_file(p);
    }
    match res {
        Ok(()) => {}
        Err(Error::Corrupt(m)) => {
            rep.failures.push((REST_OF_ARCHIVE.into(), txt!("the archive is damaged: {error}", error = m)))
        }
        Err(e) => {
            cleanup(&rep.created);
            return Err(if ctl.check().is_err() { Error::Canceled } else { e });
        }
    }
    // Folder permissions last (a read-only folder would stop the extraction).
    for (p, m) in dir_modes.iter().rev() {
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(*m));
    }
    Ok(rep)
}

/// Error while reading entry data: wrong password or damaged data.
fn data_err(e: io::Error, encrypted: bool) -> Error {
    let msg = raw(&e);
    // Decrypting with a wrong key gives garbage: the decompressor or the
    // check sum fails. A damaged encrypted item looks the same.
    if encrypted && e.kind() != io::ErrorKind::PermissionDenied && e.kind() != io::ErrorKind::StorageFull {
        return Error::BadPassword;
    }
    match e.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof | io::ErrorKind::Other => {
            Error::Corrupt(msg)
        }
        _ => Error::Io(e),
    }
}

/// Remove what an extraction created (newest first; folders only if empty).
pub fn cleanup(created: &[PathBuf]) {
    for p in created.iter().rev() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => {
                let _ = fs::remove_dir(p);
            }
            Ok(_) => {
                let _ = fs::remove_file(p);
            }
            Err(_) => {}
        }
    }
}

#[derive(Debug, Default)]
pub struct TestReport {
    pub items: u64,
    pub bytes: u64,
    /// Damaged items: name (or [`WHOLE_ARCHIVE`]) and reason. Empty = the archive is fine.
    pub bad: Vec<(String, Text)>,
}

/// Read and verify every entry (CRC / check sums); writes nothing.
pub fn test(path: &Path, format: Format, opts: &ReadOpts, ctl: &Ctl) -> Result<TestReport> {
    let mut rep = TestReport::default();
    let mut last = String::new();
    let mut buf = vec![0u8; 256 << 10];
    let size = fs::metadata(path).map_or(0, |m| m.len());
    let consumed = Arc::new(AtomicU64::new(0));
    let res = walk(path, format, opts, true, consumed.clone(), &mut |r: &Raw, data| {
        ctl.check()?;
        last = r.name.clone();
        ctl.tick(Tick::Item(&r.name));
        rep.items += 1;
        if let Some(d) = data {
            loop {
                ctl.check()?;
                match d.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        rep.bytes += n as u64;
                        ctl.tick(Tick::Bytes(n as u64));
                        ctl.tick(Tick::Read { done: consumed.load(Ordering::Relaxed), total: size });
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => match data_err(e, r.encrypted) {
                        Error::Corrupt(m) => {
                            rep.bad.push((r.name.clone(), m));
                            // Zip items are independent: go on. A stream
                            // (tar.*) cannot continue after damaged data.
                            return Ok(if format == Format::Zip { Flow::Next } else { Flow::Stop });
                        }
                        e => return Err(e),
                    },
                }
            }
        }
        ctl.tick(Tick::Done);
        Ok(Flow::Next)
    });
    match res {
        Ok(()) => Ok(rep),
        Err(Error::Corrupt(m)) => {
            let at = if last.is_empty() { WHOLE_ARCHIVE.to_string() } else { last };
            if !rep.bad.iter().any(|(n, _)| *n == at) {
                rep.bad.push((at, m));
            }
            Ok(rep)
        }
        Err(e) => Err(if ctl.check().is_err() { Error::Canceled } else { e }),
    }
}
