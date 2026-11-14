//! Quick filter used by panels: case-insensitive substring, falling back to
//! fuzzy subsequence matching. Returns a score (higher = better) or `None`.

pub fn score(name: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    // Most names are ASCII: compare without making lowercase copies (a panel
    // with a million files scores every name on each key of the filter).
    if name.is_ascii() && query.is_ascii() {
        return score_ascii(name.as_bytes(), query.as_bytes());
    }
    score_unicode(name, query)
}

/// [`score`] for ASCII text, without allocating.
fn score_ascii(n: &[u8], q: &[u8]) -> Option<i32> {
    let same = |a: &[u8]| a.iter().zip(q).all(|(x, y)| x.eq_ignore_ascii_case(y));
    if n.len() >= q.len() {
        if same(&n[..q.len()]) {
            return Some(3000 - n.len() as i32);
        }
        if let Some(pos) = (1..=n.len() - q.len()).find(|&i| same(&n[i..i + q.len()])) {
            return Some(2000 - pos as i32);
        }
    }
    let mut score = 1000;
    let mut last = 0usize;
    let mut i = 0usize;
    for qc in q {
        loop {
            let c = n.get(i)?;
            i += 1;
            if c.eq_ignore_ascii_case(qc) {
                score -= (i - 1 - last) as i32;
                last = i - 1;
                break;
            }
        }
    }
    Some(score)
}

fn score_unicode(name: &str, query: &str) -> Option<i32> {
    let n = name.to_lowercase();
    let q = query.to_lowercase();
    if n.starts_with(&q) {
        return Some(3000 - n.len() as i32);
    }
    if let Some(pos) = n.find(&q) {
        return Some(2000 - pos as i32);
    }
    // Fuzzy subsequence: all query chars appear in order.
    let mut score = 1000;
    let mut last = 0usize;
    let mut chars = n.char_indices();
    for qc in q.chars() {
        loop {
            match chars.next() {
                Some((i, c)) if c == qc => {
                    score -= (i - last) as i32;
                    last = i;
                    break;
                }
                Some(_) => continue,
                None => return None,
            }
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn ranking() {
        assert!(score("Cargo.toml", "car").unwrap() > score("mycar.txt", "car").unwrap());
        assert!(score("mycar.txt", "car").unwrap() > score("c_a_r.txt", "car").unwrap());
        assert!(score("readme", "xyz").is_none());
        assert_eq!(score("anything", ""), Some(0));
    }

    /// The fast path for ASCII gives the same scores as the general one.
    #[test]
    fn ascii_path_matches_the_general_one() {
        let names = ["Cargo.toml", "mycar.txt", "c_a_r.txt", "readme", "f0012345", "F12", "", "x", "aXbXc", "ab", "ABAB", "cart car"];
        let queries = ["car", "CAR", "f12", "f1", "x", "abc", "b", "ab", "aab", "ar", "zzz", "car "];
        for n in names {
            for q in queries {
                assert_eq!(super::score_ascii(n.as_bytes(), q.as_bytes()), super::score_unicode(n, q), "{n:?} / {q:?}");
            }
        }
        // Not ASCII: the general path.
        assert!(score("Ärger.txt", "ärg").is_some());
    }
}
