//! Masks for "Select / Deselect group" (`*.jpg;*.png`), as in Total Commander:
//! `*` any text, `?` one character, case-insensitive, several masks separated
//! by `;`.

/// Does `name` match any of the `;`-separated masks? An empty list matches nothing.
pub fn matches(masks: &str, name: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    masks
        .split(';')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .any(|m| glob(&m.to_lowercase().chars().collect::<Vec<_>>(), &name))
}

fn glob(p: &[char], s: &[char]) -> bool {
    // Iterative matcher with backtracking on the last '*'.
    let (mut pi, mut si) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_masks() {
        assert!(matches("*.jpg", "holiday.JPG"));
        assert!(matches("*.jpg; *.png", "a.png"));
        assert!(!matches("*.jpg;*.png", "a.gif"));
        assert!(matches("*", "anything"));
        assert!(matches("photo??.*", "photo01.webp"));
        assert!(!matches("photo??.*", "photo1.webp"));
        assert!(matches("*report*", "Q3-Report-final.pdf"));
        assert!(matches("a*b*c", "aXXbYYc"));
        assert!(!matches("a*b*c", "aXXbYY"));
        assert!(!matches("", "a"));
        assert!(!matches(" ; ", "a"));
        assert!(matches("*.tar.gz", "backup.tar.gz"));
        assert!(!matches("*.tar.gz", "backup.gz"));
    }
}
