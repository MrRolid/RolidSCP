//! A local archive shown as a read-only folder.
//!
//! Paths are `<archive name>/<path inside>` ("photos.zip/2024/january"); the
//! archive root is just the name. The listing is read once (unsafe names are
//! hidden, see `rolid_archive::safe_rel`); a file is read by extracting it into
//! a private temporary folder (kept while the archive is open). Nothing can
//! be written.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rolid_archive::{Encoding, Exist, Format, Item, Kind, Limits, Listing, ReadOpts, Select};
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, EntryKind, Vfs, VfsError, VfsResult};
use rolid_i18n::rolid_text::{n, txt, Text};
use secrecy::SecretString;
use tokio::io::AsyncSeekExt;

use crate::app::archive_ui::{new_temp, TempDir};

/// English, marked with `n!`: translate with `rolid_i18n::t_str`.
pub const READ_ONLY: &str = n!("The archive is open read-only – it cannot be written to.");

pub struct ArchiveFs(Arc<Inner>);

pub struct Inner {
    /// The archive file on this computer.
    pub archive: PathBuf,
    /// Its name (root of the paths).
    pub name: String,
    pub format: Format,
    pub encoding: Encoding,
    /// Some item is encrypted (data needs the password).
    pub encrypted: bool,
    /// Folder (virtual path) → its entries.
    tree: HashMap<String, Vec<Entry>>,
    /// Virtual path → item (only items present in the archive).
    items: HashMap<String, Item>,
    password: Mutex<Option<SecretString>>,
    /// Extracted files (virtual path → local file).
    cache: Mutex<HashMap<String, PathBuf>>,
    work: Mutex<Option<Arc<TempDir>>>,
    /// Temporary folder holding a downloaded remote archive (deleted with us).
    _download: Option<Arc<TempDir>>,
}

impl std::ops::Deref for ArchiveFs {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Folders and entries of a listing, keyed by virtual path under `root`.
pub fn build_tree(root: &str, l: &Listing) -> (HashMap<String, Vec<Entry>>, HashMap<String, Item>) {
    let mut dirs: BTreeMap<String, BTreeMap<String, Entry>> = BTreeMap::new();
    let mut items = HashMap::new();
    dirs.insert(root.to_string(), BTreeMap::new());
    let dir_entry = |name: &str| Entry {
        name: name.to_string(),
        kind: EntryKind::Dir,
        link_to_dir: false,
        size: 0,
        modified: None,
        mode: None,
    };
    let explicit_dirs: std::collections::HashSet<&str> =
        l.items.iter().filter(|i| i.kind == Kind::Dir).map(|i| i.path.as_str()).collect();
    for it in &l.items {
        let parts: Vec<&str> = it.path.split('/').collect();
        // Implied parent folders.
        let mut parent = root.to_string();
        for p in &parts[..parts.len() - 1] {
            let child = format!("{parent}/{p}");
            dirs.entry(parent.clone()).or_default().entry(p.to_string()).or_insert_with(|| dir_entry(p));
            dirs.entry(child.clone()).or_default();
            parent = child;
        }
        let name = parts[parts.len() - 1];
        let vpath = format!("{parent}/{name}");
        let kind = match it.kind {
            Kind::Dir => EntryKind::Dir,
            Kind::File | Kind::Hardlink => EntryKind::File,
            Kind::Symlink => EntryKind::Symlink,
            Kind::Special => EntryKind::Other,
        };
        if kind == EntryKind::Dir {
            dirs.entry(vpath.clone()).or_default();
        }
        // A file and a folder of the same name: the folder wins.
        let slot = dirs.entry(parent).or_default();
        if kind != EntryKind::Dir && (slot.get(name).is_some_and(|e| e.kind == EntryKind::Dir) || explicit_dirs.contains(it.path.as_str())) {
            continue;
        }
        let link_to_dir = it.kind == Kind::Symlink
            && it.link.as_deref().is_some_and(|t| {
                rolid_archive::link_inside(&it.path, t) && explicit_dirs.contains(resolve(&it.path, t).as_str())
            });
        slot.insert(
            name.to_string(),
            Entry { name: name.to_string(), kind, link_to_dir, size: it.size, modified: it.mtime, mode: it.mode },
        );
        items.insert(vpath, it.clone());
    }
    let tree = dirs.into_iter().map(|(k, v)| (k, v.into_values().collect())).collect();
    (tree, items)
}

/// Archive path a link at `link` pointing to `target` leads to (lexically).
fn resolve(link: &str, target: &str) -> String {
    let mut stack: Vec<&str> = link.split('/').collect();
    stack.pop();
    for p in target.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            p => stack.push(p),
        }
    }
    stack.join("/")
}

impl ArchiveFs {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        archive: PathBuf,
        name: String,
        format: Format,
        listing: &Listing,
        encoding: Encoding,
        password: Option<SecretString>,
        download: Option<Arc<TempDir>>,
    ) -> ArchiveFs {
        let (tree, items) = build_tree(&name, listing);
        ArchiveFs(Arc::new(Inner {
            archive,
            name,
            format,
            encoding,
            encrypted: listing.encrypted,
            tree,
            items,
            password: Mutex::new(password),
            cache: Mutex::default(),
            work: Mutex::default(),
            _download: download,
        }))
    }
}

impl Inner {
    pub fn has_password(&self) -> bool {
        self.password.lock().unwrap().is_some()
    }

    pub fn set_password(&self, pw: Option<SecretString>) {
        *self.password.lock().unwrap() = pw;
        self.cache.lock().unwrap().clear();
    }

    pub fn password(&self) -> Option<SecretString> {
        self.password.lock().unwrap().clone()
    }

    /// Path inside the archive ("" = root), `None` = not ours.
    pub fn inner<'a>(&self, path: &'a str) -> Option<&'a str> {
        let p = path.trim_end_matches('/');
        if p == self.name || p.is_empty() || p == "/" {
            return Some("");
        }
        p.strip_prefix(self.name.as_str()).and_then(|r| r.strip_prefix('/'))
    }

    fn norm(&self, path: &str) -> String {
        match self.inner(path) {
            Some("") | None => self.name.clone(),
            Some(i) => format!("{}/{i}", self.name),
        }
    }

    /// Items to extract for these names in folder `dir` (for F5 out).
    pub fn select(&self, dir: &str, names: &[String]) -> Select {
        let base = self.inner(dir).unwrap_or("").to_string();
        let items = names.iter().map(|n| if base.is_empty() { n.clone() } else { format!("{base}/{n}") }).collect();
        Select { base, items }
    }

    pub fn is_encrypted_item(&self, path: &str) -> bool {
        self.items.get(&self.norm(path)).is_some_and(|i| i.encrypted)
    }

    pub fn read_opts(&self) -> ReadOpts {
        ReadOpts { password: self.password(), encoding: self.encoding, max_entries: 0 }
    }

    /// Extract one file (blocking); cached while the archive is open.
    pub fn extract_file(&self, path: &str) -> Result<PathBuf, rolid_archive::Error> {
        let vpath = self.norm(path);
        if let Some(p) = self.cache.lock().unwrap().get(&vpath) {
            if p.exists() {
                return Ok(p.clone());
            }
        }
        let item = self
            .items
            .get(&vpath)
            .ok_or_else(|| not_found(txt!("{path}: not in the archive", path = vpath.as_str())))?;
        let work = {
            let mut w = self.work.lock().unwrap();
            if w.is_none() {
                *w = Some(new_temp().map_err(|e| rolid_archive::Error::Io(std::io::Error::other(e)))?);
            }
            w.clone().unwrap()
        };
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = work.path.join(format!("{n}"));
        std::fs::create_dir_all(&dir)?;
        let inner = self.inner(&vpath).unwrap_or("").to_string();
        let (base, file) = match inner.rsplit_once('/') {
            Some((b, f)) => (b.to_string(), f.to_string()),
            None => (String::new(), inner.clone()),
        };
        let sel = Select { base, items: vec![inner] };
        // The declared size limits what is written (a lying header cannot fill the disk).
        let limits = Limits { max_total: item.size.saturating_add(64 << 10), max_entries: u64::MAX, max_ratio: None };
        let cancel = AtomicBool::new(false);
        let on = |_: rolid_archive::Tick<'_>| {};
        let ctl = rolid_archive::Ctl { cancel: &cancel, on: &on };
        rolid_archive::extract(&self.archive, self.format, &dir, &self.read_opts(), limits, Exist::Overwrite, Some(&sel), &ctl)?;
        let out = dir.join(file);
        if !out.exists() {
            return Err(not_found(txt!("{path}: could not be extracted", path = vpath.as_str())));
        }
        self.cache.lock().unwrap().insert(vpath, out.clone());
        Ok(out)
    }
}

fn ro<T>() -> VfsResult<T> {
    Err(VfsError::Backend(Text::new(READ_ONLY)))
}

/// "Not found" carrying a translatable message (the UI finds the inner
/// [`VfsError`] through `VfsError::text`).
fn not_found(t: Text) -> rolid_archive::Error {
    rolid_archive::Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, VfsError::Backend(t)))
}

#[async_trait]
impl Vfs for ArchiveFs {
    fn label(&self) -> String {
        "Archive".into()
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn read_only(&self) -> bool {
        true
    }
    fn join(&self, dir: &str, name: &str) -> String {
        let d = self.norm(dir);
        format!("{d}/{name}")
    }
    fn parent(&self, path: &str) -> Option<String> {
        let p = self.norm(path);
        if p == self.name {
            return None;
        }
        p.rsplit_once('/').map(|(a, _)| a.to_string())
    }
    async fn home(&self) -> VfsResult<String> {
        Ok(self.name.clone())
    }
    async fn canonicalize(&self, path: &str) -> VfsResult<String> {
        Ok(self.norm(path))
    }
    async fn list(&self, path: &str) -> VfsResult<Vec<Entry>> {
        self.tree
            .get(&self.norm(path))
            .cloned()
            .ok_or_else(|| VfsError::Backend(txt!("{path}: no such folder in the archive", path = path)))
    }
    async fn stat(&self, path: &str) -> VfsResult<Entry> {
        let p = self.norm(path);
        if let Some(parent) = self.parent(&p) {
            let name = p.rsplit('/').next().unwrap_or("");
            if let Some(e) = self.tree.get(&parent).and_then(|v| v.iter().find(|e| e.name == name)) {
                return Ok(e.clone());
            }
        } else {
            return Ok(Entry { name: self.name.clone(), kind: EntryKind::Dir, link_to_dir: false, size: 0, modified: None, mode: None });
        }
        Err(VfsError::Backend(txt!("{path}: not in the archive", path = path)))
    }
    async fn mkdir(&self, _: &str) -> VfsResult<()> {
        ro()
    }
    async fn remove_file(&self, _: &str) -> VfsResult<()> {
        ro()
    }
    async fn remove_dir(&self, _: &str) -> VfsResult<()> {
        ro()
    }
    async fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
        ro()
    }
    async fn open_read_at(&self, path: &str, offset: u64) -> VfsResult<Reader> {
        let me = self.0.clone();
        let path = path.to_string();
        let local = tokio::task::spawn_blocking(move || me.extract_file(&path))
            .await
            .map_err(VfsError::raw)?
            .map_err(|e| match e {
                rolid_archive::Error::Io(io) => VfsError::Io(io),
                other => VfsError::Backend(other.text()),
            })?;
        let mut f = tokio::fs::File::open(&local).await?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }
    async fn open_write_at(&self, _: &str, _: u64) -> VfsResult<Writer> {
        ro()
    }
    async fn set_attrs(&self, _: &str, _: Option<u32>, _: Option<std::time::SystemTime>) -> VfsResult<()> {
        ro()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, kind: Kind) -> Item {
        Item { path: path.into(), kind, size: 3, packed: None, mode: Some(0o644), mtime: None, link: None, encrypted: false }
    }

    fn fs_of(items: Vec<Item>) -> ArchiveFs {
        let l = Listing { items, ..Default::default() };
        ArchiveFs::new("/tmp/x.zip".into(), "x.zip".into(), Format::Zip, &l, Encoding::Cp852, None, None)
    }

    #[test]
    fn virtual_paths() {
        let fs = fs_of(vec![
            item("a.txt", Kind::File),
            item("dir/b.txt", Kind::File),
            item("dir/sub/c.txt", Kind::File),
            item("empty", Kind::Dir),
        ]);
        assert_eq!(fs.inner("x.zip"), Some(""));
        assert_eq!(fs.inner("x.zip/dir/sub"), Some("dir/sub"));
        assert_eq!(fs.inner("/"), Some(""));
        assert_eq!(fs.inner("y.zip/dir"), None);
        assert_eq!(fs.join("x.zip", "dir"), "x.zip/dir");
        assert_eq!(fs.join("x.zip/dir", "b.txt"), "x.zip/dir/b.txt");
        assert_eq!(fs.parent("x.zip/dir/sub").as_deref(), Some("x.zip/dir"));
        assert_eq!(fs.parent("x.zip/dir").as_deref(), Some("x.zip"));
        assert_eq!(fs.parent("x.zip"), None, "at the root '..' leaves the archive");
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let names = |p: &str| -> Vec<String> { rt.block_on(fs.list(p)).unwrap().into_iter().map(|e| e.name).collect() };
        assert_eq!(names("x.zip"), ["a.txt", "dir", "empty"]);
        assert_eq!(names("x.zip/dir"), ["b.txt", "sub"], "implied folder");
        assert_eq!(names("x.zip/dir/sub"), ["c.txt"]);
        assert!(names("x.zip/empty").is_empty());
        assert!(rt.block_on(fs.list("x.zip/missing")).is_err());
        let d = rt.block_on(fs.stat("x.zip/dir")).unwrap();
        assert_eq!(d.kind, EntryKind::Dir);
        assert!(rt.block_on(fs.mkdir("x.zip/n")).is_err(), "read-only");
        assert!(rt.block_on(fs.open_write_at("x.zip/a.txt", 0)).is_err());
        assert!(fs.read_only() && !fs.is_local_disk());
        let sel = fs.select("x.zip/dir", &["sub".into(), "b.txt".into()]);
        assert_eq!(sel.base, "dir");
        assert_eq!(sel.items, ["dir/sub", "dir/b.txt"]);
        assert_eq!(fs.select("x.zip", &["a.txt".into()]).items, ["a.txt"]);
    }

    #[test]
    fn reads_a_file_from_the_archive() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("data");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/f.txt"), "content").unwrap();
        let arch = t.path().join("a.7z");
        let cancel = AtomicBool::new(false);
        let on = |_: rolid_archive::Tick<'_>| {};
        let ctl = rolid_archive::Ctl { cancel: &cancel, on: &on };
        rolid_archive::pack(&[src], &arch, Format::SevenZ, rolid_archive::Level::Fast, None, &ctl).unwrap();
        let l = rolid_archive::list(&arch, Format::SevenZ, &ReadOpts::new()).unwrap();
        let fs = ArchiveFs::new(arch, "a.7z".into(), Format::SevenZ, &l, Encoding::Cp852, None, None);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let mut s = String::new();
        rt.block_on(async {
            use tokio::io::AsyncReadExt;
            let mut r = fs.open_read_at("a.7z/data/sub/f.txt", 2).await.unwrap();
            r.read_to_string(&mut s).await.unwrap();
        });
        assert_eq!(s, "ntent");
    }
}
