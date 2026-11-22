//! The quick filter in a big folder. The names of a listing are copied into
//! one block of memory (made in the background with the sorting): scoring a
//! million names then reads the block from start to end instead of a million
//! separate strings spread over the heap, which on a busy machine took most
//! of the first key's 0.1–0.3 s. The order of the matches comes from counting
//! the scores (they lie in a small range), not from comparing pairs.

use rolid_core::{filter, Entry};

/// Listings at least this long get the block (smaller ones are fast anyway).
pub const MIN_ITEMS: usize = 50_000;

/// The names of a listing, in its order, one after another.
pub struct NameIndex {
    buf: Vec<u8>,
    /// Where each name starts; one more at the end.
    offs: Vec<u32>,
}

impl NameIndex {
    /// The block of `entries`' names; `None` when the listing is small or
    /// the names do not fit 4 GB.
    pub fn new(entries: &[Entry]) -> Option<NameIndex> {
        if entries.len() < MIN_ITEMS {
            return None;
        }
        // One walk over the names (each walk over a million strings spread
        // over the heap costs tens of ms); the block grows if they are longer.
        let mut buf = Vec::with_capacity(entries.len() * 16);
        let mut offs = Vec::with_capacity(entries.len() + 1);
        for e in entries {
            offs.push(u32::try_from(buf.len()).ok()?);
            buf.extend_from_slice(e.name.as_bytes());
        }
        offs.push(u32::try_from(buf.len()).ok()?);
        Some(NameIndex { buf, offs })
    }

    pub fn len(&self) -> usize {
        self.offs.len() - 1
    }

    /// The score of name `i` (as [`filter::score`]); a name that is not
    /// ASCII is scored from the listing.
    pub fn score(&self, i: usize, entries: &[Entry], query: &str) -> Option<i32> {
        let n = &self.buf[self.offs[i] as usize..self.offs[i + 1] as usize];
        if query.is_ascii() && n.is_ascii() {
            filter::score_ascii(n, query.as_bytes())
        } else {
            filter::score(&entries[i].name, query)
        }
    }
}

/// Which items the filter scores: all of a listing of this length, or these
/// (in increasing order).
pub enum Candidates<'a> {
    All(usize),
    Some(&'a [u32]),
}

/// Score the candidates, on several threads for many: (score, item) of the
/// matches in the candidates' order, in parts (one per thread, never copied
/// into one: with a million matches that copy took as long as the scoring).
pub fn score_all(c: &Candidates, f: &(impl Fn(usize) -> Option<i32> + Sync)) -> Vec<Vec<(i32, u32)>> {
    let len = match c {
        Candidates::All(n) => *n,
        Candidates::Some(v) => v.len(),
    };
    let item = |k: usize| match c {
        Candidates::All(_) => k,
        Candidates::Some(v) => v[k] as usize,
    };
    let part = |r: std::ops::Range<usize>| -> Vec<(i32, u32)> {
        let mut out = Vec::with_capacity(r.len());
        for k in r {
            let i = item(k);
            if let Some(s) = f(i) {
                out.push((s, i as u32));
            }
        }
        out
    };
    let threads = super::threads_for(len);
    if threads == 1 {
        return vec![part(0..len)];
    }
    let step = len.div_ceil(threads);
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..len).step_by(step).map(|a| s.spawn(move || part(a..(a + step).min(len)))).collect();
        hs.into_iter().map(|h| h.join().expect("filter thread")).collect()
    })
}

/// The items of `parts` (score, item) ordered by score, best first; equal
/// scores keep their order in the parts.
pub fn rank(parts: Vec<Vec<(i32, u32)>>) -> Vec<u32> {
    let all = || parts.iter().flatten();
    let Some(lo) = all().map(|s| s.0).min() else { return Vec::new() };
    let hi = all().map(|s| s.0).max().unwrap_or(lo);
    let total: usize = parts.iter().map(Vec::len).sum();
    if lo == hi {
        // One score (one letter typed: often every name starts with it).
        let mut out = Vec::with_capacity(total);
        out.extend(all().map(|s| s.1));
        return out;
    }
    let span = (hi as i64 - lo as i64) as usize + 1;
    if span > 1 << 16 {
        let mut s: Vec<(i32, u32)> = parts.into_iter().flatten().collect();
        s.sort_by_key(|x| std::cmp::Reverse(x.0));
        return s.into_iter().map(|(_, i)| i).collect();
    }
    // Counting: where each score starts (the best first), then each item in place.
    let mut start = vec![0usize; span + 1];
    for &(s, _) in all() {
        start[(hi - s) as usize + 1] += 1;
    }
    for k in 1..=span {
        start[k] += start[k - 1];
    }
    let mut out = vec![0u32; total];
    for &(s, i) in all() {
        let k = (hi - s) as usize;
        out[start[k]] = i;
        start[k] += 1;
    }
    out
}

/// `items` (indices below `n`, each at most once) in increasing order: a
/// bitmap walked once, faster than sorting a million numbers.
pub fn in_order(items: impl Iterator<Item = usize>, n: usize) -> Vec<u32> {
    let mut bits = vec![0u64; n.div_ceil(64)];
    let mut count = 0;
    for i in items {
        bits[i / 64] |= 1 << (i % 64);
        count += 1;
    }
    let mut out = Vec::with_capacity(count);
    for (w, &word) in bits.iter().enumerate() {
        let mut b = word;
        while b != 0 {
            out.push((w * 64 + b.trailing_zeros() as usize) as u32);
            b &= b - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolid_core::EntryKind;

    fn ent(name: &str) -> Entry {
        Entry { name: name.into(), kind: EntryKind::File, link_to_dir: false, size: 0, modified: None, mode: None }
    }

    #[test]
    fn block_scores_like_the_names() {
        let mut names: Vec<String> = (0..MIN_ITEMS).map(|i| format!("f{i:07}")).collect();
        names[7] = "Žluťoučký kůň f1.txt".into();
        names[8] = "".into();
        names[9] = "CART car".into();
        let entries: Vec<Entry> = names.iter().map(|n| ent(n)).collect();
        let ix = NameIndex::new(&entries).unwrap();
        assert_eq!(ix.len(), entries.len());
        for q in ["f", "F1", "f12", "car", "kůň", "ž", "zzz", "f0000009"] {
            for (i, e) in entries.iter().enumerate().take(200) {
                assert_eq!(ix.score(i, &entries, q), filter::score(&e.name, q), "{:?} / {q:?}", e.name);
            }
        }
        assert!(NameIndex::new(&entries[..10]).is_none(), "a small folder needs none");
    }

    #[test]
    fn rank_orders_by_score_and_keeps_ties() {
        let scored = vec![(5, 10), (9, 3), (5, 1), (-2, 7), (9, 4), (2995, 0)];
        assert_eq!(rank(vec![scored.clone()]), vec![0, 3, 4, 10, 1, 7]);
        // In parts: the same as one list.
        assert_eq!(rank(vec![scored[..2].to_vec(), vec![], scored[2..].to_vec()]), vec![0, 3, 4, 10, 1, 7]);
        // The same as a stable sort by score, best first.
        let mut s = scored;
        s.sort_by_key(|x| std::cmp::Reverse(x.0));
        assert_eq!(rank(vec![s.clone()]), s.iter().map(|x| x.1).collect::<Vec<_>>());
        assert!(rank(Vec::new()).is_empty() && rank(vec![vec![]]).is_empty());
        // One score: the order of the parts.
        assert_eq!(rank(vec![vec![(7, 4), (7, 2)], vec![(7, 9)]]), vec![4, 2, 9]);
        // A wide range takes the sort.
        assert_eq!(rank(vec![vec![(i32::MIN, 1), (i32::MAX, 2), (0, 3)]]), vec![2, 3, 1]);
    }

    #[test]
    fn scoring_in_parts_keeps_the_order() {
        let f = |i: usize| (!i.is_multiple_of(3)).then_some((i % 5) as i32);
        for n in [0, 10, 49_999, 50_000, 123_457] {
            let want: Vec<(i32, u32)> = (0..n).filter_map(|i| f(i).map(|s| (s, i as u32))).collect();
            let got: Vec<(i32, u32)> = score_all(&Candidates::All(n), &f).into_iter().flatten().collect();
            assert_eq!(got, want, "{n}");
            let some: Vec<u32> = (0..n as u32).filter(|i| !i.is_multiple_of(2)).collect();
            let want: Vec<(i32, u32)> = some.iter().filter_map(|&i| f(i as usize).map(|s| (s, i))).collect();
            let got: Vec<(i32, u32)> = score_all(&Candidates::Some(&some), &f).into_iter().flatten().collect();
            assert_eq!(got, want, "{n} some");
        }
    }

    #[test]
    fn in_order_sorts_unique_indices() {
        assert_eq!(in_order([130, 2, 64, 0, 63, 129].into_iter(), 131), vec![0, 2, 63, 64, 129, 130]);
        assert!(in_order(std::iter::empty(), 0).is_empty());
    }
}
