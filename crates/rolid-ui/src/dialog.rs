//! One look for every dialog (modeled on the classic file-transfer dialogs):
//! a colored title bar with the name centered, the action icon on the left
//! and a close ✕ on the right (drag the bar to move the dialog), even
//! margins, framed groups with a heading in the frame, labels in a column,
//! long explanations in a tooltip on a small ⓘ, and a row of buttons of the
//! same width at the bottom right: the default first and highlighted,
//! Cancel last.
//!
//! Keys are the same everywhere: Enter presses the default button (unless
//! the focus is on another button, which Enter then presses), Esc and the ✕
//! press the Cancel/Close button, Tab walks the fields top to bottom and the
//! buttons left to right.
//!
//! ```ignore
//! dialog::window("copy", tr!("Copy")).icon(Role::Copy).show(ctx, |ui| {
//!     dialog::lead(ui, &text);
//!     dialog::group(ui, &tr!("Options"), |ui| { … });
//!     let b = dialog::buttons(ui, [dialog::btn(Role::Copy, tr!("Copy")).default(true), dialog::cancel()]);
//!     if b[0] { … } else if b[1] { … }
//! });
//! ```

use std::cell::Cell;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Id, Key, Layout, Margin, Rect, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
    WidgetText,
};
use rolid_i18n::tr;

use crate::icons;
use crate::look::{self, Role};

/// Height of the title bar.
pub const BAR_HEIGHT: f32 = 30.0;
/// Margin around the content.
pub const MARGIN: f32 = 14.0;
/// Space between rows.
pub const SPACING: f32 = 8.0;
/// Narrowest dialog (content width).
pub const MIN_WIDTH: f32 = 340.0;
/// Widest content before text wraps.
pub const MAX_WIDTH: f32 = 640.0;
/// Narrowest button.
pub const BUTTON_MIN_WIDTH: f32 = 96.0;
/// Size of the icons in buttons.
pub const BUTTON_ICON: f32 = 15.0;

/// Title bar colors: (fill, text).
pub fn bar_colors(dark: bool, danger: bool) -> (Color32, Color32) {
    match (dark, danger) {
        (false, false) => (Color32::from_rgb(0xd6, 0xe2, 0xf3), look::TEXT_LIGHT),
        (true, false) => (Color32::from_rgb(0x2a, 0x3c, 0x58), look::TEXT_DARK),
        (false, true) => (Color32::from_rgb(0xf6, 0xd0, 0xd0), look::TEXT_LIGHT),
        (true, true) => (Color32::from_rgb(0x5a, 0x22, 0x22), look::TEXT_DARK),
    }
}

thread_local! {
    /// Content width of the dialog being drawn (from the last frame).
    static WIDTH: Cell<f32> = const { Cell::new(MIN_WIDTH) };
    /// The ✕ of the dialog being drawn was clicked this frame.
    static CLOSE: Cell<bool> = const { Cell::new(false) };
    /// Another dialog is open on top: Enter and Esc are not for this one.
    static BLOCKED: Cell<bool> = const { Cell::new(false) };
    /// A drop-down or menu was open when the dialog started drawing (it
    /// takes Enter and Esc; it may close itself before the buttons ask).
    static POPUP: Cell<bool> = const { Cell::new(false) };
}

/// While `on`, [`enter`] and [`cancel_pressed`] report nothing (a dialog on
/// top of the current one takes the keys).
pub fn block_keys(on: bool) {
    BLOCKED.set(on);
}

/// A dialog window; see the module docs.
pub struct Window {
    id: Id,
    title: String,
    icon: Option<Role>,
    min_width: f32,
    max_width: f32,
    danger: bool,
}

pub fn window(id: impl std::hash::Hash + std::fmt::Debug, title: impl Into<String>) -> Window {
    Window {
        id: Id::unique(("dialog", id)),
        title: title.into(),
        icon: None,
        min_width: MIN_WIDTH,
        max_width: MAX_WIDTH,
        danger: false,
    }
}

impl Window {
    /// The action icon on the left of the title bar.
    pub fn icon(mut self, role: Role) -> Self {
        self.icon = Some(role);
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }

    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = w;
        self
    }

    /// A red title bar (production server, deleting).
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }

    /// Draw the dialog.
    pub fn show<R>(self, ctx: &egui::Context, body: impl FnOnce(&mut Ui) -> R) -> R {
        let Window { id, title, icon, min_width, max_width, danger } = self;
        let (off_id, w_id) = (id.with("offset"), id.with("width"));
        let offset: Vec2 = ctx.data(|d| d.get_temp(off_id)).unwrap_or_default();
        let last_w: f32 = ctx.data(|d| d.get_temp(w_id)).unwrap_or(min_width + 2.0 * MARGIN);
        let area = egui::Modal::default_area(id).anchor(egui::Align2::CENTER_CENTER, offset);
        let style = ctx.global_style();
        let frame = egui::Frame::popup(&style).inner_margin(Margin::ZERO);
        let radius = frame.corner_radius;
        let popup_open = egui::Popup::is_any_open(ctx);
        let modal = egui::Modal::new(id).area(area).frame(frame).show(ctx, |ui| {
            let dark = ui.visuals().dark_mode;
            let (fill, text) = bar_colors(dark, danger);
            ui.spacing_mut().item_spacing = Vec2::new(SPACING, 0.0);
            // The bar, as wide as the content was in the last frame.
            let (bar, drag) = ui.allocate_exact_size(Vec2::new(last_w, BAR_HEIGHT), Sense::click_and_drag());
            if drag.dragged() {
                let d = drag.drag_delta();
                ctx.data_mut(|m| m.insert_temp(off_id, offset + d));
            }
            let painter = ui.painter().clone();
            painter.rect_filled(bar, CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 }, fill);
            painter.hline(bar.x_range(), bar.bottom(), Stroke::new(1.0, ui.visuals().window_stroke.color));
            let mut left = bar.left() + 10.0;
            if let Some(r) = icon {
                let (glyph, color) = r.icon(dark);
                painter.text(
                    egui::pos2(left, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    glyph,
                    icons::font(16.0),
                    color,
                );
                left += 22.0;
            }
            let font = egui::FontId::proportional(crate::fonts::fs(14.5));
            let galley = painter.layout_no_wrap(title.clone(), font, text);
            let tw = galley.size().x;
            // Centered unless it would run into the icon or the ✕.
            let x = (bar.center().x - tw / 2.0).max(left);
            painter.galley(egui::pos2(x, bar.center().y - galley.size().y / 2.0), galley, text);
            let close = Rect::from_center_size(egui::pos2(bar.right() - 18.0, bar.center().y), Vec2::splat(24.0));
            let cr = ui.interact(close, id.with("close"), Sense::click()).on_hover_text(tr!("Close (Esc)"));
            if cr.hovered() {
                painter.rect_filled(close, 4.0, if dark { Color32::from_gray(80) } else { Color32::from_gray(200) });
            }
            painter.text(close.center(), egui::Align2::CENTER_CENTER, icons::X, icons::font(15.0), text);
            let need = tw + (left - bar.left()) + 44.0;

            let mut inner = None;
            let content_w = (last_w - 2.0 * MARGIN).max(min_width);
            let r = egui::Frame::NONE.inner_margin(Margin::same(MARGIN as i8)).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(SPACING, SPACING);
                ui.set_min_width(min_width.max(need - 2.0 * MARGIN));
                ui.set_max_width(max_width);
                let (w0, c0) = (WIDTH.replace(content_w), CLOSE.replace(cr.clicked()));
                let p0 = POPUP.replace(popup_open);
                // A dialog taller than the window scrolls instead of leaving the screen.
                let cap = (ctx.content_rect().height() - BAR_HEIGHT - 2.0 * MARGIN - 16.0).max(120.0);
                egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(cap).min_scrolled_height(cap).auto_shrink([true, true]).show(ui, |ui| {
                    ui.set_min_width(min_width.max(need - 2.0 * MARGIN));
                    ui.set_max_width(max_width);
                    inner = Some(body(ui));
                });
                WIDTH.set(w0);
                CLOSE.set(c0);
                POPUP.set(p0);
            });
            let w = r.response.rect.width();
            if (w - last_w).abs() > 0.5 {
                ctx.data_mut(|m| m.insert_temp(w_id, w));
                ctx.request_repaint();
            }
            inner
        });
        modal.inner.expect("dialog body")
    }
}

/// Width of the content of the dialog being drawn.
pub fn width() -> f32 {
    WIDTH.get()
}

/// The ✕ of the dialog being drawn was clicked (dialogs without
/// [`buttons`] check this themselves).
pub fn close_clicked() -> bool {
    CLOSE.get()
}

/// Enter pressed for the dialog (not while a menu or a list is open).
pub fn enter(ui: &Ui) -> bool {
    !BLOCKED.get() && !POPUP.get() && !egui::Popup::is_any_open(ui.ctx()) && ui.input(|i| i.key_pressed(Key::Enter))
}

/// Esc pressed or the ✕ clicked.
pub fn cancel_pressed(ui: &Ui) -> bool {
    close_clicked() || (!BLOCKED.get() && !POPUP.get() && !egui::Popup::is_any_open(ui.ctx()) && ui.input(|i| i.key_pressed(Key::Escape)))
}

/// The main sentence of the dialog.
pub fn lead(ui: &mut Ui, text: impl Into<WidgetText>) {
    ui.add(egui::Label::new(text).wrap());
}

/// A quieter line (a hint under a field).
pub fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text.into()).weak()).wrap());
}

/// A warning line (orange, or red for `danger`).
pub fn warn(ui: &mut Ui, text: impl Into<String>, danger: bool) {
    let dark = ui.visuals().dark_mode;
    let color = match (danger, dark) {
        (true, false) => look::RED,
        (true, true) => Color32::from_rgb(0xff, 0x7a, 0x7a),
        (false, false) => look::ORANGE,
        (false, true) => Color32::from_rgb(0xff, 0xa8, 0x4d),
    };
    ui.add(egui::Label::new(RichText::new(text.into()).color(color).strong()).wrap());
}

/// A small ⓘ with the explanation in its tooltip.
pub fn help(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let t = text.into();
    let color = ui.visuals().weak_text_color();
    ui.add(egui::Label::new(icons::icon(icons::INFO, color, 15.0)).sense(Sense::hover()))
        .on_hover_ui(|ui| {
            ui.set_max_width(380.0);
            ui.label(t);
        })
}

/// A framed group with its heading in the frame, as wide as the dialog.
pub fn group<R>(ui: &mut Ui, heading: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    let dark = ui.visuals().dark_mode;
    let stroke = Stroke::new(1.0, if dark { Color32::from_gray(110) } else { Color32::from_gray(160) });
    let w = width().max(ui.min_rect().width());
    ui.add_space(6.0);
    let r = egui::Frame::NONE
        .stroke(stroke)
        .corner_radius(4.0)
        .inner_margin(Margin { left: 12, right: 12, top: 14, bottom: 10 })
        .show(ui, |ui| {
            ui.set_min_width(w - 26.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui)
        });
    if !heading.is_empty() {
        let rect = r.response.rect;
        let font = TextStyle::Body.resolve(ui.style());
        let color = ui.visuals().strong_text_color();
        let galley = ui.painter().layout_no_wrap(heading.to_string(), font, color);
        let pos = egui::pos2(rect.left() + 10.0, rect.top() - galley.size().y / 2.0);
        let bg = Rect::from_min_size(pos - Vec2::new(4.0, 0.0), galley.size() + Vec2::new(8.0, 0.0));
        ui.painter().rect_filled(bg, 0.0, ui.visuals().window_fill);
        ui.painter().galley(pos, galley, color);
    }
    r.inner
}

/// Labels in one column, fields in the next: `form(ui, "id", |ui| { ui.label(…); field; ui.end_row(); })`.
pub fn form<R>(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id).num_columns(2).spacing([12.0, 8.0]).show(ui, body).inner
}

/// A dialog button for [`buttons`].
pub struct Btn {
    role: Role,
    text: String,
    default: bool,
    enabled: bool,
    tooltip: Option<String>,
}

pub fn btn(role: Role, text: impl Into<String>) -> Btn {
    Btn { role, text: text.into(), default: false, enabled: true, tooltip: None }
}

/// Cancel: last in the row, pressed by Esc and the ✕.
pub fn cancel() -> Btn {
    btn(Role::Cancel, tr!("Cancel"))
}

/// Close: like [`cancel`] for dialogs that change nothing.
pub fn close() -> Btn {
    btn(Role::Close, tr!("Close"))
}

impl Btn {
    /// The button Enter presses (highlighted, first in the row).
    pub fn default(mut self, on: bool) -> Self {
        self.default = on;
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn tooltip(mut self, t: impl Into<String>) -> Self {
        self.tooltip = Some(t.into());
        self
    }

    fn escapes(&self) -> bool {
        matches!(self.role, Role::Cancel | Role::Close)
    }

    /// Only the main actions carry their colored icon.
    fn has_icon(&self) -> bool {
        !self.escapes()
    }
}

/// Width of every button in a row.
fn button_width(ui: &Ui, list: &[Btn]) -> f32 {
    let widest = list
        .iter()
        .map(|b| {
            let t = WidgetText::from(RichText::new(&b.text).size(crate::fonts::fs(14.5)));
            let g = t.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
            g.size().x + if b.has_icon() { BUTTON_ICON + 6.0 } else { 0.0 } + 28.0
        })
        .fold(0.0, f32::max);
    widest.max(BUTTON_MIN_WIDTH).ceil()
}

/// The row of buttons at the bottom right; returns which were pressed
/// (by a click or by Enter/Esc/✕, see the module docs).
pub fn buttons<const N: usize>(ui: &mut Ui, list: [Btn; N]) -> [bool; N] {
    let mut pressed = [false; N];
    let w = button_width(ui, &list);
    let gap = SPACING;
    let total = w * N as f32 + gap * (N.saturating_sub(1)) as f32;
    let full = width().max(ui.min_rect().width());
    ui.add_space(4.0);
    let dark = ui.visuals().dark_mode;
    let text_color = if dark { look::TEXT_DARK } else { look::TEXT_LIGHT };
    let mut focused = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_space((full - total).max(0.0) - gap.min((full - total).max(0.0)));
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            for (i, b) in list.iter().enumerate() {
                let text = RichText::new(&b.text).size(crate::fonts::fs(14.5));
                let stroke = if b.default && b.enabled {
                    Stroke::new(2.5, text_color)
                } else {
                    Stroke::new(1.0, if dark { look::WEAK_DARK } else { Color32::from_gray(150) })
                };
                let wb = if b.has_icon() {
                    let (glyph, color) = b.role.icon(dark);
                    let text = if b.role == Role::Overwrite { text.color(color) } else { text };
                    egui::Button::new((icons::icon(glyph, color, BUTTON_ICON), text))
                } else {
                    egui::Button::new(text)
                };
                let wb = wb.min_size(Vec2::new(w, look::BUTTON_HEIGHT)).stroke(stroke);
                let mut r = ui.add_enabled(b.enabled, wb);
                if let Some(t) = &b.tooltip {
                    r = r.on_hover_text(t);
                }
                focused |= r.has_focus();
                pressed[i] = r.clicked();
            }
        });
    });
    // Keys: Enter = the default (unless a button has the focus, then that
    // button got it as a click), Esc / ✕ = Cancel/Close.
    if !focused && enter(ui) {
        if let Some(i) = list.iter().position(|b| b.default && b.enabled) {
            pressed[i] = true;
        }
    }
    if cancel_pressed(ui) {
        if let Some(i) = list.iter().rposition(|b| b.escapes()) {
            pressed[i] = true;
        }
    }
    pressed
}

/// A one-line text field with a drop-down of earlier values; returns the
/// field's response (`changed()` also when a value was picked).
pub fn history_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, text: &mut String, history: &[String], width: f32) -> egui::Response {
    let id = Id::unique(("history", id));
    let mut picked = false;
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(width - 30.0));
            let b = ui
                .add_enabled(!history.is_empty(), egui::Button::new("⏷").min_size(Vec2::new(26.0, r.rect.height())))
                .on_hover_text(tr!("Recent targets"));
            egui::Popup::menu(&b).id(id).width(width).show(|ui| {
                for h in history {
                    if ui.add(egui::Button::new(h.as_str()).truncate().min_size(Vec2::new(width - 16.0, 0.0))).on_hover_text(h.as_str()).clicked() {
                        *text = h.clone();
                        picked = true;
                    }
                }
            });
            r
        })
        .inner;
    let mut r = r;
    if picked {
        r.mark_changed();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_equally_wide_and_keys_follow_the_roles() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let l = [btn(Role::Copy, "Copy everything"), cancel()];
            let w = button_width(ui, &l);
            assert!(w >= BUTTON_MIN_WIDTH);
            assert!(l[1].escapes() && !l[1].has_icon() && l[0].has_icon());
        });
        out.textures_delta.clear();
    }

    #[test]
    fn danger_bar_is_red_and_readable() {
        let (fill, text) = bar_colors(false, true);
        assert!(fill.r() > fill.g() && text == look::TEXT_LIGHT);
    }
}
