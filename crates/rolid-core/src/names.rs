//! File names that are not valid UTF-8 (roadmap 0.8.15: odd names).
//!
//! On Linux a file name is any bytes except `/` and NUL; the program keeps
//! names as `String`. Each byte that is not part of valid UTF-8 is carried as
//! one character from the end of Unicode's last private use plane,
//! U+10FF80 + (byte − 0x80), so the exact bytes come back when the file is
//! opened, copied, renamed or deleted on the local disk. A real character
//! from that range (practically never used) is carried byte by byte the same
//! way, so every name goes there and back unchanged.
//!
//! Such names are shown with `\xNN` for each odd byte and control characters
//! as `\n`, `\t`, `\xNN` (never as an empty or broken line). An SFTP server
//! gets names as UTF-8 text, so a name with odd bytes cannot be sent there.

use std::borrow::Cow;

const FIRST: u32 = 0x10_FF80;

fn escape(b: u8) -> char {
    char::from_u32(FIRST + (b as u32 - 0x80)).expect("in the Unicode range")
}

fn is_escape(c: char) -> bool {
    (FIRST..=0x10_FFFF).contains(&(c as u32))
}

/// The name (or path) for the program: valid UTF-8 as it is, other bytes as
/// escape characters.
pub fn from_bytes(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(s) = std::str::from_utf8(bytes) {
        if !s.chars().any(is_escape) {
            return Cow::Borrowed(s);
        }
    }
    let mut out = String::with_capacity(bytes.len() + 8);
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if is_escape(c) {
                let mut b = [0u8; 4];
                out.extend(c.encode_utf8(&mut b).bytes().map(escape));
            } else {
                out.push(c);
            }
        }
        out.extend(chunk.invalid().iter().map(|&b| escape(b)));
    }
    Cow::Owned(out)
}

/// The exact bytes of a name (or path) made by [`from_bytes`].
pub fn to_bytes(s: &str) -> Cow<'_, [u8]> {
    if !has_escapes(s) {
        return Cow::Borrowed(s.as_bytes());
    }
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if is_escape(c) {
            out.push((c as u32 - FIRST + 0x80) as u8);
        } else {
            let mut b = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        }
    }
    Cow::Owned(out)
}

/// The name holds bytes that are not valid UTF-8 (it cannot go to a server).
pub fn has_escapes(s: &str) -> bool {
    // Every escape character is encoded as F4 8F BE xx or F4 8F BF xx.
    s.as_bytes().contains(&0xF4) && s.chars().any(is_escape)
}

/// The name as it is shown: odd bytes as `\xNN`, control characters as
/// `\n`, `\r`, `\t` or `\xNN`. Borrowed when there is nothing to change.
pub fn visible(s: &str) -> Cow<'_, str> {
    let odd = |c: char| c.is_control() || is_escape(c);
    if !s.chars().any(odd) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if is_escape(c) => out.push_str(&format!("\\x{:02X}", c as u32 - FIRST + 0x80)),
            c if (c as u32) < 0x100 && c.is_control() => out.push_str(&format!("\\x{:02X}", c as u32)),
            c if c.is_control() => out.push_str(&format!("\\u{{{:X}}}", c as u32)),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A local path for the operating system.
#[cfg(unix)]
pub fn to_os(s: &str) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&to_bytes(s)))
}

/// A local path for the operating system.
#[cfg(not(unix))]
pub fn to_os(s: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(s)
}

/// A local name or path from the operating system, without losing bytes.
#[cfg(unix)]
pub fn from_os(s: &std::ffi::OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    from_bytes(s.as_bytes()).into_owned()
}

/// A local name or path from the operating system.
#[cfg(not(unix))]
pub fn from_os(s: &std::ffi::OsStr) -> String {
    s.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_bytes_go_there_and_back() {
        let samples: &[&[u8]] = &[
            b"plain.txt",
            b"caf\xc3\xa9",
            b"bad\xff\xfe.txt",
            b"\x80",
            b"half \xe6\x97",
            b"\xf4\x8f\xbe\x80 real escape character",
            "日本".as_bytes(),
            b"",
        ];
        for &b in samples {
            let s = from_bytes(b);
            assert_eq!(&*to_bytes(&s), b, "{s:?}");
        }
        assert!(matches!(from_bytes(b"plain"), Cow::Borrowed(_)));
        assert!(!has_escapes("caf\u{e9} 日本"));
        assert!(has_escapes(&from_bytes(b"bad\xff")));
        // Two different byte names never become the same text.
        assert_ne!(from_bytes(b"a\xff"), from_bytes(b"a\xfe"));
        assert_ne!(from_bytes(b"a\xff"), "a\u{FFFD}");
    }

    #[test]
    fn names_are_shown_readably() {
        assert_eq!(visible("plain"), "plain");
        assert!(matches!(visible("plain 日本"), Cow::Borrowed(_)));
        assert_eq!(visible("new\nline\ttab\r"), "new\\nline\\ttab\\r");
        assert_eq!(visible("bell\u{7}"), "bell\\x07");
        assert_eq!(visible(&from_bytes(b"bad\xff.txt")), "bad\\xFF.txt");
        assert_eq!(visible(&from_bytes(b"x\xe6\x97")), "x\\xE6\\x97");
    }

    #[cfg(unix)]
    #[test]
    fn paths_for_the_system_keep_the_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let s = from_bytes(b"/tmp/bad\xff/f");
        assert_eq!(to_os(&s).as_os_str().as_bytes(), b"/tmp/bad\xff/f");
        assert_eq!(from_os(to_os(&s).as_os_str()), s);
    }
}
