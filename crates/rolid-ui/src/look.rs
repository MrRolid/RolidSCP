//! Look of dialogs and popups: a dark border around every window, popup and
//! menu (the soft shadow stays), black text with darker hints (contrast ≥ 7:1,
//! the same inverted in dark mode), and one kind of dialog button – taller,
//! with a colored icon per action; the Enter button has a thicker border
//! (inspired by Double Commander). Text is hinted and drawn on whole pixels.

use eframe::egui::{self, Color32, Response, RichText, Stroke, Theme, Ui, Vec2};

use crate::icons;

/// Text and hints in light mode.
pub const TEXT_LIGHT: Color32 = Color32::BLACK;
pub const WEAK_LIGHT: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
/// The same contrast in dark mode.
pub const TEXT_DARK: Color32 = Color32::WHITE;
pub const WEAK_DARK: Color32 = Color32::from_rgb(0xb8, 0xb8, 0xb8);
/// Border of windows, dialogs, popups and menus.
pub const BORDER_WIDTH: f32 = 1.5;

/// Install the look for both themes.
pub fn apply(ctx: &egui::Context) {
    ctx.style_mut_of(Theme::Light, |s| tune(&mut s.visuals, false));
    ctx.style_mut_of(Theme::Dark, |s| tune(&mut s.visuals, true));
}

fn tune(v: &mut egui::Visuals, dark: bool) {
    let (text, weak, border) = if dark { (TEXT_DARK, WEAK_DARK, TEXT_DARK) } else { (TEXT_LIGHT, WEAK_LIGHT, TEXT_LIGHT) };
    v.window_stroke = Stroke::new(BORDER_WIDTH, border);
    v.widgets.noninteractive.fg_stroke.color = text;
    v.widgets.inactive.fg_stroke.color = text;
    v.widgets.hovered.fg_stroke.color = text;
    v.widgets.active.fg_stroke.color = text;
    v.weak_text_color = Some(weak);
    same_geometry(&mut v.widgets);
    // Sharp text: glyphs hinted to the pixel grid, no sub-pixel positions
    // (egui renders each glyph at four offsets otherwise, which blurs it).
    v.text_options.font_hinting = true;
    v.text_options.subpixel_binning = false;
}

/// Hovering or pressing may change only colors, never sizes. egui gives an
/// inactive widget no outline and a hovered one a 1 px outline and shrinks the
/// inner margin to match, which holds only while a widget keeps egui's own
/// stroke: a button with its own stroke (the selected row of a list, the
/// default dialog button, tabs) kept the margin shrunk on hover and so changed
/// size. Every state gets the same outline width (invisible when inactive)
/// and no expansion.
pub(crate) fn same_geometry(w: &mut egui::style::Widgets) {
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.expansion = 0.0;
        if s.bg_stroke.width != OUTLINE {
            if s.bg_stroke.width <= 0.0 {
                s.bg_stroke.color = Color32::TRANSPARENT;
            }
            s.bg_stroke.width = OUTLINE;
        }
    }
}

/// The outline width of every widget state.
const OUTLINE: f32 = 1.0;

/// What a dialog button does – decides its icon and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Move (F6), Download, Lock, Info: planned actions
pub enum Role {
    Copy,
    Upload,
    Download,
    Move,
    Delete,
    Cancel,
    /// Close / Not now / Back – leaves without doing anything, not a "no".
    Close,
    Ok,
    Save,
    Connect,
    Overwrite,
    Open,
    Edit,
    Search,
    Folder,
    NewFolder,
    Unlock,
    Lock,
    Trust,
    Send,
    Info,
    Pack,
    Unpack,
}

pub const GREEN: Color32 = Color32::from_rgb(0x1b, 0x7a, 0x33);
pub const BLUE: Color32 = Color32::from_rgb(0x1f, 0x5f, 0xbf);
pub const RED: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28);
pub const ORANGE: Color32 = Color32::from_rgb(0xc2, 0x58, 0x00);
const GREEN_D: Color32 = Color32::from_rgb(0x6c, 0xd6, 0x86);
const BLUE_D: Color32 = Color32::from_rgb(0x84, 0xb4, 0xff);
const RED_D: Color32 = Color32::from_rgb(0xff, 0x7a, 0x7a);
const ORANGE_D: Color32 = Color32::from_rgb(0xff, 0xa8, 0x4d);

impl Role {
    /// Icon and color.
    pub fn icon(self, dark: bool) -> (&'static str, Color32) {
        let (green, blue, red, orange) =
            if dark { (GREEN_D, BLUE_D, RED_D, ORANGE_D) } else { (GREEN, BLUE, RED, ORANGE) };
        let plain = if dark { TEXT_DARK } else { TEXT_LIGHT };
        match self {
            Role::Copy => (icons::COPY, green),
            Role::Upload => (icons::UPLOAD, green),
            Role::Download => (icons::DOWNLOAD, green),
            Role::Move => (icons::MOVE, blue),
            Role::Delete => (icons::TRASH, red),
            Role::Cancel => (icons::X, red),
            Role::Close => (icons::X, plain),
            Role::Ok | Role::Save | Role::Send => (icons::CHECK, green),
            Role::Connect => (icons::PLUG, blue),
            Role::Overwrite => (icons::WARNING, orange),
            Role::Open => (icons::OPEN_EXTERNAL, blue),
            Role::Edit => (icons::PENCIL, blue),
            Role::Search => (icons::SEARCH, blue),
            Role::Folder => (icons::FOLDER_OPEN, plain),
            Role::NewFolder => (icons::FOLDER_PLUS, green),
            Role::Unlock => (icons::LOCK_OPEN, green),
            Role::Lock => (icons::LOCK, plain),
            Role::Trust => (icons::SHIELD_CHECK, green),
            Role::Info => (icons::INFO, blue),
            Role::Pack => (icons::PACKAGE, green),
            Role::Unpack => (icons::UNPACK, green),
        }
    }
}

/// A dialog button: `btn(Role::Copy, tr!("Copy (Enter)")).default(true).ui(ui)`.
pub struct DlgButton {
    role: Role,
    text: RichText,
    default: bool,
    enabled: bool,
}

pub fn btn(role: Role, text: impl Into<RichText>) -> DlgButton {
    DlgButton { role, text: text.into(), default: false, enabled: true }
}

/// Minimum height of dialog buttons.
pub const BUTTON_HEIGHT: f32 = 32.0;

impl DlgButton {
    /// The button Enter presses: thicker border.
    pub fn default(mut self, on: bool) -> Self {
        self.default = on;
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn ui(self, ui: &mut Ui) -> Response {
        let dark = ui.visuals().dark_mode;
        let (glyph, color) = self.role.icon(dark);
        let text_color = if dark { TEXT_DARK } else { TEXT_LIGHT };
        let mut text = self.text.size(crate::fonts::fs(14.5));
        if self.role == Role::Overwrite {
            text = text.color(color);
        }
        let stroke = if self.default {
            Stroke::new(2.5, text_color)
        } else {
            Stroke::new(1.0, if dark { WEAK_DARK } else { Color32::from_gray(150) })
        };
        let b = egui::Button::new((icons::icon(glyph, color, 18.0), text))
            .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
            .stroke(stroke);
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(14.0, 6.0);
            ui.add_enabled(self.enabled, b)
        })
        .inner
    }
}

/// WCAG contrast ratio of two colors.
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let ch = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_hints_have_enough_contrast() {
        for dark in [false, true] {
            let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
            tune(&mut v, dark);
            let bg = v.window_fill();
            assert!(contrast(v.text_color(), bg) >= 15.0, "text, dark={dark}");
            assert!(contrast(v.weak_text_color(), bg) >= 7.0, "hints, dark={dark}: {}", contrast(v.weak_text_color(), bg));
            // Also on the panel background.
            assert!(contrast(v.weak_text_color(), v.panel_fill) >= 7.0, "hints on panel, dark={dark}");
            assert!(v.window_stroke.width >= 1.5 && v.window_stroke.width <= 2.0);
        }
    }

    /// The size of a button with its own outline, with the pointer away and over it.
    fn button_sizes(tuned: bool, stroke: Stroke) -> (egui::Rect, egui::Rect) {
        let ctx = egui::Context::default();
        if tuned {
            apply(&ctx);
        }
        let mut rect = egui::Rect::NOTHING;
        let run = |pos: Option<egui::Pos2>| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 200.0))),
                ..Default::default()
            };
            if let Some(p) = pos {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let mut r = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(input, |ui| {
                r = ui.add(egui::Button::new("Home folder").stroke(stroke)).rect;
            });
            out.textures_delta.clear();
            r
        };
        for _ in 0..2 {
            rect = run(None);
        }
        let mut over = rect;
        for _ in 0..3 {
            over = run(Some(rect.center()));
        }
        (rect, over)
    }

    #[test]
    fn hover_changes_only_colors() {
        for dark in [false, true] {
            let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
            tune(&mut v, dark);
            let w = &v.widgets;
            for s in [&w.noninteractive, &w.inactive, &w.hovered, &w.active, &w.open] {
                assert_eq!(s.expansion, 0.0, "dark={dark}");
                assert_eq!(s.bg_stroke.width, w.inactive.bg_stroke.width, "dark={dark}");
            }
        }
        // Buttons with their own outline (selected rows, tabs, the Enter button).
        for stroke in [Stroke::new(2.0, Color32::BLACK), Stroke::new(1.0, Color32::GRAY), Stroke::NONE] {
            let (away, over) = button_sizes(true, stroke);
            assert!(away.is_positive());
            assert_eq!(away, over, "{stroke:?}");
        }
        // The check itself sees the fault of egui's own look.
        let (away, over) = button_sizes(false, Stroke::new(2.0, Color32::BLACK));
        assert_ne!(away, over, "the test no longer notices a hover change");
    }

    #[test]
    fn action_colors_are_readable() {
        // Icon colors on the light button background (≥ 3:1 for graphics).
        for c in [GREEN, BLUE, RED, ORANGE] {
            assert!(contrast(c, Color32::from_gray(230)) >= 3.0, "{c:?}");
        }
        assert_eq!(Role::Delete.icon(false), (icons::TRASH, RED));
        assert_eq!(Role::Overwrite.icon(false).0, icons::WARNING);
        assert_eq!(Role::Cancel.icon(false), (icons::X, RED));
    }
}
