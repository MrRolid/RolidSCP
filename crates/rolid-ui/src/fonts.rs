//! Text font, size and rendering.
//!
//! Built in: Noto Sans Regular 2.004 (Google, SIL Open Font License 1.1), a
//! normal-weight face made for screens and hinted with ttfautohint (egui's own
//! default, the thin Ubuntu Light, is left out of the build), and Hack for
//! monospace text. Optionally a system font picked by
//! fontconfig (`fc-match`). The size is the size of the file list text in
//! whole points; the other text styles, fixed sizes in the code (`fs`) and the
//! list row height (`row_h`) scale with it, and every font size is rounded to
//! whole pixels at the screen scale (a 13.5 px font is drawn blurred).
//!
//! Rendering: how hard the outlines are fitted to the pixel grid
//! (`FontHinting`, egui `HintingTarget`) and how the edge pixels are blended
//! (`FontContrast`, egui `FontColorTransferFunction`). Sub-pixel (color)
//! anti-aliasing is not available in egui.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, FontTweak, TextStyle};
use eframe::epaint::text::{HintingTarget, SmoothHinting};
use eframe::epaint::FontColorTransferFunction;
use rolid_config::{FontContrast, FontHinting};

pub const NOTO: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
/// License text of the built-in font (shipped in the .deb, linked from About).
pub const NOTO_NAME: &str = "Noto Sans 2.004";
pub const NOTO_LICENSE: &str = "SIL Open Font License 1.1";
/// Monospace font (dates, sizes): Hack 3.003, MIT + Bitstream Vera license
/// (assets/fonts/Hack-LICENSE.txt); the same file egui bundles.
pub const HACK: &[u8] = include_bytes!("../assets/fonts/Hack-Regular.ttf");

/// File list text size by default (whole points).
pub const DEFAULT_SIZE: f32 = 14.0;
pub const MIN_SIZE: f32 = 10.0;
pub const MAX_SIZE: f32 = 20.0;
/// The list text size the sizes in the code were made for (`fs(13.5)` is
/// the list text).
const BASE_SIZE: f32 = 13.5;

/// Name of the text font in the font definitions.
const TEXT: &str = "rolid-text";
const MONO: &str = "Hack";

static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0
/// Screen pixels per point the font sizes were rounded for.
static PPP: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

/// Text size ÷ the size the code was made for.
pub fn scale() -> f32 {
    load(&SCALE)
}

/// `pt` rounded to whole screen pixels (at least one).
fn whole_px(pt: f32) -> f32 {
    let ppp = load(&PPP);
    (pt * ppp).round().max(1.0) / ppp
}

/// A font size from the code (made for the 13.5 list text) scaled to the
/// chosen size, in whole pixels.
pub fn fs(pt: f32) -> f32 {
    whole_px(pt * scale())
}

/// Width of a hairline (table separators, the cursor frame): one physical
/// pixel, so it stays sharp at 125 % or 150 % screen scale too.
pub fn hairline() -> f32 {
    1.0 / load(&PPP)
}

/// Proportional font at a scaled size.
pub fn prop(pt: f32) -> FontId {
    FontId::proportional(fs(pt))
}

/// Height of a row in the detailed listing (whole pixels at 1:1).
pub fn row_h() -> f32 {
    (20.0 * scale()).round()
}

/// A text size from the settings file: whole points 10 – 20 (an older
/// 13.5 becomes 14).
pub fn clamp_size(v: f32) -> f32 {
    if v.is_finite() { v.round().clamp(MIN_SIZE, MAX_SIZE) } else { DEFAULT_SIZE }
}

/// A system font for `family` (empty = the default sans-serif) by fontconfig:
/// path, face index and "Family Style".
pub fn system_font(family: &str) -> Result<(PathBuf, u32, String), String> {
    let family = family.trim();
    let pattern = if family.is_empty() { "sans-serif".to_string() } else { family.to_string() };
    let out = std::process::Command::new("fc-match")
        .arg("--format=%{file}\n%{index}\n%{family[0]} %{style[0]}")
        .arg(format!("{pattern}:weight=regular:slant=roman"))
        .output()
        .map_err(|e| tr!("cannot run fc-match (fontconfig): {error}", error = e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let file = lines.next().unwrap_or("").trim();
    if !out.status.success() || file.is_empty() {
        return Err(tr!("fontconfig did not find the font “{pattern}”", pattern = pattern));
    }
    let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    let name = lines.next().unwrap_or(file).trim().to_string();
    Ok((PathBuf::from(file), index, name))
}

/// What `apply` installed: a description for the log and the settings, and
/// why the system font was not used.
pub struct Applied {
    pub desc: String,
    pub error: Option<String>,
}

/// The font settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub system: bool,
    pub family: String,
    pub size: f32,
    pub hinting: FontHinting,
    pub contrast: FontContrast,
}

impl Spec {
    pub fn from_settings(s: &rolid_config::Settings) -> Self {
        Self {
            system: s.font_system,
            family: s.font_family.clone().unwrap_or_default(),
            size: clamp_size(s.font_size.unwrap_or(DEFAULT_SIZE)),
            hinting: s.font_hinting.unwrap_or_default(),
            contrast: s.font_contrast.unwrap_or_default(),
        }
    }
}

/// A font for egui with the hinting setting applied.
fn font_data(bytes: Vec<u8>, index: u32, h: FontHinting) -> FontData {
    let bytes = if h == FontHinting::Strong { without_instructions(bytes) } else { bytes };
    let mut d = FontData::from_owned(bytes).tweak(tweak(h));
    d.index = index;
    d
}

/// The font with its TrueType hinting program hidden, so that skrifa runs its
/// auto-hinter (FreeType "autofit") instead of the font's instructions.
///
/// skrifa, like FreeType, runs instructions in v40 mode, which never moves
/// points horizontally: the vertical stems of "D", "l", "m" keep their
/// fractional position and get a gray column at their edge. The auto-hinter in
/// its normal (not light) mode fits them to whole pixels as well. It is chosen
/// when the font has no `fpgm` / `prep` tables and no glyph instructions
/// (`maxp.maxSizeOfInstructions` = 0): the two table tags are renamed in the
/// table directory and the `maxp` field is zeroed, in memory only; the outlines
/// are untouched. A collection (.ttc) or a font that does not parse comes back
/// unchanged.
fn without_instructions(mut b: Vec<u8>) -> Vec<u8> {
    let u16_at = |b: &[u8], o: usize| b.get(o..o + 2).map(|x| u16::from_be_bytes([x[0], x[1]]) as usize);
    let u32_at = |b: &[u8], o: usize| b.get(o..o + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]) as usize);
    let sfnt = b.get(0..4).is_some_and(|v| v == [0, 1, 0, 0] || v == b"true");
    let Some(n) = u16_at(&b, 4).filter(|_| sfnt) else { return b };
    let mut maxp = None;
    for i in 0..n {
        let rec = 12 + 16 * i;
        let Some(tag) = b.get(rec..rec + 4) else { return b };
        match tag {
            b"fpgm" => b[rec..rec + 4].copy_from_slice(b"xpgm"),
            b"prep" => b[rec..rec + 4].copy_from_slice(b"xrep"),
            b"maxp" => maxp = u32_at(&b, rec + 8),
            _ => {}
        }
    }
    // maxp 1.0: maxSizeOfInstructions at byte 26 (version 0.5 has no such field).
    if let Some(m) = maxp {
        if u32_at(&b, m) == Some(0x0001_0000) && b.len() >= m + 28 {
            b[m + 26..m + 28].copy_from_slice(&[0, 0]);
        }
    }
    b
}

/// Glyph fitting for the hinting setting.
///
/// Noto Sans and Hack carry their own TrueType instructions (ttfautohint):
/// skrifa runs them in FreeType's v40 mode, which fits only vertically, so a
/// vertical stem 1.2 px wide keeps a light gray column next to it. "Strong"
/// hides the instructions and asks the auto-hinter for its Mono target, which
/// rounds stem positions and widths to whole pixels (measured: the gray share
/// of the file names 0.60 → 0.47, "D" in Hack one black column). The text
/// gets a little lighter, as the stems are thinner.
fn tweak(h: FontHinting) -> FontTweak {
    let (hinting, hinting_target) = match h {
        // With the instructions hidden (see `font_data`) this is the auto-hinter
        // in its strongest mode: stems and their widths on whole pixels.
        FontHinting::Strong => (true, HintingTarget::Mono),
        // The font's own instructions; asymmetric rendering gives measurably
        // fewer gray edge pixels than egui's default (symmetric).
        FontHinting::Normal => (
            true,
            HintingTarget::Smooth(SmoothHinting { light: false, symmetric_rendering: false, preserve_linear_metrics: false }),
        ),
        FontHinting::Off => (false, HintingTarget::default()),
    };
    FontTweak { hinting: Some(hinting), hinting_target, ..Default::default() }
}

/// Edge blending for the contrast setting: alpha = coverage^gamma, a lower
/// gamma darkens the half-covered edge pixels.
pub fn transfer(c: FontContrast, dark: bool) -> FontColorTransferFunction {
    use FontColorTransferFunction as F;
    match (c, dark) {
        (FontContrast::Normal, false) => F::LIGHT_MODE_DEFAULT,
        (FontContrast::Normal, true) => F::DARK_MODE_DEFAULT,
        (FontContrast::Higher, false) => F::Gamma(0.8),
        (FontContrast::Higher, true) => F::Gamma(0.35),
        (FontContrast::Highest, false) => F::Gamma(0.65),
        (FontContrast::Highest, true) => F::Gamma(0.25),
    }
}

/// Text styles at the current scale, in whole pixels.
fn set_styles(ctx: &egui::Context, contrast: Option<FontContrast>) {
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(fs(9.5))),
            (TextStyle::Body, FontId::proportional(fs(13.0))),
            (TextStyle::Button, FontId::proportional(fs(13.0))),
            (TextStyle::Heading, FontId::proportional(fs(18.0))),
            (TextStyle::Monospace, FontId::monospace(fs(12.5))),
        ]
        .into();
        if let Some(c) = contrast {
            s.visuals.text_options.color_transfer_function = transfer(c, s.visuals.dark_mode);
        }
    });
}

/// Every frame: when the screen scale changes (another monitor, Ctrl+plus),
/// round the font sizes again for it.
pub fn follow_screen_scale(ctx: &egui::Context) {
    let ppp = ctx.pixels_per_point();
    if ppp.is_finite() && ppp > 0.0 && (ppp - load(&PPP)).abs() > 1e-4 {
        PPP.store(ppp.to_bits(), Ordering::Relaxed);
        set_styles(ctx, None);
    }
}

/// Install the font, the text sizes and the rendering (at start and after
/// the settings change).
pub fn apply(ctx: &egui::Context, spec: &Spec) -> Applied {
    let size = clamp_size(spec.size);
    SCALE.store((size / BASE_SIZE).to_bits(), Ordering::Relaxed);
    let ppp = ctx.pixels_per_point();
    if ppp.is_finite() && ppp > 0.0 {
        PPP.store(ppp.to_bits(), Ordering::Relaxed);
    }
    let mut error = None;
    let mut chosen = None;
    if spec.system {
        match system_font(&spec.family).and_then(|(path, index, name)| {
            let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            // A font egui cannot read would only show squares.
            if skrifa_ok(&data, index) {
                Ok((data, index, format!("{name} ({})", path.display())))
            } else {
                Err(tr!("{path}: cannot load the font", path = &path))
            }
        }) {
            Ok(x) => chosen = Some(x),
            Err(e) => error = Some(e),
        }
    }
    let (data, desc) = match chosen {
        Some((bytes, index, desc)) => (font_data(bytes, index, spec.hinting), tr!("system: {font}", font = desc)),
        None => (font_data(NOTO.to_vec(), 0, spec.hinting), tr!("built-in: {font}", font = NOTO_NAME)),
    };
    // egui's own fonts are not built in (no Ubuntu Light): proportional text in
    // our font, monospace in Hack falling back to it; symbols the two lack come
    // from the system fonts (eframe "system_fonts").
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert(TEXT.into(), Arc::new(data));
    defs.font_data.insert(MONO.into(), Arc::new(font_data(HACK.to_vec(), 0, spec.hinting)));
    defs.families.insert(FontFamily::Proportional, vec![TEXT.into()]);
    defs.families.insert(FontFamily::Monospace, vec![MONO.into(), TEXT.into()]);
    crate::icons::add_to(&mut defs);
    ctx.set_fonts(defs);
    set_styles(ctx, Some(spec.contrast));
    let desc = tr!(
        "{font}, size {size} pt ({px} px), hinting {hinting}, contrast {contrast}",
        font = desc,
        size = size.to_string(),
        px = (fs(BASE_SIZE) * load(&PPP)).to_string(),
        hinting = rolid_i18n::tc_str("hinting", spec.hinting.label()).to_lowercase(),
        contrast = rolid_i18n::tc_str("contrast", spec.contrast.label()).to_lowercase()
    );
    Applied { desc, error }
}

/// The face can be read (the same parser egui uses).
fn skrifa_ok(data: &[u8], index: u32) -> bool {
    use skrifa::raw::TableProvider;
    skrifa::FontRef::from_index(data, index).is_ok_and(|f| f.head().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scale is global: the tests that change it take turns.
    pub static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn sizes_and_the_built_in_font() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(skrifa_ok(NOTO, 0));
        assert!(!skrifa_ok(b"not a font", 0));
        assert_eq!(clamp_size(13.5), 14.0);
        assert_eq!(clamp_size(13.4), 13.0);
        assert_eq!(clamp_size(9.0), MIN_SIZE);
        assert_eq!(clamp_size(f32::NAN), DEFAULT_SIZE);
        let ctx = egui::Context::default();
        let spec = |size| Spec { system: false, family: String::new(), size, hinting: FontHinting::default(), contrast: FontContrast::default() };
        let a = apply(&ctx, &spec(18.0));
        assert!(a.desc.contains("Noto Sans") && a.error.is_none(), "{}", a.desc);
        assert!((scale() - 18.0 / BASE_SIZE).abs() < 1e-6);
        assert_eq!(row_h(), 27.0);
        apply(&ctx, &spec(99.0));
        assert_eq!(scale(), MAX_SIZE / BASE_SIZE);
        // Every size from the code lands on whole pixels, at 1:1 and at 125 %.
        for ppp in [1.0f32, 1.25, 1.5, 2.0] {
            PPP.store(ppp.to_bits(), Ordering::Relaxed);
            for size in 10..=20 {
                SCALE.store((size as f32 / BASE_SIZE).to_bits(), Ordering::Relaxed);
                assert_eq!(fs(BASE_SIZE) * ppp, (size as f32 * ppp).round(), "{size} pt at {ppp}");
                for pt in [9.5, 12.5, 13.0, 18.0, 11.0] {
                    let px = fs(pt) * ppp;
                    assert!((px - px.round()).abs() < 1e-3, "{pt} at {size} pt, {ppp}: {px}");
                }
            }
        }
        PPP.store(1.0f32.to_bits(), Ordering::Relaxed);
        let a = apply(&ctx, &spec(DEFAULT_SIZE));
        assert!(a.desc.contains("14 pt (14 px)"), "{}", a.desc);
        assert_eq!((fs(13.5), row_h()), (14.0, 21.0));
        // Darker edges for higher contrast, in both themes.
        for dark in [false, true] {
            let a = |c| transfer(c, dark).alpha_from_coverage(0.5);
            assert!(a(FontContrast::Normal) < a(FontContrast::Higher) && a(FontContrast::Higher) < a(FontContrast::Highest), "{dark}");
        }
        // Czech, Polish and Nordic letters are in the built-in font.
        let f = skrifa::FontRef::new(NOTO).unwrap();
        use skrifa::MetadataProvider;
        let cmap = f.charmap();
        for c in "ěščřžýáíéůúťďňĚŠČŘŽŮąćęłńóśźżĄĆĘŁŃÓŚŹŻæøåÆØÅäöüßÄÖÜñÑçÇ".chars() {
            assert!(cmap.map(c).is_some(), "{c}");
        }
    }

    #[test]
    fn system_font_by_fontconfig() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // fontconfig may be missing on a build machine: then the error must be readable.
        match system_font("") {
            Ok((path, _, name)) => {
                assert!(path.exists(), "{path:?}");
                assert!(!name.is_empty());
            }
            Err(e) => assert!(e.contains("fc-match") || e.contains("fontconfig"), "{e}"),
        }
        let ctx = egui::Context::default();
        let spec = Spec { system: true, family: String::new(), size: DEFAULT_SIZE, hinting: FontHinting::Normal, contrast: FontContrast::Normal };
        let a = apply(&ctx, &spec);
        assert!(a.desc.starts_with("system") || a.error.is_some(), "{}", a.desc);
    }

    /// Text painted at a fractional position, aligned right or centered (the
    /// size column, the dates, the list names), ends up on whole physical
    /// pixels at every screen scale: egui rounds the galley position
    /// (`round_text_to_pixels`) and every glyph inside it, and lines
    /// (`round_line_segments_to_pixels`).
    #[test]
    fn text_and_lines_land_on_whole_pixels() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use egui::{epaint::Primitive, pos2, Align2, Color32, Stroke};
        for ppp in [1.0f32, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            for hinting in FontHinting::ALL {
                let spec = Spec { system: false, family: String::new(), size: DEFAULT_SIZE, hinting, contrast: FontContrast::Normal };
                let mut out = None;
                for _ in 0..3 {
                    apply(&ctx, &spec);
                    let mut o = ctx.run_ui(Default::default(), |ui| {
                        let p = ui.painter();
                        p.text(pos2(100.37, 20.61), Align2::RIGHT_CENTER, "<DIR>", FontId::monospace(fs(12.5)), Color32::BLACK);
                        p.text(pos2(140.21, 20.61), Align2::LEFT_CENTER, "09.11.2026 10:58", FontId::monospace(fs(12.5)), Color32::BLACK);
                        p.text(pos2(77.77, 47.3), Align2::CENTER_CENTER, "Příliš žluťoučký kůň", prop(13.5), Color32::BLACK);
                        p.vline(400.33, egui::Rangef::new(5.2, 60.7), Stroke::new(hairline(), Color32::BLACK));
                    });
                    o.textures_delta.clear(); // no renderer here
                    out = Some(o);
                }
                let out = out.unwrap();
                let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
                let mut n = 0;
                let mut line = Vec::new();
                for prim in prims {
                    let Primitive::Mesh(mesh) = prim.primitive else { continue };
                    for v in &mesh.vertices {
                        let (x, y) = (v.pos.x * ppp, v.pos.y * ppp);
                        if v.pos.x > 390.0 {
                            line.push((x, v.color.a()));
                            continue;
                        }
                        for px in [x, y] {
                            assert!((px - px.round()).abs() < 0.02, "{hinting:?} at {ppp}: text vertex at {px} px");
                        }
                        n += 1;
                    }
                }
                // The 1 px separator: its opaque center runs through the middle of
                // one pixel column and fades to nothing at the neighboring
                // centers, so exactly that column is covered.
                assert!(!line.is_empty(), "line painted at {ppp}");
                for (x, a) in &line {
                    let frac = x - x.floor();
                    assert!((frac - 0.5).abs() < 0.02, "line vertex at {x} px (scale {ppp})");
                    let center = line.iter().find(|(_, a)| *a == 255).map(|(x, _)| *x).unwrap();
                    assert!(*a == 255 && *x == center || *a == 0 && ((x - center).abs() - 1.0).abs() < 0.02, "line at {ppp}: {line:?}");
                }
                assert!(n > 100, "text was painted ({n} vertices)");
            }
        }
        PPP.store(1.0f32.to_bits(), Ordering::Relaxed);
    }

    #[test]
    fn hinting_instructions_hidden_only_for_strong() {
        use skrifa::raw::TableProvider;
        for font in [NOTO, HACK] {
            let f = skrifa::FontRef::new(font).unwrap();
            assert!(f.maxp().unwrap().max_size_of_instructions().unwrap_or(0) > 0);
            let b = without_instructions(font.to_vec());
            let g = skrifa::FontRef::new(&b).unwrap();
            assert_eq!(g.maxp().unwrap().max_size_of_instructions(), Some(0));
            assert!(g.data_for_tag(skrifa::Tag::new(b"fpgm")).is_none() && g.data_for_tag(skrifa::Tag::new(b"prep")).is_none());
            assert_eq!(g.glyf().unwrap().offset_data().len(), f.glyf().unwrap().offset_data().len(), "outlines untouched");
            assert_eq!(b.len(), font.len());
        }
        // Not a plain font: unchanged.
        assert_eq!(without_instructions(b"ttcf1234".to_vec()), b"ttcf1234");
    }
}
