//! Layout of the function-key bar at the bottom ("F5 Copy" …).
//!
//! The version / saved-passwords buttons on the right always get their room
//! first. The function keys show the full text when it fits, otherwise only
//! the key ("F5", full text in the tooltip), and when even that does not fit
//! they wrap onto more rows.

/// How the function-key buttons are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarMode {
    /// "F5 Copy" on one row.
    Full,
    /// "F5" on one row, the text in the tooltip.
    Keys,
    /// "F5" wrapped onto several rows.
    Wrap,
}

/// Width of a row of buttons (each `widths[i]`) with `gap` between them.
fn row_width(widths: &[f32], gap: f32) -> f32 {
    widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32
}

/// Choose the mode for `avail` points of width; `right` is the width the
/// buttons on the right need (reserved first), `full` / `keys` the widths of
/// the function-key buttons with and without the text.
pub fn bar_mode(avail: f32, right: f32, full: &[f32], keys: &[f32], gap: f32) -> BarMode {
    let room = avail - right - gap;
    if row_width(full, gap) <= room {
        BarMode::Full
    } else if row_width(keys, gap) <= room {
        BarMode::Keys
    } else {
        BarMode::Wrap
    }
}

/// Width left for the function keys (never below one button, so wrapping works).
pub fn keys_room(avail: f32, right: f32, gap: f32, widest_key: f32) -> f32 {
    (avail - right - gap).max(widest_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: [f32; 4] = [100.0, 90.0, 120.0, 80.0];
    const KEYS: [f32; 4] = [30.0, 30.0, 60.0, 30.0];

    #[test]
    fn full_text_then_keys_then_rows() {
        // 4 buttons: full 390 + 3 gaps = 402, keys 150 + 12 = 162; right side 150.
        assert_eq!(bar_mode(1000.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Full);
        assert_eq!(bar_mode(556.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Full);
        assert_eq!(bar_mode(555.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Keys);
        assert_eq!(bar_mode(316.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Keys);
        assert_eq!(bar_mode(315.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Wrap);
        assert_eq!(bar_mode(100.0, 150.0, &FULL, &KEYS, 4.0), BarMode::Wrap);
    }

    #[test]
    fn the_right_side_is_always_reserved() {
        // A wider version button pushes the keys into a smaller mode, never off screen.
        assert_eq!(bar_mode(600.0, 100.0, &FULL, &KEYS, 4.0), BarMode::Full);
        assert_eq!(bar_mode(600.0, 300.0, &FULL, &KEYS, 4.0), BarMode::Keys);
        assert_eq!(keys_room(600.0, 300.0, 4.0, 60.0), 296.0);
        // Narrower than the right side: the keys still get one button of room (wrapping).
        assert_eq!(keys_room(200.0, 300.0, 4.0, 60.0), 60.0);
        assert_eq!(row_width(&[], 4.0), 0.0);
    }
}
