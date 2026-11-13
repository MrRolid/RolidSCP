//! Rows: the lines of a text file as the viewer shows them.
//!
//! A row ends after a line end, or at a segment boundary inside an extremely
//! long line: a multiple of [`SEG`] is a boundary when the line has already
//! been [`SEG`] bytes long there (no line end in the [`SEG`] bytes before
//! it). The rule only looks at nearby bytes, so the row containing any
//! position is found by looking at most `3 × SEG` bytes back: a jump into
//! the middle of a 100 MB line costs nothing, and no row is longer than
//! about `2 × SEG` bytes.

use crate::enc::Enc;

/// Segment length of very long lines (bytes).
pub const SEG: u64 = 2048;
/// How far back a row start may be from a position.
pub const BACK: u64 = 3 * SEG;

/// Bytes of a file from `base` (a window into the cached blocks).
#[derive(Clone, Copy)]
pub struct Window<'a> {
    pub base: u64,
    pub data: &'a [u8],
    /// The size of the whole file (the window may end before).
    pub size: u64,
    /// Where the text starts (after a BOM).
    pub start: u64,
    pub enc: Enc,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Next {
    Row(u64),
    /// The row is the last one.
    End,
    /// The window does not reach far enough.
    NeedData,
}

impl<'a> Window<'a> {
    pub fn end(&self) -> u64 {
        self.base + self.data.len() as u64
    }

    fn at(&self, off: u64) -> usize {
        (off - self.base) as usize
    }

    /// Does a line start at `off` (after a line end, or at the text start)?
    /// `None` = not in the window.
    pub fn is_line_start(&self, off: u64) -> Option<bool> {
        if off <= self.start {
            return Some(true);
        }
        let u = self.enc.unit() as u64;
        if off < self.base + u || off > self.end() {
            return None;
        }
        Some(self.enc.newline_at(self.data, self.at(off - u)))
    }

    /// The start of the row containing `off` (`None` = the window does not
    /// reach [`BACK`] bytes before it).
    pub fn row_start(&self, off: u64) -> Option<u64> {
        let off = off.clamp(self.start, self.size.max(self.start));
        if off == self.start {
            return Some(off);
        }
        let from = align(self, off.saturating_sub(BACK).max(self.start));
        if from < self.base || off > self.end() {
            return None;
        }
        let nl = self.enc.rfind_newline(&self.data[self.at(from)..], (off - from) as usize);
        let u = self.enc.unit() as u64;
        // None = the line starts somewhere before the window (far enough for the segment rule).
        let line_start = match nl {
            Some(i) => Some(from + i as u64 + u),
            None if from == self.start => Some(self.start),
            None => None,
        };
        let mut b = off / SEG * SEG;
        for _ in 0..2 {
            if b <= self.start || line_start.is_some_and(|ls| b < ls + SEG) {
                break;
            }
            let s = self.boundary(b);
            if s <= off {
                return Some(s);
            }
            b -= SEG;
        }
        line_start
    }

    /// A segment boundary moved to the start of a character.
    fn boundary(&self, b: u64) -> u64 {
        if b < self.base || b >= self.end() {
            return b;
        }
        let i = self.at(b);
        self.base + self.enc.char_start(self.data, i) as u64
    }

    /// The start of the row after the row starting at `r`.
    pub fn next_row(&self, r: u64) -> Next {
        if r >= self.size {
            return Next::End;
        }
        if r < self.base {
            return Next::NeedData;
        }
        let Some(line_start) = self.is_line_start(r) else { return Next::NeedData };
        let seg_end = if line_start { ((r + SEG).div_ceil(SEG)) * SEG } else { (r / SEG + 1) * SEG };
        let look_to = seg_end.min(self.size).min(self.end());
        let nl = self.enc.find_newline(&self.data[self.at(r)..self.at(look_to)], 0);
        if let Some(i) = nl {
            let next = r + i as u64 + self.enc.unit() as u64;
            return if next >= self.size { Next::End } else { Next::Row(next) };
        }
        if seg_end >= self.size {
            return if self.end() >= self.size { Next::End } else { Next::NeedData };
        }
        if seg_end + 4 > self.end() {
            return Next::NeedData;
        }
        Next::Row(self.boundary(seg_end))
    }

    /// The start of the row before the row starting at `r` (`None` = no
    /// data or `r` is the first row).
    pub fn prev_row(&self, r: u64) -> Option<u64> {
        if r <= self.start {
            return None;
        }
        self.row_start(r - 1)
    }

    /// The bytes of the row starting at `r` without its line end (and a `\r`
    /// before it), and the start of the next row.
    pub fn row(&self, r: u64) -> Option<(&'a [u8], Next)> {
        let next = self.next_row(r);
        let end = match next {
            Next::Row(n) => n,
            Next::End => self.size.min(self.end()),
            Next::NeedData => return None,
        };
        let mut bytes = &self.data[self.at(r)..self.at(end)];
        let u = self.enc.unit();
        if bytes.len() >= u && self.enc.newline_at(bytes, bytes.len() - u) {
            bytes = &bytes[..bytes.len() - u];
            let cr: &[u8] = match self.enc {
                Enc::Utf16Le => b"\r\0",
                Enc::Utf16Be => b"\0\r",
                _ => b"\r",
            };
            if bytes.ends_with(cr) {
                bytes = &bytes[..bytes.len() - u];
            }
        }
        Some((bytes, next))
    }
}

fn align(w: &Window, off: u64) -> u64 {
    let u = w.enc.unit() as u64;
    let rel = (off - w.start) % u;
    if rel == 0 {
        off
    } else {
        off + (u - rel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(data: &[u8], enc: Enc) -> Window<'_> {
        Window { base: 0, data, size: data.len() as u64, start: 0, enc }
    }

    /// All rows from the start, walking forward.
    fn rows_forward(w: &Window) -> Vec<u64> {
        let mut v = vec![w.start];
        let mut r = w.start;
        while let Next::Row(n) = w.next_row(r) {
            assert!(n > r);
            v.push(n);
            r = n;
        }
        v
    }

    #[test]
    fn short_lines_are_rows() {
        let d = b"one\r\ntwo\n\nthree";
        let w = win(d, Enc::Utf8);
        assert_eq!(rows_forward(&w), vec![0, 5, 9, 10]);
        assert_eq!(w.row(0).unwrap().0, b"one");
        assert_eq!(w.row(9).unwrap().0, b"");
        assert_eq!(w.row(10).unwrap(), (&b"three"[..], Next::End));
        for (off, start) in [(0, 0), (4, 0), (5, 5), (8, 5), (9, 9), (12, 10), (15, 10)] {
            assert_eq!(w.row_start(off), Some(start), "{off}");
        }
        assert_eq!(w.prev_row(10), Some(9));
        assert_eq!(w.prev_row(0), None);
    }

    #[test]
    fn an_extremely_long_line_is_cut_and_found_from_anywhere() {
        let mut d = b"head\n".to_vec();
        d.extend(std::iter::repeat_n(b'x', 20 * SEG as usize + 77));
        d.extend(b"\ntail\n");
        let w = win(&d, Enc::Utf8);
        let rows = rows_forward(&w);
        // Every row is short.
        for p in rows.windows(2) {
            assert!(p[1] - p[0] <= 2 * SEG, "{p:?}");
        }
        // Walking back from every row gives the same rows, and row_start agrees.
        for i in (1..rows.len()).rev() {
            assert_eq!(w.prev_row(rows[i]), Some(rows[i - 1]), "{i}");
        }
        for off in (0..d.len() as u64).step_by(97) {
            let s = w.row_start(off).unwrap();
            let i = rows.partition_point(|&r| r <= off) - 1;
            assert_eq!(s, rows[i], "{off}");
        }
        // A window starting in the middle of the long line still finds rows.
        let base = 10 * SEG + 5;
        let part = Window { base, data: &d[base as usize..], size: d.len() as u64, start: 0, enc: Enc::Utf8 };
        let off = base + 3 * SEG + 11;
        let i = rows.partition_point(|&r| r <= off) - 1;
        assert_eq!(part.row_start(off), Some(rows[i]));
        assert_eq!(part.next_row(rows[i]), Next::Row(rows[i + 1]));
    }

    #[test]
    fn segment_boundaries_do_not_split_characters() {
        let mut d = Vec::new();
        while d.len() < 5 * SEG as usize {
            d.extend("žluťoučký ".as_bytes());
        }
        let w = win(&d, Enc::Utf8);
        for r in rows_forward(&w) {
            assert!(std::str::from_utf8(&d[r as usize..]).is_ok() || (d[r as usize] & 0xC0) != 0x80, "{r}");
        }
    }

    #[test]
    fn utf16_rows() {
        let s = "first\r\nsecond\nthird";
        let mut d = vec![0xFF, 0xFE];
        d.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let w = Window { base: 0, data: &d, size: d.len() as u64, start: 2, enc: Enc::Utf16Le };
        let rows = rows_forward(&w);
        assert_eq!(rows.len(), 3);
        let (b, _) = w.row(2).unwrap();
        assert_eq!(Enc::Utf16Le.decode(b), "first");
        assert_eq!(w.row_start(rows[2] + 4), Some(rows[2]));
        assert_eq!(w.prev_row(rows[2]), Some(rows[1]));
    }

    #[test]
    fn missing_data_is_reported() {
        let d = vec![b'a'; 100];
        let w = Window { base: 0, data: &d, size: 1000, start: 0, enc: Enc::Utf8 };
        assert_eq!(w.next_row(0), Next::NeedData);
        assert_eq!(w.row_start(500), None);
    }
}
