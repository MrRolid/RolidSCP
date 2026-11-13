//! Columns of the detailed listing: Name takes what the others leave, the
//! others have a width set by dragging the separator at their left edge
//! (double click: fit the content). On a narrow panel Permissions, then
//! Modified, then Ext are hidden before Name gets narrower than its minimum.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Galley, Painter, Rect};
use rolid_config::ColumnWidths;

/// A column with its own width (all but Name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Col {
    Ext,
    Size,
    Date,
    Mode,
}

/// The narrowest column: its separator can still be grabbed and moved back.
pub const MIN_COL: f32 = 28.0;
/// The narrowest Name column.
pub const MIN_NAME: f32 = 80.0;
/// Room left and right of a column's text.
pub const PAD: f32 = 6.0;

impl Col {
    /// Left to right.
    pub const ALL: [Col; 4] = [Col::Ext, Col::Size, Col::Date, Col::Mode];

    pub fn default_width(self, seconds: bool) -> f32 {
        match self {
            Col::Ext => 64.0,
            Col::Size => 90.0,
            Col::Date if seconds => 154.0,
            Col::Date => 130.0,
            Col::Mode => 90.0,
        }
    }

    pub fn get(self, w: &ColumnWidths) -> Option<f32> {
        match self {
            Col::Ext => w.ext,
            Col::Size => w.size,
            Col::Date => w.date,
            Col::Mode => w.mode,
        }
    }

    pub fn set(self, w: &mut ColumnWidths, v: Option<f32>) {
        let slot = match self {
            Col::Ext => &mut w.ext,
            Col::Size => &mut w.size,
            Col::Date => &mut w.date,
            Col::Mode => &mut w.mode,
        };
        *slot = v.map(|v| v.max(MIN_COL));
    }

    /// Width in points at text scale `k`.
    pub fn width(self, w: &ColumnWidths, seconds: bool, k: f32) -> f32 {
        self.get(w).unwrap_or(self.default_width(seconds)).max(MIN_COL) * k
    }
}

pub struct Columns {
    pub name: Rect,
    pub ext: Rect,
    pub size: Rect,
    pub date: Rect,
    pub mode: Rect,
}

impl Columns {
    pub fn new(r: Rect, ext: bool, seconds: bool, widths: &ColumnWidths) -> Self {
        Self::layout(r, ext, seconds, widths, crate::fonts::scale())
    }

    /// The columns in a row `r` at text scale `k`; whole points, so that the
    /// separators fall on the same pixels in the header and in every row.
    pub fn layout(r: Rect, ext: bool, seconds: bool, widths: &ColumnWidths, k: f32) -> Self {
        let w = |c: Col| c.width(widths, seconds, k).round();
        let mut shown = [ext, true, true, true];
        let used = |shown: &[bool; 4]| Col::ALL.iter().zip(shown).filter(|(_, s)| **s).map(|(c, _)| w(*c)).sum::<f32>();
        for c in [Col::Mode, Col::Date, Col::Ext] {
            if r.width() - used(&shown) >= MIN_NAME * k {
                break;
            }
            shown[c as usize] = false;
        }
        let mut x = r.right().round();
        let mut take = |c: Col| {
            let width = if shown[c as usize] { w(c) } else { 0.0 };
            let rect = Rect::from_min_max(egui::pos2(x - width, r.top()), egui::pos2(x, r.bottom()));
            x -= width;
            rect
        };
        let mode = take(Col::Mode);
        let date = take(Col::Date);
        let size = take(Col::Size);
        let ext = take(Col::Ext);
        let name = Rect::from_min_max(r.min, egui::pos2(x, r.bottom()));
        Self { name, ext, size, date, mode }
    }

    pub fn rect(&self, c: Col) -> Rect {
        match c {
            Col::Ext => self.ext,
            Col::Size => self.size,
            Col::Date => self.date,
            Col::Mode => self.mode,
        }
    }

    /// The shown columns after Name with their left edge: the separators.
    pub fn separators(&self) -> impl Iterator<Item = (Col, f32)> + '_ {
        Col::ALL.into_iter().filter(|c| self.rect(*c).width() > 0.0).map(|c| (c, self.rect(c).left()))
    }

    /// The width (at text scale 1) column `c` gets when its left separator is
    /// dragged to `x`: at least [`MIN_COL`], and Name keeps [`MIN_NAME`].
    pub fn dragged(&self, c: Col, x: f32, k: f32) -> f32 {
        let r = self.rect(c);
        let between = r.left() - self.name.right();
        let min_x = self.name.left() + MIN_NAME * k + between;
        let max_x = r.right() - MIN_COL * k;
        let x = if min_x > max_x { max_x } else { x.clamp(min_x, max_x) };
        ((r.right() - x) / k).round().max(MIN_COL)
    }
}

/// The width (at text scale 1) that fits text `content` points wide.
pub fn fit(content: f32, k: f32) -> f32 {
    ((content + 2.0 * PAD * k) / k).ceil().max(MIN_COL)
}

/// `text` on one line at most `max_w` wide, cut with "…"; also whether it was cut.
pub fn elided(p: &Painter, text: String, font: FontId, color: Color32, max_w: f32) -> (Arc<Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width: max_w.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let g = p.layout_job(job);
    let cut = g.elided;
    (g, cut)
}

/// A separator line on the pixel grid.
pub fn separator(p: &Painter, x: f32, top: f32, bottom: f32, color: Color32) {
    let px = crate::fonts::hairline();
    let x = p.round_to_pixel_center(x - px * 0.5);
    p.vline(x, top..=bottom, egui::Stroke::new(px, color));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(w: f32) -> Rect {
        Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(w, 20.0))
    }

    #[test]
    fn name_takes_the_rest_and_narrow_panels_hide_columns() {
        let d = ColumnWidths::default();
        let c = Columns::layout(row(700.0), true, false, &d, 1.0);
        assert_eq!(c.mode.width(), 90.0);
        assert_eq!(c.date.width(), 130.0);
        assert_eq!(c.size.width(), 90.0);
        assert_eq!(c.ext.width(), 64.0);
        assert_eq!(c.name.width(), 700.0 - 374.0);
        assert_eq!(c.ext.left(), c.name.right(), "no gaps");
        // A wider window: the difference all goes to Name.
        let wide = Columns::layout(row(900.0), true, false, &d, 1.0);
        assert_eq!(wide.name.width() - c.name.width(), 200.0);
        assert_eq!(wide.size.width(), c.size.width());
        // Narrow: Permissions goes first, then Modified, then Ext; Size stays.
        let n = Columns::layout(row(400.0), true, false, &d, 1.0);
        assert_eq!((n.mode.width(), n.date.width() > 0.0, n.ext.width() > 0.0), (0.0, true, true));
        assert!(n.name.width() >= MIN_NAME);
        let n = Columns::layout(row(250.0), true, false, &d, 1.0);
        assert_eq!((n.mode.width(), n.date.width(), n.ext.width()), (0.0, 0.0, 64.0));
        let n = Columns::layout(row(160.0), true, false, &d, 1.0);
        assert_eq!((n.ext.width(), n.size.width()), (0.0, 90.0));
        // Without the Ext column.
        let n = Columns::layout(row(700.0), false, false, &d, 1.0);
        assert_eq!(n.ext.width(), 0.0);
        assert_eq!(n.separators().map(|(c, _)| c).collect::<Vec<_>>(), [Col::Size, Col::Date, Col::Mode]);
    }

    #[test]
    fn saved_widths_restored_and_kept_above_the_minimum() {
        let mut w = ColumnWidths::default();
        Col::Size.set(&mut w, Some(150.0));
        Col::Ext.set(&mut w, Some(3.0));
        assert_eq!(w.size, Some(150.0));
        assert_eq!(w.ext, Some(MIN_COL), "too narrow: the minimum, so it can be grabbed again");
        let c = Columns::layout(row(800.0), true, false, &w, 1.0);
        assert_eq!(c.size.width(), 150.0);
        assert_eq!(c.ext.width(), MIN_COL);
        // Larger text scales the widths.
        let c2 = Columns::layout(row(1200.0), true, false, &w, 1.5);
        assert_eq!(c2.size.width(), 225.0);
        Col::Size.set(&mut w, None);
        assert_eq!(Columns::layout(row(800.0), true, false, &w, 1.0).size.width(), 90.0, "default again");
        // Separators on whole points.
        let c = Columns::layout(Rect::from_min_size(egui::pos2(10.3, 0.0), egui::vec2(700.4, 20.0)), true, true, &w, 1.13);
        for (_, x) in c.separators() {
            assert_eq!(x, x.round());
        }
    }

    #[test]
    fn dragging_a_separator() {
        let d = ColumnWidths::default();
        let c = Columns::layout(row(700.0), true, false, &d, 1.0);
        // The Size separator dragged 30 points left: Size 30 wider.
        assert_eq!(c.dragged(Col::Size, c.size.left() - 30.0, 1.0), 120.0);
        assert_eq!(c.dragged(Col::Size, c.size.left() + 30.0, 1.0), 60.0);
        // Never under the minimum, never squeezing Name under its minimum.
        assert_eq!(c.dragged(Col::Size, c.size.right() + 50.0, 1.0), MIN_COL);
        let widest = c.dragged(Col::Size, -1000.0, 1.0);
        assert_eq!(c.size.right() - widest - c.ext.width(), 10.0 + MIN_NAME);
        // At scale 2 the saved width is in points at scale 1.
        let c = Columns::layout(row(1400.0), true, false, &d, 2.0);
        assert_eq!(c.dragged(Col::Mode, c.mode.left() - 20.0, 2.0), 100.0);
        assert_eq!(fit(100.0, 1.0), 112.0);
        assert_eq!(fit(1.0, 1.0), MIN_COL);
    }
}
