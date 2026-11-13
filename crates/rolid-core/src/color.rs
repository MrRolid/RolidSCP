//! Tab colors: any 24-bit RGB value plus 16 soft pastel presets.

use rolid_text::n;

/// 24-bit RGB color (16.7M possible values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `#RRGGBB` or `RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Rgb(p(0)?, p(2)?, p(4)?))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// Relative luminance (WCAG), used to pick readable text color.
    pub fn luminance(self) -> f32 {
        let c = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(self.0) + 0.7152 * c(self.1) + 0.0722 * c(self.2)
    }

    /// Black or white text, whichever contrasts better with this background.
    pub fn text_color(self) -> Rgb {
        if self.luminance() > 0.179 {
            Rgb(0x1A, 0x1A, 0x1A)
        } else {
            Rgb(0xFF, 0xFF, 0xFF)
        }
    }
}

/// 16 soft pastel presets, ordered around the color wheel. The names are
/// English, marked with `n!` (the UI translates them).
pub const PASTEL_PRESETS: [(&str, Rgb); 16] = [
    (n!("Pink"), Rgb(0xFF, 0xD1, 0xDC)),
    (n!("Salmon"), Rgb(0xFF, 0xD6, 0xC9)),
    (n!("Peach"), Rgb(0xFF, 0xE0, 0xB8)),
    (n!("Apricot"), Rgb(0xFF, 0xEB, 0xB3)),
    (n!("Lemon"), Rgb(0xFF, 0xF7, 0xB8)),
    (n!("Lime"), Rgb(0xE8, 0xF7, 0xB8)),
    (n!("Pistachio"), Rgb(0xD4, 0xF0, 0xC0)),
    (n!("Mint"), Rgb(0xC1, 0xF0, 0xD6)),
    (n!("Aquamarine"), Rgb(0xBF, 0xEF, 0xE8)),
    (n!("Turquoise"), Rgb(0xC2, 0xE9, 0xF2)),
    (n!("Sky blue"), Rgb(0xC9, 0xE2, 0xFF)),
    (n!("Cornflower"), Rgb(0xCF, 0xD8, 0xFF)),
    (n!("Lavender"), Rgb(0xDC, 0xD3, 0xFF)),
    (n!("Lilac"), Rgb(0xE8, 0xD1, 0xFA)),
    (n!("Orchid"), Rgb(0xF5, 0xD0, 0xF0)),
    (n!("Pearl"), Rgb(0xE8, 0xE6, 0xE1)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Rgb::from_hex("#a1B2c3").unwrap();
        assert_eq!(c, Rgb(0xA1, 0xB2, 0xC3));
        assert_eq!(c.to_hex(), "#A1B2C3");
        assert!(Rgb::from_hex("#12345").is_none());
        assert!(Rgb::from_hex("zzzzzz").is_none());
    }

    #[test]
    fn pastels_use_dark_text() {
        for (_, c) in PASTEL_PRESETS {
            assert_eq!(c.text_color(), Rgb(0x1A, 0x1A, 0x1A));
        }
    }
}
