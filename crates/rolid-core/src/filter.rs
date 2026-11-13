//! Quick filter used by panels: case-insensitive substring, falling back to
//! fuzzy subsequence matching. Returns a score (higher = better) or `None`.

pub fn score(name: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
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
}
