//! Thumbnails: loaded and decoded in the background (never on the UI thread),
//! kept in a memory cache with a size budget, canceled when the panel leaves
//! the folder. Image bombs are stopped by [`image::Limits`] (dimensions and
//! memory). Remote files only up to a size limit, at most two downloads at a
//! time, never on production servers.

use rolid_i18n::rolid_text::n;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use rolid_core::{Entry, Vfs};
use tokio::io::AsyncReadExt;

/// Image types that get a preview.
pub const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];
/// Largest image dimension accepted by the decoder.
pub const MAX_DIM: u32 = 10_000;
/// Most memory the decoder may allocate for one image.
pub const MAX_ALLOC: u64 = 200 * 1024 * 1024;
/// Largest local file read for a preview.
pub const MAX_LOCAL_FILE: u64 = 64 * 1024 * 1024;
/// Memory for decoded previews (RGBA textures).
pub const BUDGET: usize = 48 * 1024 * 1024;
/// Concurrent remote downloads / local decodes.
pub const REMOTE_PARALLEL: usize = 2;
pub const LOCAL_PARALLEL: usize = 4;
/// A queued preview nobody asked for during this long is dropped (scrolled away).
const STALE: Duration = Duration::from_millis(1500);

pub fn is_image(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| IMAGE_EXT.contains(&e.to_lowercase().as_str()))
}

/// Why a file gets no preview (or `None` = it does), marked with `n!`
/// (translate with `rolid_i18n::t_str`).
pub fn refusal(name: &str, size: u64, remote: bool, remote_on: bool, production: bool, remote_max: u64) -> Option<&'static str> {
    if !is_image(name) {
        return Some(n!("not an image"));
    }
    if remote && production {
        return Some(n!("production server – thumbnails are off"));
    }
    if remote && !remote_on {
        return Some(n!("thumbnails on servers are off"));
    }
    if remote && size > remote_max {
        return Some(n!("over the size limit for thumbnails from a server"));
    }
    if !remote && size > MAX_LOCAL_FILE {
        return Some(n!("file too large"));
    }
    None
}

/// Decode `bytes` into a preview at most `px` × `px`, with limits.
pub fn decode(bytes: &[u8], px: u32, max_dim: u32, max_alloc: u64) -> Result<ColorImage, String> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(max_dim);
    lim.max_image_height = Some(max_dim);
    lim.max_alloc = Some(max_alloc);
    r.limits(lim);
    let img = r.decode().map_err(|e| e.to_string())?;
    // Small images stay as they are (thumbnail() would enlarge them).
    let t = if img.width() <= px && img.height() <= px { img.to_rgba8() } else { img.thumbnail(px, px).to_rgba8() };
    let size = [t.width() as usize, t.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, t.as_raw()))
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    fs: usize,
    path: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
    px: u32,
}

enum Slot {
    Pending { wanted: Arc<AtomicU64> },
    Ready { tex: TextureHandle, bytes: usize, used: u64 },
    Failed,
}

pub enum Thumb {
    Ready(egui::TextureId, egui::Vec2),
    Loading,
    /// Could not be decoded (or refused by the limits): show the type icon.
    Failed,
}

type Done = Arc<Mutex<Vec<(Key, Result<ColorImage, String>)>>>;

pub struct Thumbs {
    rt: tokio::runtime::Handle,
    ctx: egui::Context,
    cache: HashMap<Key, Slot>,
    bytes: usize,
    tick: u64,
    started: Instant,
    done: Done,
    remote: Arc<tokio::sync::Semaphore>,
    local: Arc<tokio::sync::Semaphore>,
    /// Running decodes / downloads (for tests and the status line).
    pub running: Arc<AtomicU64>,
}

impl Thumbs {
    pub fn new(rt: tokio::runtime::Handle, ctx: egui::Context) -> Self {
        Thumbs {
            rt,
            ctx,
            cache: HashMap::new(),
            bytes: 0,
            tick: 0,
            started: Instant::now(),
            done: Arc::default(),
            remote: Arc::new(tokio::sync::Semaphore::new(REMOTE_PARALLEL)),
            local: Arc::new(tokio::sync::Semaphore::new(LOCAL_PARALLEL)),
            running: Arc::default(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The preview of `entry` in `dir` (requested in the background if needed).
    /// `cancel` belongs to the panel's current folder.
    pub fn get(&mut self, fs: &Arc<dyn Vfs>, path: String, entry: &Entry, px: u32, cancel: &Arc<AtomicBool>) -> Thumb {
        let key = Key {
            fs: Arc::as_ptr(fs) as *const () as usize,
            path,
            size: entry.size,
            modified: entry.modified,
            px,
        };
        self.tick += 1;
        let now = self.now_ms();
        match self.cache.get_mut(&key) {
            Some(Slot::Ready { tex, used, .. }) => {
                *used = self.tick;
                return Thumb::Ready(tex.id(), tex.size_vec2());
            }
            Some(Slot::Failed) => return Thumb::Failed,
            Some(Slot::Pending { wanted }) => {
                wanted.store(now, Ordering::Relaxed);
                return Thumb::Loading;
            }
            None => {}
        }
        let wanted = Arc::new(AtomicU64::new(now));
        self.cache.insert(key.clone(), Slot::Pending { wanted: wanted.clone() });
        let (fs, cancel, done, ctx) = (fs.clone(), cancel.clone(), self.done.clone(), self.ctx.clone());
        let sem = if fs.is_remote() || fs.read_only() { self.remote.clone() } else { self.local.clone() };
        let running = self.running.clone();
        let started = self.started;
        self.rt.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let stale = (started.elapsed().as_millis() as u64).saturating_sub(wanted.load(Ordering::Relaxed)) > STALE.as_millis() as u64;
            let r = if cancel.load(Ordering::Relaxed) || stale {
                Err(String::new()) // canceled: forget, may be asked again later
            } else {
                running.fetch_add(1, Ordering::Relaxed);
                let r = load(fs.as_ref(), &key.path, key.size, &cancel).await;
                let r = match r {
                    Ok(bytes) => {
                        let px = key.px;
                        tokio::task::spawn_blocking(move || decode(&bytes, px, MAX_DIM, MAX_ALLOC))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    }
                    Err(e) => Err(e),
                };
                running.fetch_sub(1, Ordering::Relaxed);
                r.map_err(|e| if e.is_empty() { "error".into() } else { e })
            };
            done.lock().unwrap().push((key, r));
            ctx.request_repaint();
        });
        Thumb::Loading
    }

    /// Per frame: turn finished previews into textures, keep the memory budget.
    pub fn frame(&mut self) {
        let finished: Vec<_> = std::mem::take(&mut *self.done.lock().unwrap());
        for (key, r) in finished {
            match r {
                Ok(img) => {
                    let bytes = img.width() * img.height() * 4;
                    let tex = self.ctx.load_texture(format!("thumb:{}", key.path), img, TextureOptions::LINEAR);
                    self.tick += 1;
                    self.bytes += bytes;
                    self.cache.insert(key, Slot::Ready { tex, bytes, used: self.tick });
                }
                Err(e) if e.is_empty() => {
                    self.cache.remove(&key);
                }
                Err(e) => {
                    tracing::debug!("thumbnail {}: {e}", key.path);
                    self.cache.insert(key, Slot::Failed);
                }
            }
        }
        self.evict(BUDGET);
    }

    /// Drop the least recently used previews over `budget` bytes.
    fn evict(&mut self, budget: usize) {
        if self.bytes <= budget {
            return;
        }
        let mut ready: Vec<(u64, Key, usize)> = self
            .cache
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Ready { used, bytes, .. } => Some((*used, k.clone(), *bytes)),
                _ => None,
            })
            .collect();
        ready.sort_by_key(|r| r.0);
        for (_, k, b) in ready {
            if self.bytes <= budget * 3 / 4 {
                break;
            }
            self.cache.remove(&k);
            self.bytes -= b;
        }
    }

    pub fn memory(&self) -> usize {
        self.bytes
    }
}

/// Read the whole file (it is small: limits are checked before).
async fn load(fs: &dyn Vfs, path: &str, size: u64, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut r = fs.open_read(path).await.map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(size.min(MAX_LOCAL_FILE) as usize);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(String::new());
        }
        let n = r.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > MAX_LOCAL_FILE {
            return Err("the file is larger than the listing says".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in data {
            c ^= *b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// A tiny PNG claiming to be `w` × `h` (an "image bomb" header).
    fn bomb(w: u32, h: u32) -> Vec<u8> {
        let mut p = png(4, 4);
        // IHDR data starts at byte 16 (8 signature + 4 length + 4 type).
        p[16..20].copy_from_slice(&w.to_be_bytes());
        p[20..24].copy_from_slice(&h.to_be_bytes());
        let crc = crc32(&p[12..29]);
        p[29..33].copy_from_slice(&crc.to_be_bytes());
        p
    }

    #[test]
    fn decodes_and_scales_down() {
        let img = decode(&png(400, 200), 100, MAX_DIM, MAX_ALLOC).unwrap();
        assert_eq!(img.size, [100, 50]);
        let small = decode(&png(20, 10), 100, MAX_DIM, MAX_ALLOC).unwrap();
        assert_eq!(small.size, [20, 10], "never scaled up");
    }

    #[test]
    fn limits_stop_image_bombs() {
        let e = decode(&bomb(60_000, 60_000), 100, MAX_DIM, MAX_ALLOC).unwrap_err();
        assert!(!e.is_empty());
        // Within the dimension limit but over the memory limit.
        assert!(decode(&bomb(9_000, 9_000), 100, MAX_DIM, 1024 * 1024).is_err());
        // Tight dimension limit.
        assert!(decode(&png(300, 300), 100, 200, MAX_ALLOC).is_err());
        assert!(decode(b"not an image", 100, MAX_DIM, MAX_ALLOC).is_err());
    }

    #[test]
    fn which_files_get_previews() {
        let mb = 1024 * 1024;
        assert_eq!(refusal("a.JPG", mb, false, true, false, 2 * mb), None);
        assert!(refusal("a.txt", 10, false, true, false, 2 * mb).is_some());
        assert_eq!(refusal("a.webp", mb, true, true, false, 2 * mb), None);
        assert!(refusal("a.webp", 3 * mb, true, true, false, 2 * mb).is_some(), "over the remote limit");
        assert!(refusal("a.png", 10, true, true, true, 2 * mb).is_some(), "production");
        assert!(refusal("a.png", 10, true, false, false, 2 * mb).is_some(), "remote off");
        assert!(refusal("a.png", 3 * mb, false, false, true, 2 * mb).is_none(), "local ignores remote rules");
        assert!(is_image("x.Gif") && is_image("x.bmp") && !is_image("gif"));
    }
}
