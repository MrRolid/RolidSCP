//! Numbers, sizes and dates the way the chosen language writes them.

use std::time::SystemTime;

/// How a language writes numbers and dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    pub decimal: char,
    /// Thousands separator.
    pub group: &'static str,
    /// Group only numbers with at least this many digits (Polish and
    /// Spanish write 1234 but 12 345).
    pub group_from: usize,
    pub date: DateStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// 2026-11-08 21:08
    Iso,
    /// 08.11.2026 21:08
    Dots,
    /// 08/11/2026 21:08
    Slashes,
    /// 08.11.2026 21.08 (Danish time with a dot)
    DotsDotTime,
}

const NBSP: &str = "\u{a0}";
const NNBSP: &str = "\u{202f}";

impl Locale {
    pub const EN: Locale = Locale { decimal: '.', group: ",", group_from: 4, date: DateStyle::Iso };

    /// The conventions of a language code ("cs", "de"…); unknown = English
    /// numbers with ISO dates.
    pub fn of(lang: &str) -> Locale {
        let l = |decimal, group, group_from, date| Locale { decimal, group, group_from, date };
        match lang {
            "cs" => l(',', NBSP, 4, DateStyle::Dots),
            "de" => l(',', ".", 4, DateStyle::Dots),
            "fr" => l(',', NNBSP, 4, DateStyle::Slashes),
            "pl" => l(',', NBSP, 5, DateStyle::Dots),
            "es" => l(',', ".", 5, DateStyle::Slashes),
            "nb" => l(',', NBSP, 4, DateStyle::Dots),
            "da" => l(',', ".", 4, DateStyle::DotsDotTime),
            _ => Locale::EN,
        }
    }

    /// A whole number with the thousands separator.
    pub fn int(&self, v: i128) -> String {
        let digits = v.unsigned_abs().to_string();
        let mut out = String::new();
        if v < 0 {
            out.push('-');
        }
        if digits.len() < self.group_from {
            out.push_str(&digits);
            return out;
        }
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push_str(self.group);
            }
            out.push(c);
        }
        out
    }

    /// A decimal number with `places` decimals.
    pub fn decimal(&self, v: f64, places: usize) -> String {
        let s = format!("{:.*}", places, v.abs());
        let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
        let sign = if v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') { "-" } else { "" };
        let int = self.int(int.parse::<i128>().unwrap_or(0));
        if frac.is_empty() {
            format!("{sign}{int}")
        } else {
            format!("{sign}{int}{}{frac}", self.decimal)
        }
    }

    /// Date and time in local time (minutes, or seconds with `seconds`).
    pub fn datetime(&self, t: SystemTime, seconds: bool) -> String {
        let dt: chrono::DateTime<chrono::Local> = t.into();
        let fmt = match (self.date, seconds) {
            (DateStyle::Iso, false) => "%Y-%m-%d %H:%M",
            (DateStyle::Iso, true) => "%Y-%m-%d %H:%M:%S",
            (DateStyle::Dots, false) => "%d.%m.%Y %H:%M",
            (DateStyle::Dots, true) => "%d.%m.%Y %H:%M:%S",
            (DateStyle::Slashes, false) => "%d/%m/%Y %H:%M",
            (DateStyle::Slashes, true) => "%d/%m/%Y %H:%M:%S",
            (DateStyle::DotsDotTime, false) => "%d.%m.%Y %H.%M",
            (DateStyle::DotsDotTime, true) => "%d.%m.%Y %H.%M.%S",
        };
        dt.format(fmt).to_string()
    }

    /// A size in bytes, binary steps with the units given (already
    /// translated: "B", "kB", "MB"…): "512 B", "38.1 MB" / "38,1 MB".
    pub fn size(&self, n: u64, units: &[String; 6]) -> String {
        if n < 1024 {
            return format!("{} {}", self.int(n as i128), units[0]);
        }
        let mut v = n as f64;
        let mut u = 0;
        while v >= 1024.0 && u < units.len() - 1 {
            v /= 1024.0;
            u += 1;
        }
        format!("{} {}", self.decimal(v, 1), units[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units() -> [String; 6] {
        ["B", "kB", "MB", "GB", "TB", "PB"].map(String::from)
    }

    #[test]
    fn numbers_for_every_language() {
        let cases: [(&str, &str, &str, &str, &str); 8] = [
            // lang, 1234, 1234567, 38.1, -0.04 rounded to 1 place
            ("en", "1,234", "1,234,567", "38.1", "0.0"),
            ("cs", "1\u{a0}234", "1\u{a0}234\u{a0}567", "38,1", "0,0"),
            ("de", "1.234", "1.234.567", "38,1", "0,0"),
            ("fr", "1\u{202f}234", "1\u{202f}234\u{202f}567", "38,1", "0,0"),
            ("pl", "1234", "1\u{a0}234\u{a0}567", "38,1", "0,0"),
            ("es", "1234", "1.234.567", "38,1", "0,0"),
            ("nb", "1\u{a0}234", "1\u{a0}234\u{a0}567", "38,1", "0,0"),
            ("da", "1.234", "1.234.567", "38,1", "0,0"),
        ];
        for (lang, a, b, c, d) in cases {
            let l = Locale::of(lang);
            assert_eq!(l.int(1234), a, "{lang}");
            assert_eq!(l.int(1_234_567), b, "{lang}");
            assert_eq!(l.decimal(38.1, 1), c, "{lang}");
            assert_eq!(l.decimal(-0.04, 1), d, "{lang}");
            assert_eq!(l.int(999), "999");
            assert_eq!(l.int(-12_345), format!("-{}", l.int(12_345)));
        }
        assert_eq!(Locale::of("pl").int(12_345), "12\u{a0}345");
        assert_eq!(Locale::of("xx"), Locale::EN);
        let mb = 40_000_000u64;
        assert_eq!(Locale::of("en").size(mb, &units()), "38.1 MB");
        assert_eq!(Locale::of("cs").size(mb, &units()), "38,1 MB");
        assert_eq!(Locale::of("de").size(0, &units()), "0 B");
        assert_eq!(Locale::of("de").size(1023, &units()), "1.023 B");
    }

    #[test]
    fn dates_for_every_language() {
        use chrono::TimeZone;
        let t: SystemTime = chrono::Local.with_ymd_and_hms(2026, 11, 8, 21, 8, 5).unwrap().into();
        let want = [
            ("en", "2026-11-08 21:08"),
            ("cs", "08.11.2026 21:08"),
            ("de", "08.11.2026 21:08"),
            ("fr", "08/11/2026 21:08"),
            ("pl", "08.11.2026 21:08"),
            ("es", "08/11/2026 21:08"),
            ("nb", "08.11.2026 21:08"),
            ("da", "08.11.2026 21.08"),
        ];
        for (lang, s) in want {
            assert_eq!(Locale::of(lang).datetime(t, false), s, "{lang}");
        }
        assert_eq!(Locale::of("cs").datetime(t, true), "08.11.2026 21:08:05");
        assert_eq!(Locale::EN.datetime(t, true), "2026-11-08 21:08:05");
    }
}
