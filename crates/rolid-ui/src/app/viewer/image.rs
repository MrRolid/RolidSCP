//! The image mode of the viewer: the file is read in the background (from a
//! server only up to a set size, with progress) and decoded there with the
//! same limits against image bombs as the previews. Animated GIF and WebP
//! play; the EXIF orientation is applied; rotating is for the view only.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, Vec2};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, RgbaImage};
use rolid_core::Vfs;
use rolid_i18n::tr;

use crate::pane::thumbs::{MAX_ALLOC, MAX_DIM};

/// Most bytes read of a local image.
pub const MAX_LOCAL: u64 = 512 << 20;
/// Most frames of an animation kept.
pub const MAX_FRAMES: usize = 2000;

/// A decoded image.
pub struct Decoded {
    /// Frames and how long each is shown (one frame = a still image).
    pub frames: Vec<(ColorImage, Duration)>,
    /// Size of the picture (after the EXIF orientation).
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
    /// Made smaller to fit the largest texture of the graphics card.
    pub scaled: bool,
    /// An animation cut short (too many frames or too much memory).
    pub truncated: bool,
}

/// The name of a format for people.
fn format_name(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Png => "PNG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        ImageFormat::Bmp => "BMP",
        _ => "?",
    }
}

fn limits(max_dim: u32, max_alloc: u64) -> image::Limits {
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(max_dim);
    lim.max_image_height = Some(max_dim);
    lim.max_alloc = Some(max_alloc);
    lim
}

fn color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

/// Smaller when a side is over `max_side` (the texture limit).
fn fit_side(img: RgbaImage, max_side: u32) -> (RgbaImage, bool) {
    let (w, h) = img.dimensions();
    if w <= max_side && h <= max_side {
        return (img, false);
    }
    let k = max_side as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * k) as u32).max(1), ((h as f64 * k) as u32).max(1));
    (image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle), true)
}

/// Decode `bytes` (an image file); `max_side` is the largest texture side.
pub fn decode(bytes: &[u8], max_dim: u32, max_alloc: u64, max_side: u32) -> Result<Decoded, String> {
    // The size from the header first: a picture too large is refused before decoding.
    if let Ok((w, h)) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string()).and_then(|r| r.into_dimensions().map_err(|e| e.to_string())) {
        if w > max_dim || h > max_dim {
            return Err(tr!("The image is too large ({w} × {h} px; at most {max} px a side).", w = w, h = h, max = max_dim));
        }
    }
    decode_inner(bytes, max_dim, max_alloc, max_side).map_err(|e| match e {
        Msg::Ours(m) => m,
        Msg::Lib(e) => tr!("The file is damaged or cannot be read as an image: {error}", error = e),
    })
}

/// Why decoding failed: our own message, or the decoder's (wrapped).
enum Msg {
    Ours(String),
    Lib(String),
}

fn lib(e: image::ImageError) -> Msg {
    Msg::Lib(e.to_string())
}

fn decode_inner(bytes: &[u8], max_dim: u32, max_alloc: u64, max_side: u32) -> Result<Decoded, Msg> {
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| Msg::Lib(e.to_string()))?;
    let format = reader.format().ok_or_else(|| Msg::Ours(tr!("Not an image in a known format.")))?;
    let name = format_name(format);
    if name == "?" {
        return Err(Msg::Ours(tr!("Not an image in a known format.")));
    }
    // Animations: every frame (each of the full size) counts against the memory limit.
    let anim: Option<(Vec<image::Frame>, bool)> = match format {
        ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(lib)?;
            d.set_limits(limits(max_dim, max_alloc)).map_err(lib)?;
            Some(collect_frames(d.into_frames(), max_alloc)?)
        }
        ImageFormat::WebP => {
            let d = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(lib)?;
            if d.has_animation() {
                let (w, h) = d.dimensions();
                if w > max_dim || h > max_dim {
                    return Err(Msg::Ours(tr!("The image is too large ({w} × {h} px).", w = w, h = h)));
                }
                Some(collect_frames(d.into_frames(), max_alloc)?)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some((frames, truncated)) = anim {
        let mut out = Vec::new();
        let mut scaled = false;
        let (mut width, mut height) = (0, 0);
        for f in frames {
            let delay = Duration::from_millis({
                let (n, d) = f.delay().numer_denom_ms();
                let ms = n as u64 / d.max(1) as u64;
                // As browsers do: no delay means a tenth of a second.
                if ms < 20 { 100 } else { ms }
            });
            let img = f.into_buffer();
            width = img.width();
            height = img.height();
            let (img, s) = fit_side(img, max_side);
            scaled |= s;
            out.push((color_image(&img), delay));
        }
        if out.is_empty() {
            return Err(Msg::Ours(tr!("The image has no frames.")));
        }
        return Ok(Decoded { frames: out, width, height, format: name, scaled, truncated });
    }
    let mut dec = reader.into_decoder().map_err(lib)?;
    dec.set_limits(limits(max_dim, max_alloc)).map_err(lib)?;
    // The picture and its RGBA copy must fit in the memory limit.
    if dec.total_bytes().saturating_mul(2) > max_alloc {
        return Err(Msg::Ours(tr!("The image needs too much memory.")));
    }
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec).map_err(lib)?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    let (rgba, scaled) = fit_side(rgba, max_side);
    Ok(Decoded { frames: vec![(color_image(&rgba), Duration::ZERO)], width, height, format: name, scaled, truncated: false })
}

/// Frames of an animation up to [`MAX_FRAMES`] and `max_alloc` bytes;
/// `true` = cut short.
fn collect_frames(frames: image::Frames<'_>, max_alloc: u64) -> Result<(Vec<image::Frame>, bool), Msg> {
    let mut out = Vec::new();
    let mut used = 0u64;
    for f in frames {
        let f = match f {
            Ok(f) => f,
            // A damaged frame after good ones: show those.
            Err(_) if !out.is_empty() => break,
            Err(e) => return Err(lib(e)),
        };
        used += f.buffer().len() as u64;
        if used > max_alloc || out.len() >= MAX_FRAMES {
            if out.is_empty() {
                return Err(Msg::Ours(tr!("The image needs too much memory.")));
            }
            return Ok((out, true));
        }
        out.push(f);
    }
    Ok((out, false))
}

/// What the loading is at.
enum Load {
    Reading { got: u64, total: u64 },
    Decoding,
    Ready(Decoded),
    /// The frames went to textures.
    Shown,
    Failed(String),
}

pub struct ImageView {
    load: Arc<Mutex<Load>>,
    cancel: Arc<AtomicBool>,
    textures: Vec<(TextureHandle, Duration)>,
    /// Size, format, frames (kept after the frames went to textures).
    pub info: Option<(u32, u32, &'static str, usize, bool, bool)>,
    /// `None` = fit to the window.
    pub zoom: Option<f32>,
    /// The scale last drawn (for zooming from "fit").
    shown_scale: f32,
    offset: Vec2,
    /// Quarter turns to the right (the view only).
    pub turns: u8,
    started: Instant,
    frame: usize,
    frame_at: Instant,
}

impl Drop for ImageView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl ImageView {
    /// Start reading and decoding `path` (`limit` = most bytes read).
    pub fn start(rt: &tokio::runtime::Handle, fs: Arc<dyn Vfs>, path: String, size: u64, limit: u64, max_side: u32, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        let load = Arc::new(Mutex::new(Load::Reading { got: 0, total: size }));
        let cancel = Arc::new(AtomicBool::new(false));
        if size > limit {
            *load.lock().unwrap() = Load::Failed(tr!("The image is larger ({size}) than the limit for viewing ({limit}); the limit is in Settings → Viewer (F3).", size = rolid_i18n::size(size), limit = rolid_i18n::size(limit)));
        } else {
            let (l2, c2) = (load.clone(), cancel.clone());
            rt.spawn(async move {
                let r = read_all(fs.as_ref(), &path, size, limit, &c2, &l2, &repaint).await;
                if c2.load(Ordering::SeqCst) {
                    return;
                }
                match r {
                    Err(e) => *l2.lock().unwrap() = Load::Failed(e),
                    Ok(bytes) => {
                        *l2.lock().unwrap() = Load::Decoding;
                        repaint();
                        let r = tokio::task::spawn_blocking(move || decode(&bytes, MAX_DIM, MAX_ALLOC, max_side)).await;
                        *l2.lock().unwrap() = match r {
                            Ok(Ok(d)) => Load::Ready(d),
                            Ok(Err(e)) => Load::Failed(e),
                            Err(e) => Load::Failed(e.to_string()),
                        };
                    }
                }
                repaint();
            });
        }
        ImageView {
            load,
            cancel,
            textures: Vec::new(),
            info: None,
            zoom: None,
            shown_scale: 1.0,
            offset: Vec2::ZERO,
            turns: 0,
            started: Instant::now(),
            frame: 0,
            frame_at: Instant::now(),
        }
    }

    /// Reading progress `(got, total)`, or `None` when read.
    pub fn progress(&self) -> Option<(u64, u64)> {
        match &*self.load.lock().unwrap() {
            Load::Reading { got, total } => Some((*got, *total)),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<String> {
        match &*self.load.lock().unwrap() {
            Load::Failed(e) => Some(e.clone()),
            _ => None,
        }
    }

    pub fn fit(&mut self) {
        self.zoom = None;
        self.offset = Vec2::ZERO;
    }

    pub fn actual_size(&mut self) {
        self.zoom = Some(1.0);
        self.offset = Vec2::ZERO;
    }

    /// Zoom by `k` around the middle of the view.
    pub fn zoom_by(&mut self, k: f32) {
        let z = (self.zoom.unwrap_or(self.shown_scale) * k).clamp(0.01, 64.0);
        self.offset *= z / self.zoom.unwrap_or(self.shown_scale);
        self.zoom = Some(z);
    }

    pub fn rotate(&mut self, right: bool) {
        self.turns = (self.turns + if right { 1 } else { 3 }) % 4;
        self.offset = Vec2::ZERO;
    }

    /// The scale shown now (for the status bar).
    pub fn scale(&self) -> f32 {
        self.shown_scale
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(full, Sense::click_and_drag());
        let weak = ui.visuals().weak_text_color();
        // The decoded frames become textures once.
        let ready = {
            let mut l = self.load.lock().unwrap();
            if let Load::Ready(_) = &*l {
                let Load::Ready(d) = std::mem::replace(&mut *l, Load::Decoding) else { unreachable!() };
                drop(l);
                let n = d.frames.len();
                self.info = Some((d.width, d.height, d.format, n, d.scaled, d.truncated));
                self.textures = d.frames.into_iter().enumerate().map(|(i, (img, delay))| (ui.ctx().load_texture(format!("viewer-image-{i}"), img, egui::TextureOptions::LINEAR), delay)).collect();
                self.started = Instant::now();
                self.frame_at = Instant::now();
                self.frame = 0;
                *self.load.lock().unwrap() = Load::Shown;
            }
            !self.textures.is_empty()
        };
        if !ready {
            let msg = match &*self.load.lock().unwrap() {
                Load::Reading { got, total } => {
                    let p = if *total == 0 { 0.0 } else { *got as f32 / *total as f32 };
                    let w = 320.0f32.min(full.width() - 40.0);
                    let bar = Rect::from_center_size(full.center() + Vec2::new(0.0, 24.0), Vec2::new(w, 8.0));
                    ui.painter().rect_filled(bar, 3.0, ui.visuals().widgets.inactive.bg_fill);
                    ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(w * p.clamp(0.0, 1.0), 8.0)), 3.0, ui.visuals().selection.bg_fill);
                    tr!("Reading the image… {got} of {total}", got = rolid_i18n::size(*got), total = rolid_i18n::size(*total))
                }
                Load::Decoding => tr!("Decoding the image…"),
                Load::Failed(e) => e.clone(),
                Load::Ready(_) | Load::Shown => String::new(),
            };
            ui.painter().text(full.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(crate::fonts::fs(14.0)), weak);
            return;
        }
        // Animation.
        if self.textures.len() > 1 {
            let delay = self.textures[self.frame].1;
            let since = self.frame_at.elapsed();
            if since >= delay {
                self.frame = (self.frame + 1) % self.textures.len();
                self.frame_at = Instant::now();
            }
            ui.ctx().request_repaint_after(self.textures[self.frame].1.saturating_sub(self.frame_at.elapsed()));
        }
        let tex = self.textures[self.frame].0.clone();
        let (w, h) = {
            let s = tex.size_vec2();
            // The picture's own size (the texture may be smaller).
            let (iw, ih) = self.info.map_or((s.x, s.y), |i| (i.0 as f32, i.1 as f32));
            (iw, ih)
        };
        let turned = self.turns % 2 == 1;
        let (vw, vh) = if turned { (h, w) } else { (w, h) };
        let fit = (full.width() / vw).min(full.height() / vh).min(1.0);
        let scale = self.zoom.unwrap_or(fit);
        // Zoom with the wheel around the pointer; drag to move.
        if resp.hovered() {
            let (scroll, pointer) = ui.input(|i| (i.smooth_scroll_delta.y + i.smooth_scroll_delta.x * 0.0, i.pointer.hover_pos()));
            if scroll != 0.0 {
                let k = (scroll / 200.0).exp();
                let new = (scale * k).clamp(0.01, 64.0);
                if let Some(p) = pointer {
                    let c = full.center() + self.offset;
                    let v = (p - c) / scale;
                    self.offset = (p - v * new) - full.center();
                }
                self.zoom = Some(new);
            }
        }
        if resp.dragged() {
            self.offset += resp.drag_delta();
        }
        if resp.double_clicked() {
            if self.zoom.is_some() { self.fit() } else { self.actual_size() }
        }
        let scale = self.zoom.unwrap_or(fit);
        self.shown_scale = scale;
        let (dw, dh) = (vw * scale, vh * scale);
        // Keep the picture in view.
        let lim = Vec2::new(((dw - full.width()) / 2.0).max(0.0), ((dh - full.height()) / 2.0).max(0.0));
        self.offset = Vec2::new(self.offset.x.clamp(-lim.x, lim.x), self.offset.y.clamp(-lim.y, lim.y));
        let center = full.center() + self.offset;
        // The unturned rectangle, turned around its middle.
        let rect = Rect::from_center_size(center, Vec2::new(w * scale, h * scale));
        let painter = ui.painter_at(full);
        // A checkerboard behind transparent pictures.
        let shown = Rect::from_center_size(center, Vec2::new(dw, dh)).intersect(full);
        checker(&painter, shown, ui.visuals().dark_mode);
        let angle = self.turns as f32 * std::f32::consts::FRAC_PI_2;
        let img = egui::Image::new((tex.id(), Vec2::new(w * scale, h * scale))).rotate(angle, Vec2::splat(0.5));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full));
        child.set_clip_rect(full);
        img.paint_at(&child, rect);
    }
}

fn checker(painter: &egui::Painter, r: Rect, dark: bool) {
    let (a, b) = if dark { (Color32::from_gray(40), Color32::from_gray(52)) } else { (Color32::from_gray(235), Color32::from_gray(250)) };
    painter.rect_filled(r, 0.0, a);
    let s = 12.0;
    let mut y = r.top();
    let mut row = 0;
    // Only when the area is not huge (a few thousand squares).
    if (r.width() / s) * (r.height() / s) > 20_000.0 {
        return;
    }
    while y < r.bottom() {
        let mut x = r.left() + if row % 2 == 1 { s } else { 0.0 };
        while x < r.right() {
            painter.rect_filled(Rect::from_min_max(Pos2::new(x, y), Pos2::new((x + s).min(r.right()), (y + s).min(r.bottom()))), 0.0, b);
            x += 2.0 * s;
        }
        y += s;
        row += 1;
    }
}

/// Read the whole file with progress (at most `limit` bytes).
async fn read_all(fs: &dyn Vfs, path: &str, size: u64, limit: u64, cancel: &AtomicBool, load: &Mutex<Load>, repaint: &Arc<dyn Fn() + Send + Sync>) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut r = fs.open_read(path).await.map_err(|e| rolid_i18n::t(&e.text()))?;
    let mut out = Vec::with_capacity(size.min(limit) as usize);
    let mut buf = vec![0u8; 256 * 1024];
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(String::new());
        }
        let n = r.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > limit {
            return Err(tr!("The image is larger than the limit for viewing ({limit}).", limit = rolid_i18n::size(limit)));
        }
        if last.elapsed() > Duration::from_millis(100) {
            last = Instant::now();
            *load.lock().unwrap() = Load::Reading { got: out.len() as u64, total: size.max(out.len() as u64) };
            repaint();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(img: &DynamicImage, f: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, f).unwrap();
        out.into_inner()
    }

    fn sample(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 0, 255]);
        }
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn decodes_all_formats() {
        let img = sample(40, 20);
        for (f, name) in [(ImageFormat::Png, "PNG"), (ImageFormat::Jpeg, "JPEG"), (ImageFormat::Bmp, "BMP"), (ImageFormat::Gif, "GIF"), (ImageFormat::WebP, "WebP")] {
            let src = if f == ImageFormat::Jpeg { DynamicImage::ImageRgb8(img.to_rgb8()) } else { img.clone() };
            let d = decode(&encode(&src, f), MAX_DIM, MAX_ALLOC, 8192).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((d.width, d.height, d.format, d.frames.len()), (40, 20, name, 1), "{name}");
            assert_eq!(d.frames[0].0.size, [40, 20]);
        }
        assert!(decode(b"<svg></svg>", MAX_DIM, MAX_ALLOC, 8192).is_err());
    }

    #[test]
    fn animated_gif_has_its_frames_and_delays() {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for i in 0..5u32 {
                let mut f = RgbaImage::new(8, 8);
                f.put_pixel(i, 0, image::Rgba([255, 0, 0, 255]));
                enc.encode_frame(image::Frame::from_parts(f, 0, 0, image::Delay::from_numer_denom_ms(if i == 0 { 0 } else { 70 }, 1))).unwrap();
            }
        }
        let d = decode(&out, MAX_DIM, MAX_ALLOC, 8192).unwrap();
        assert_eq!(d.frames.len(), 5);
        assert_eq!(d.frames[0].1, Duration::from_millis(100));
        assert_eq!(d.frames[1].1, Duration::from_millis(70));
        assert!(!d.truncated);
        // Too little memory for all frames: the first ones are shown.
        let d = decode(&out, MAX_DIM, 8 * 8 * 4 * 2, 8192).unwrap();
        assert_eq!(d.frames.len(), 2);
        assert!(d.truncated);
    }

    #[test]
    fn image_bombs_are_refused_and_huge_pictures_scaled() {
        let big = encode(&sample(3000, 200), ImageFormat::Png);
        assert!(decode(&big, 2000, MAX_ALLOC, 8192).is_err());
        assert!(decode(&big, MAX_DIM, 1 << 20, 8192).is_err());
        let d = decode(&big, MAX_DIM, MAX_ALLOC, 1000).unwrap();
        assert!(d.scaled);
        assert_eq!((d.width, d.height), (3000, 200));
        assert_eq!(d.frames[0].0.size, [1000, 66]);
    }

    #[test]
    fn exif_orientation_is_applied() {
        // A JPEG with an EXIF block saying "turned 90° to the right" (orientation 6).
        let jpeg = encode(&DynamicImage::ImageRgb8(sample(40, 20).to_rgb8()), ImageFormat::Jpeg);
        let exif: Vec<u8> = {
            let mut e = b"Exif\0\0".to_vec();
            e.extend_from_slice(b"MM\0\x2a\0\0\0\x08"); // big endian TIFF, IFD at 8
            e.extend_from_slice(&[0, 1]); // one entry
            e.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]); // Orientation = 6
            e.extend_from_slice(&[0, 0, 0, 0]);
            e
        };
        let mut with = jpeg[..2].to_vec(); // SOI
        with.extend_from_slice(&[0xFF, 0xE1]);
        with.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        with.extend_from_slice(&exif);
        with.extend_from_slice(&jpeg[2..]);
        let d = decode(&with, MAX_DIM, MAX_ALLOC, 8192).unwrap();
        assert_eq!((d.width, d.height), (20, 40));
    }
}
