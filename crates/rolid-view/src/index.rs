//! The line index: built in the background from the start of the file. It
//! keeps, for every [`MARK`] bytes, how many line ends come before, so the
//! number of any line in the indexed part is exact after counting at most
//! [`MARK`] bytes, and the start of any line is found the same way. Past the
//! indexed part the numbers are estimated from the average line length.

use crate::enc::Enc;

/// Distance of the marks (bytes).
pub const MARK: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineNo {
    Exact(u64),
    /// Estimated (the index has not got that far yet).
    About(u64),
}

impl LineNo {
    pub fn get(self) -> u64 {
        match self {
            LineNo::Exact(n) | LineNo::About(n) => n,
        }
    }
    pub fn exact(self) -> bool {
        matches!(self, LineNo::Exact(_))
    }
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    enc: Enc,
    /// Where the text starts (after a BOM).
    start: u64,
    /// `marks[k]` = line ends in `[0, k × MARK)`.
    marks: Vec<u64>,
    /// Bytes indexed (from 0).
    scanned: u64,
    /// Line ends in `[0, scanned)`.
    lines: u64,
    /// The whole file is indexed.
    pub done: bool,
    /// Length of the line being scanned, and the longest one so far.
    cur: u64,
    pub longest: u64,
    /// Average line length in the first block (for estimates before
    /// counting got that far) and that block's length.
    hint: Option<(f64, u64)>,
}

impl LineIndex {
    pub fn new(enc: Enc, start: u64) -> Self {
        LineIndex { enc, start, marks: vec![0], scanned: 0, lines: 0, done: false, cur: 0, longest: 0, hint: None }
    }

    /// Lines of `sample` (the start of the file) give the average line
    /// length until the index has counted more than it.
    pub fn set_hint(&mut self, sample: &[u8]) {
        let mut n = 0u64;
        let mut at = 0;
        while let Some(p) = self.enc.find_newline(sample, at) {
            n += 1;
            at = p + self.enc.unit();
        }
        // No line end at all: the lines are at least this long.
        let avg = sample.len().max(1) as f64 / n.max(1) as f64;
        self.hint = Some((avg, sample.len() as u64));
    }

    /// Take over the hint of another index of the same file.
    pub fn keep_hint(mut self, other: Option<&LineIndex>) -> Self {
        self.hint = other.and_then(|o| o.hint);
        self
    }

    pub fn enc(&self) -> Enc {
        self.enc
    }

    pub fn scanned(&self) -> u64 {
        self.scanned
    }

    /// Line ends found so far.
    pub fn line_ends(&self) -> u64 {
        self.lines
    }

    /// Index the next bytes (they continue exactly at [`Self::scanned`];
    /// for UTF-16 the chunks must have an even length).
    pub fn feed(&mut self, data: &[u8]) {
        let base = self.scanned;
        let mut at = 0usize;
        // The BOM is not text.
        if base < self.start {
            at = ((self.start - base) as usize).min(data.len());
        }
        let u = self.enc.unit();
        let mut last = at;
        while let Some(p) = self.enc.find_newline(data, at) {
            let off = base + p as u64;
            self.mark_upto(off);
            self.lines += 1;
            let len = self.cur + (p - last) as u64;
            self.longest = self.longest.max(len);
            self.cur = 0;
            at = p + u;
            last = at;
        }
        self.cur += (data.len() - last) as u64;
        self.longest = self.longest.max(self.cur);
        self.scanned = base + data.len() as u64;
        self.mark_upto(self.scanned.saturating_sub(1));
    }

    /// Record marks up to (and including the one at or before) `off`, with
    /// the line ends counted so far (all before `off`).
    fn mark_upto(&mut self, off: u64) {
        while (self.marks.len() as u64) * MARK <= off {
            self.marks.push(self.lines);
        }
    }

    /// The file is indexed to its end.
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// Average line length so far (for estimates).
    pub fn average_line(&self) -> f64 {
        if let Some((avg, _)) = self.hint.filter(|h| self.scanned < h.1) {
            return avg;
        }
        if self.lines == 0 {
            (self.scanned.max(1)) as f64
        } else {
            self.scanned as f64 / self.lines as f64
        }
    }

    /// The mark before `off`: where counting must start to number the line at `off`.
    pub fn mark_before(off: u64) -> u64 {
        off / MARK * MARK
    }

    /// The number (from 1) of the line containing `off`; `from_mark` are the
    /// bytes from [`Self::mark_before`] to `off` (`None` = not available, the
    /// number is then estimated).
    pub fn line_at(&self, off: u64, from_mark: Option<&[u8]>) -> LineNo {
        if off <= self.scanned || (self.done && off <= self.scanned + 1) {
            let k = (off / MARK) as usize;
            if let (Some(&before), Some(bytes)) = (self.marks.get(k), from_mark) {
                let mut n = 0;
                let mut at = 0;
                let skip = (self.start.saturating_sub(k as u64 * MARK)) as usize;
                at = at.max(skip);
                while let Some(p) = self.enc.find_newline(bytes, at) {
                    n += 1;
                    at = p + self.enc.unit();
                }
                return LineNo::Exact(before + n + 1);
            }
            let before = self.marks.get(k).copied().unwrap_or(self.lines);
            let est = before + ((off - k as u64 * MARK) as f64 / self.average_line()) as u64;
            return LineNo::About(est + 1);
        }
        let est = self.lines + ((off - self.scanned) as f64 / self.average_line()) as u64;
        LineNo::About(est + 1)
    }

    /// Where to look for the start of line `n` (from 1): the mark to start
    /// counting at and how many line ends to skip after it. `None` = the
    /// index has not got there yet.
    pub fn seek_line(&self, n: u64) -> Option<(u64, u64)> {
        if n <= 1 {
            return Some((0, 0));
        }
        let want = n - 1; // line ends before the line
        if want > self.lines {
            return None;
        }
        // The last mark with fewer line ends before it than wanted.
        let k = self.marks.partition_point(|&m| m < want).saturating_sub(1);
        Some((k as u64 * MARK, want - self.marks[k]))
    }

    /// An estimated offset of line `n` (past the indexed part).
    pub fn estimate_offset(&self, n: u64) -> u64 {
        let rest = n.saturating_sub(self.lines + 1);
        self.scanned + (rest as f64 * self.average_line()) as u64
    }

    /// Lines in the whole file (estimated until done).
    pub fn total(&self, size: u64) -> LineNo {
        if self.done || self.scanned >= size {
            // A last line without a line end counts too.
            LineNo::Exact(self.lines + u64::from(self.cur > 0))
        } else {
            LineNo::About(((size as f64) / self.average_line()).max(self.lines as f64) as u64)
        }
    }
}

/// The start of the line after skipping `skip` line ends in `bytes`
/// (counted from a mark, see [`LineIndex::seek_line`]).
pub fn skip_lines(enc: Enc, bytes: &[u8], skip: u64) -> Option<usize> {
    if skip == 0 {
        return Some(0);
    }
    let mut at = 0;
    let mut n = 0;
    while let Some(p) = enc.find_newline(bytes, at) {
        n += 1;
        at = p + enc.unit();
        if n == skip {
            return Some(at);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..lines {
            v.extend(format!("line {i} {}\n", "x".repeat(i % 50)).as_bytes());
        }
        v
    }

    #[test]
    fn line_numbers_are_exact_in_the_indexed_part() {
        let d = text(30_000);
        let mut ix = LineIndex::new(Enc::Utf8, 0);
        // Odd chunk sizes, crossing marks and lines.
        for c in d.chunks(7777) {
            ix.feed(c);
        }
        ix.finish();
        assert_eq!(ix.total(d.len() as u64), LineNo::Exact(30_000));
        let starts: Vec<usize> = std::iter::once(0).chain(d.iter().enumerate().filter(|(_, &b)| b == b'\n').map(|(i, _)| i + 1)).collect();
        for n in [1u64, 2, 3, 1000, 12_345, 29_999, 30_000] {
            let off = starts[n as usize - 1] as u64;
            let m = LineIndex::mark_before(off);
            assert_eq!(ix.line_at(off, Some(&d[m as usize..off as usize])), LineNo::Exact(n), "{n}");
            assert_eq!(ix.line_at(off + 2, Some(&d[m as usize..off as usize + 2])), LineNo::Exact(n));
            let (mark, skip) = ix.seek_line(n).unwrap();
            let at = skip_lines(Enc::Utf8, &d[mark as usize..], skip).unwrap();
            assert_eq!(mark + at as u64, off, "{n}");
        }
    }

    #[test]
    fn estimates_before_counting_use_the_first_block() {
        let d = text(10_000);
        let mut ix = LineIndex::new(Enc::Utf8, 0);
        // Nothing counted yet: a line number in the middle comes from the first block's lines.
        ix.set_hint(&d[..4096]);
        let mid = d.len() as u64 / 2;
        let n = ix.line_at(mid, None).get();
        assert!((4000..6000).contains(&n), "{n}");
        assert!(matches!(ix.total(d.len() as u64), LineNo::About(t) if (8000..12_000).contains(&t)));
        // Once counting passed the first block, the counted lines are used.
        ix.feed(&d[..8192]);
        assert!(matches!(ix.line_at(mid, None), LineNo::About(n) if (4000..6000).contains(&n)));
        // A block without any line end: lines at least that long.
        let mut ix = LineIndex::new(Enc::Utf8, 0);
        ix.set_hint(&[b'x'; 1000]);
        assert_eq!(ix.line_at(100_000, None), LineNo::About(101));
    }

    #[test]
    fn numbers_past_the_index_are_estimates() {
        let d = text(10_000);
        let mut ix = LineIndex::new(Enc::Utf8, 0);
        ix.feed(&d[..d.len() / 2]);
        let half = ix.line_ends();
        let end = d.len() as u64 - 1;
        let n = ix.line_at(end, None);
        assert!(!n.exact());
        // Roughly right (same line lengths throughout).
        assert!((n.get() as i64 - 10_000).abs() < 1500, "{n:?}");
        assert!(!ix.total(d.len() as u64).exact());
        assert_eq!(ix.seek_line(half + 5), None);
        assert!(ix.estimate_offset(9000) > ix.scanned());
    }

    #[test]
    fn longest_line_is_known_and_utf16_counts_units() {
        let mut d = b"a\n".to_vec();
        d.extend(vec![b'y'; 300_000]);
        d.extend(b"\nend");
        let mut ix = LineIndex::new(Enc::Utf8, 0);
        for c in d.chunks(65_536) {
            ix.feed(c);
        }
        ix.finish();
        assert_eq!(ix.longest, 300_000);
        assert_eq!(ix.total(d.len() as u64), LineNo::Exact(3));

        let mut u = vec![0xFF, 0xFE];
        u.extend("a\u{0A00}\nb\nc".encode_utf16().flat_map(|x| x.to_le_bytes()));
        let mut ix = LineIndex::new(Enc::Utf16Le, 2);
        ix.feed(&u);
        ix.finish();
        assert_eq!(ix.total(u.len() as u64), LineNo::Exact(3));
    }
}
