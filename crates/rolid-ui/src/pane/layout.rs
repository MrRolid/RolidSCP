//! Geometry of the brief listing (names in columns, filled column by column
//! as in Total Commander) and of the thumbnail grid (filled row by row).

/// Rows per column of the brief listing for a list `height` and row height.
pub fn brief_rows(height: f32, row_h: f32) -> usize {
    ((height / row_h).floor() as usize).max(1)
}

/// Columns visible at once for a list `width` (each at least `min_col_w`).
pub fn brief_visible_cols(width: f32, min_col_w: f32) -> usize {
    ((width / min_col_w).floor() as usize).max(1)
}

/// Column and row of item `idx`.
pub fn brief_pos(idx: usize, rows: usize) -> (usize, usize) {
    (idx / rows, idx % rows)
}

/// Number of columns needed for `len` items.
pub fn brief_cols(len: usize, rows: usize) -> usize {
    len.div_ceil(rows).max(1)
}

/// ← / → in the brief listing: one column to the side (same row). Past the
/// last item it stops on the last one; past the first on the first.
pub fn brief_move(cursor: usize, len: usize, rows: usize, dcol: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let target = cursor as isize + dcol * rows as isize;
    target.clamp(0, len as isize - 1) as usize
}

/// First visible column so that the cursor's column is on screen.
pub fn brief_scroll(first: usize, cursor: usize, rows: usize, visible: usize) -> usize {
    let col = cursor / rows;
    if col < first {
        col
    } else if col >= first + visible {
        col + 1 - visible
    } else {
        first
    }
}

/// Columns of the thumbnail grid for a list `width` and tile width.
pub fn grid_cols(width: f32, tile_w: f32) -> usize {
    ((width / tile_w).floor() as usize).max(1)
}

/// Arrows in the grid: ← / → by one, ↑ / ↓ by a row.
pub fn grid_move(cursor: usize, len: usize, cols: usize, dx: isize, dy: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let target = cursor as isize + dx + dy * cols as isize;
    if dy != 0 && !(0..len as isize).contains(&target) {
        return cursor; // no row above / below: stay
    }
    target.clamp(0, len as isize - 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_layout_fills_columns() {
        assert_eq!(brief_rows(205.0, 20.0), 10);
        assert_eq!(brief_rows(5.0, 20.0), 1);
        assert_eq!(brief_visible_cols(650.0, 200.0), 3);
        assert_eq!(brief_pos(0, 10), (0, 0));
        assert_eq!(brief_pos(9, 10), (0, 9));
        assert_eq!(brief_pos(10, 10), (1, 0));
        assert_eq!(brief_pos(23, 10), (2, 3));
        assert_eq!(brief_cols(23, 10), 3);
        assert_eq!(brief_cols(0, 10), 1);
    }

    #[test]
    fn brief_arrows_move_between_columns() {
        // 23 items, 10 rows: columns 0..9, 10..19, 20..22.
        assert_eq!(brief_move(3, 23, 10, 1), 13);
        assert_eq!(brief_move(13, 23, 10, 1), 22, "past the end: last item");
        assert_eq!(brief_move(13, 23, 10, -1), 3);
        assert_eq!(brief_move(3, 23, 10, -1), 0, "before the start: first item");
        assert_eq!(brief_move(0, 0, 10, 1), 0);
        // Scrolling keeps the cursor column visible.
        assert_eq!(brief_scroll(0, 22, 10, 2), 1);
        assert_eq!(brief_scroll(2, 3, 10, 2), 0);
        assert_eq!(brief_scroll(1, 15, 10, 2), 1);
    }

    #[test]
    fn grid_arrows() {
        // 10 items in 4 columns.
        assert_eq!(grid_cols(500.0, 120.0), 4);
        assert_eq!(grid_move(5, 10, 4, 1, 0), 6);
        assert_eq!(grid_move(5, 10, 4, 0, 1), 9);
        assert_eq!(grid_move(9, 10, 4, 0, 1), 9, "no row below");
        assert_eq!(grid_move(1, 10, 4, 0, -1), 1, "no row above");
        assert_eq!(grid_move(9, 10, 4, 1, 0), 9);
        assert_eq!(grid_move(0, 10, 4, -1, 0), 0);
    }
}
