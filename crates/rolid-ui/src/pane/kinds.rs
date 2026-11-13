//! File categories by extension (and permissions): icon and text color of
//! each item in the panels.

use eframe::egui::Color32;
use rolid_core::{Entry, EntryKind};

use crate::icons;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Folder,
    Symlink,
    Archive,
    Image,
    Code,
    Text,
    Document,
    Audio,
    Video,
    Executable,
    Other,
}

impl FileKind {
    /// Categories with their own color (settings, in this order).
    pub const COLORED: [FileKind; 10] = [
        FileKind::Folder,
        FileKind::Symlink,
        FileKind::Archive,
        FileKind::Image,
        FileKind::Code,
        FileKind::Text,
        FileKind::Document,
        FileKind::Audio,
        FileKind::Video,
        FileKind::Executable,
    ];

    pub fn of(e: &Entry) -> FileKind {
        match e.kind {
            EntryKind::Dir => return FileKind::Folder,
            EntryKind::Symlink if e.link_to_dir => return FileKind::Folder,
            EntryKind::Symlink => return FileKind::Symlink,
            EntryKind::Other => return FileKind::Other,
            EntryKind::File => {}
        }
        let by_ext = FileKind::of_name(&e.name);
        let exec = e.mode.is_some_and(|m| m & 0o111 != 0);
        match by_ext {
            // An executable script or a file without a known type.
            FileKind::Code | FileKind::Text | FileKind::Other if exec => FileKind::Executable,
            k => k,
        }
    }

    /// Category by the extension only.
    pub fn of_name(name: &str) -> FileKind {
        let lower = name.to_ascii_lowercase();
        let multi = [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"];
        if multi.iter().any(|m| lower.ends_with(m)) {
            return FileKind::Archive;
        }
        let Some((stem, ext)) = lower.rsplit_once('.') else {
            return match lower.as_str() {
                "makefile" | "dockerfile" | "cmakelists.txt" => FileKind::Code,
                "readme" | "license" | "licence" | "changelog" | "copying" => FileKind::Text,
                _ => FileKind::Other,
            };
        };
        if stem.is_empty() {
            // ".bashrc", ".gitignore": configuration text.
            return FileKind::Text;
        }
        match ext {
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "lz" | "lzma" | "cab" | "iso"
            | "deb" | "rpm" | "jar" | "war" | "apk" => FileKind::Archive,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tif" | "tiff" | "ico" | "heic" | "avif"
            | "raw" | "cr2" | "nef" | "psd" | "xcf" => FileKind::Image,
            "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "kt" | "go" | "py" | "rb" | "php" | "js"
            | "mjs" | "ts" | "tsx" | "jsx" | "sh" | "bash" | "zsh" | "fish" | "pl" | "lua" | "swift" | "scala"
            | "sql" | "html" | "htm" | "css" | "scss" | "vue" | "toml" | "yaml" | "yml" | "json" | "xml"
            | "ini" | "conf" | "cfg" | "ps1" | "bat" | "cmd" | "mk" | "cmake" | "gradle" | "dart" | "zig"
            | "hs" | "ex" | "exs" | "erl" | "r" => FileKind::Code,
            "txt" | "md" | "log" | "csv" | "tsv" | "nfo" | "rst" | "adoc" | "srt" | "diff" | "patch" => FileKind::Text,
            "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" | "epub"
            | "djvu" => FileKind::Document,
            "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "m4a" | "aac" | "wma" | "mid" | "midi" => FileKind::Audio,
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" => FileKind::Video,
            "exe" | "msi" | "bin" | "run" | "appimage" | "com" => FileKind::Executable,
            _ => FileKind::Other,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            FileKind::Folder => icons::FOLDER,
            FileKind::Symlink => icons::LINK,
            FileKind::Archive => icons::FILE_ZIP,
            FileKind::Image => icons::FILE_IMAGE,
            FileKind::Code => icons::FILE_CODE,
            FileKind::Text => icons::FILE_TEXT,
            FileKind::Document => icons::FILE_DOC,
            FileKind::Audio => icons::FILE_AUDIO,
            FileKind::Video => icons::FILE_VIDEO,
            FileKind::Executable => icons::TERMINAL,
            FileKind::Other => icons::FILE,
        }
    }

    /// Key in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            FileKind::Folder => "folder",
            FileKind::Symlink => "symlink",
            FileKind::Archive => "archive",
            FileKind::Image => "image",
            FileKind::Code => "code",
            FileKind::Text => "text",
            FileKind::Document => "document",
            FileKind::Audio => "audio",
            FileKind::Video => "video",
            FileKind::Executable => "executable",
            FileKind::Other => "other",
        }
    }

    /// Name of the category for the user (settings: colors).
    pub fn label(self) -> String {
        match self {
            FileKind::Folder => trc!("kind", "Folders"),
            FileKind::Symlink => trc!("kind", "Links"),
            FileKind::Archive => trc!("kind", "Archives"),
            FileKind::Image => trc!("kind", "Images"),
            FileKind::Code => trc!("kind", "Source code"),
            FileKind::Text => trc!("kind", "Text"),
            FileKind::Document => trc!("kind", "Documents"),
            FileKind::Audio => trc!("kind", "Audio"),
            FileKind::Video => trc!("kind", "Video"),
            FileKind::Executable => trc!("kind", "Executables"),
            FileKind::Other => trc!("kind", "Other"),
        }
    }

    /// Default text color (readable on the light panel background, ≥ 4.5 : 1).
    pub fn default_color(self, dark: bool) -> Option<Color32> {
        let (l, d) = match self {
            FileKind::Folder => ((0x1a, 0x4f, 0xa0), (0x8c, 0xb8, 0xff)),
            FileKind::Symlink => ((0x00, 0x66, 0x66), (0x6c, 0xd8, 0xd8)),
            FileKind::Archive => ((0x9a, 0x3c, 0x00), (0xff, 0xa8, 0x60)),
            FileKind::Image => ((0x8a, 0x2b, 0x9a), (0xe0, 0x9c, 0xf0)),
            FileKind::Code => ((0x2a, 0x62, 0x1a), (0x9c, 0xe0, 0x84)),
            FileKind::Text => return None,
            FileKind::Document => ((0x7a, 0x4a, 0x00), (0xf0, 0xc0, 0x60)),
            FileKind::Audio => ((0x9a, 0x1f, 0x5c), (0xff, 0x9c, 0xc8)),
            FileKind::Video => ((0x6a, 0x3a, 0xb0), (0xc0, 0xa8, 0xff)),
            FileKind::Executable => ((0xa8, 0x1c, 0x1c), (0xff, 0x8a, 0x8a)),
            FileKind::Other => return None,
        };
        let (r, g, b) = if dark { d } else { l };
        Some(Color32::from_rgb(r, g, b))
    }
}

/// "#rrggbb" → color.
pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// Text colors of the categories as the panels use them.
#[derive(Clone, Debug, Default)]
pub struct KindColors {
    pub on: bool,
    /// Overrides from the settings (light theme).
    pub custom: std::collections::BTreeMap<String, String>,
}

impl KindColors {
    pub fn color(&self, k: FileKind, dark: bool) -> Option<Color32> {
        if !self.on {
            return None;
        }
        if !dark {
            if let Some(c) = self.custom.get(k.key()).and_then(|s| parse_hex(s)) {
                return Some(c);
            }
        }
        k.default_color(dark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast ratio of two colors.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn name_colors_are_readable_on_every_tab_color() {
        use crate::pane::views::{CURSOR_DARK, CURSOR_LIGHT, MARKED_DARK, MARKED_LIGHT};
        for dark in [false, true] {
            // The list background as the app paints it: text background tinted by the tab.
            let base = if dark { Color32::from_gray(10) } else { Color32::WHITE };
            let mut backgrounds: Vec<(String, Color32)> = rolid_core::color::PASTEL_PRESETS
                .iter()
                .map(|(n, c)| (n.to_string(), crate::app::blend(base, Color32::from_rgb(c.0, c.1, c.2), crate::app::LIST_TINT)))
                .collect();
            backgrounds.push(("cursor".into(), if dark { CURSOR_DARK } else { CURSOR_LIGHT }));
            let mut colors: Vec<(String, Color32)> =
                FileKind::COLORED.iter().filter_map(|&k| Some((k.label(), k.default_color(dark)?))).collect();
            colors.push(("marked".into(), if dark { MARKED_DARK } else { MARKED_LIGHT }));
            colors.push(("text".into(), if dark { crate::look::TEXT_DARK } else { crate::look::TEXT_LIGHT }));
            colors.push(("hints".into(), if dark { crate::look::WEAK_DARK } else { crate::look::WEAK_LIGHT }));
            for (bn, bg) in &backgrounds {
                for (cn, c) in &colors {
                    let r = contrast(*c, *bg);
                    assert!(r >= 4.5, "{cn} on {bn} (dark: {dark}): {r:.2} : 1");
                }
            }
        }
    }

    fn file(name: &str, mode: Option<u32>) -> Entry {
        Entry { name: name.into(), kind: EntryKind::File, link_to_dir: false, size: 1, modified: None, mode }
    }

    #[test]
    fn categories_by_extension() {
        use FileKind::*;
        for (n, k) in [
            ("a.JPG", Image),
            ("b.tar.gz", Archive),
            ("c.zip", Archive),
            ("main.rs", Code),
            ("Makefile", Code),
            ("note.txt", Text),
            ("README", Text),
            (".bashrc", Text),
            ("contract.PDF", Document),
            ("x.docx", Document),
            ("song.flac", Audio),
            ("film.mkv", Video),
            ("setup.exe", Executable),
            ("data.bin", Executable),
            ("unknown.xyz", Other),
            ("no_extension", Other),
        ] {
            assert_eq!(FileKind::of_name(n), k, "{n}");
        }
    }

    #[test]
    fn permissions_and_kinds() {
        assert_eq!(FileKind::of(&file("script.sh", Some(0o755))), FileKind::Executable);
        assert_eq!(FileKind::of(&file("script.sh", Some(0o644))), FileKind::Code);
        assert_eq!(FileKind::of(&file("program", Some(0o755))), FileKind::Executable);
        // Images copied from a FAT disk keep their category despite x bits.
        assert_eq!(FileKind::of(&file("photo.jpg", Some(0o777))), FileKind::Image);
        let mut d = file("dir", None);
        d.kind = EntryKind::Dir;
        assert_eq!(FileKind::of(&d), FileKind::Folder);
        d.kind = EntryKind::Symlink;
        assert_eq!(FileKind::of(&d), FileKind::Symlink);
        d.link_to_dir = true;
        assert_eq!(FileKind::of(&d), FileKind::Folder);
    }

    #[test]
    fn colors_can_be_changed_or_turned_off() {
        let mut kc = KindColors { on: true, ..Default::default() };
        assert_eq!(kc.color(FileKind::Image, false), FileKind::Image.default_color(false));
        assert_eq!(kc.color(FileKind::Text, false), None, "plain text keeps the normal color");
        kc.custom.insert("image".into(), "#112233".into());
        assert_eq!(kc.color(FileKind::Image, false), Some(Color32::from_rgb(0x11, 0x22, 0x33)));
        assert_eq!(to_hex(Color32::from_rgb(0x11, 0x22, 0x33)), "#112233");
        assert_eq!(parse_hex("#12"), None);
        kc.on = false;
        assert_eq!(kc.color(FileKind::Image, false), None);
    }

    #[test]
    fn default_colors_are_readable() {
        // WCAG relative luminance contrast against the light panel background.
        fn lum(c: Color32) -> f32 {
            let ch = |v: u8| {
                let s = v as f32 / 255.0;
                if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        let bg = Color32::from_gray(248);
        for k in FileKind::COLORED {
            if let Some(c) = k.default_color(false) {
                let r = (lum(bg) + 0.05) / (lum(c) + 0.05);
                assert!(r >= 4.5, "{k:?}: {r}");
            }
        }
    }
}
