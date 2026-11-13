//! Folder history of one tab (Back / Forward, Alt+← / Alt+→).

/// Most folders kept in each direction.
pub const MAX: usize = 50;

#[derive(Debug, Default, Clone)]
pub struct History {
    back: Vec<String>,
    forward: Vec<String>,
    /// The next listing comes from Back / Forward: do not record it as a visit.
    navigating: bool,
}

impl History {
    /// The panel moved from `from` to another folder (not via Back / Forward).
    pub fn left(&mut self, from: &str) {
        if std::mem::take(&mut self.navigating) || from.is_empty() {
            return;
        }
        if self.back.last().map(String::as_str) != Some(from) {
            self.back.push(from.to_string());
            if self.back.len() > MAX {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Back from `current`: the folder to open.
    pub fn go_back(&mut self, current: &str) -> Option<String> {
        let to = self.back.pop()?;
        self.forward.push(current.to_string());
        self.navigating = true;
        Some(to)
    }

    /// Forward from `current`.
    pub fn go_forward(&mut self, current: &str) -> Option<String> {
        let to = self.forward.pop()?;
        self.back.push(current.to_string());
        self.navigating = true;
        Some(to)
    }

    /// The listing after Back / Forward failed: forget the pending flag.
    pub fn cancel_nav(&mut self) {
        self.navigating = false;
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Recent folders, newest first, without duplicates (for the "*" list).
    pub fn recent(&self, current: &str, max: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.back.iter().rev().chain(self.forward.iter().rev()) {
            if p != current && !out.contains(p) {
                out.push(p.clone());
            }
            if out.len() == max {
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward() {
        let mut h = History::default();
        assert!(!h.can_back() && !h.can_forward());
        h.left("/a"); // /a -> /b
        h.left("/b"); // /b -> /c
        assert_eq!(h.go_back("/c").as_deref(), Some("/b"));
        h.left("/c"); // the listing of /b arrives: not a new visit
        assert!(h.can_forward());
        assert_eq!(h.go_back("/b").as_deref(), Some("/a"));
        h.left("/b");
        assert!(!h.can_back());
        assert_eq!(h.go_forward("/a").as_deref(), Some("/b"));
        h.left("/a");
        assert_eq!(h.go_forward("/b").as_deref(), Some("/c"));
        h.left("/b");
        assert!(!h.can_forward());
        // A new visit after going back drops the forward list.
        assert_eq!(h.go_back("/c").as_deref(), Some("/b"));
        h.left("/c");
        h.left("/b"); // /b -> /x
        assert!(!h.can_forward());
        assert_eq!(h.recent("/x", 10), vec!["/b".to_string(), "/a".to_string()]);
    }

    #[test]
    fn limited_and_no_repeats() {
        let mut h = History::default();
        for i in 0..80 {
            h.left(&format!("/d{i}"));
            h.left(&format!("/d{i}")); // same folder twice is kept once
        }
        assert_eq!(h.back.len(), MAX);
        assert_eq!(h.go_back("/now").as_deref(), Some("/d79"));
    }
}
