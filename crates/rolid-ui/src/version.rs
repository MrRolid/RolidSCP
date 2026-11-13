//! Version and the pre-release label.
//!
//! Until 1.0 only the last number grows (0.8.1, 0.8.2, …); 1.0 is the first
//! production release. The label after the version (window title, About)
//! is controlled here alone: set [`PRERELEASE`] to `None` for 1.0.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shown after the version in the window title and in About. `None` = no label.
/// Marked with `n!`, translated when shown.
/// Not translated on purpose: the same label in every language.
pub const PRERELEASE: Option<&str> = Some("Beta - developer version");

/// "0.8.1 (Beta - developer version)" or just "0.8.1".
pub fn full_version_with(label: Option<&str>) -> String {
    match label {
        Some(l) => format!("{VERSION} ({l})"),
        None => VERSION.to_string(),
    }
}

pub fn full_version() -> String {
    full_version_with(PRERELEASE)
}

/// "RolidSCP 0.8.1 (Beta - developer version) — <location>".
pub fn window_title_with(label: Option<&str>, location: &str) -> String {
    format!("RolidSCP {} — {location}", full_version_with(label))
}

pub fn window_title(location: &str) -> String {
    window_title_with(PRERELEASE, location)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_with_and_without_the_beta_label() {
        assert_eq!(
            window_title_with(Some("Beta - developer version"), "Test SFTP:/srv"),
            format!("RolidSCP {VERSION} (Beta - developer version) — Test SFTP:/srv")
        );
        assert_eq!(window_title_with(None, "home"), format!("RolidSCP {VERSION} — home"));
        assert_eq!(full_version_with(None), VERSION);
        assert!(full_version().starts_with(VERSION));
    }
}
