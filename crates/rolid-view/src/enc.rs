//! Text encodings of the viewer: detection (BOM, valid UTF-8, a guess among
//! the single-byte ones), decoding with the source position of every
//! character (selection, search hits and colors are kept as file offsets),
//! and finding line ends in each encoding.

use std::borrow::Cow;

/// The encodings the viewer can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Enc {
    Utf8,
    Utf16Le,
    Utf16Be,
    Win1250,
    Iso8859_2,
    Cp852,
    Win1252,
}

impl Enc {
    pub const ALL: [Enc; 7] = [Enc::Utf8, Enc::Utf16Le, Enc::Utf16Be, Enc::Win1250, Enc::Iso8859_2, Enc::Cp852, Enc::Win1252];

    /// The name shown in the menu (a technical name, not translated).
    pub fn name(self) -> &'static str {
        match self {
            Enc::Utf8 => "UTF-8",
            Enc::Utf16Le => "UTF-16 LE",
            Enc::Utf16Be => "UTF-16 BE",
            Enc::Win1250 => "Windows-1250",
            Enc::Iso8859_2 => "ISO-8859-2",
            Enc::Cp852 => "CP852",
            Enc::Win1252 => "Windows-1252",
        }
    }

    /// Bytes of one code unit (a line end is one unit).
    pub fn unit(self) -> usize {
        match self {
            Enc::Utf16Le | Enc::Utf16Be => 2,
            _ => 1,
        }
    }

    /// Every byte value below 0x80 means the ASCII character (searching
    /// for ASCII text directly in the bytes is right).
    pub fn ascii_compatible(self) -> bool {
        self.unit() == 1
    }

    /// Is there a line end (`\n`) at `i` of `data` (`i` aligned to the unit)?
    #[inline]
    pub fn newline_at(self, data: &[u8], i: usize) -> bool {
        match self {
            Enc::Utf16Le => data.get(i) == Some(&b'\n') && data.get(i + 1) == Some(&0),
            Enc::Utf16Be => data.get(i) == Some(&0) && data.get(i + 1) == Some(&b'\n'),
            _ => data.get(i) == Some(&b'\n'),
        }
    }

    /// Position of the first line end at or after `from` (aligned to the unit).
    pub fn find_newline(self, data: &[u8], from: usize) -> Option<usize> {
        if self.unit() == 1 {
            return memchr::memchr(b'\n', data.get(from..)?).map(|i| from + i);
        }
        let lf = if self == Enc::Utf16Le { 0 } else { 1 };
        let mut at = from;
        loop {
            let i = at + memchr::memchr(b'\n', data.get(at..)?)?;
            if i >= from + lf {
                let p = i - lf;
                if (p - from).is_multiple_of(2) && self.newline_at(data, p) {
                    return Some(p);
                }
            }
            at = i + 1;
        }
    }

    /// Position of the last line end that starts before `before`.
    /// `data[0]` must be aligned to the unit.
    pub fn rfind_newline(self, data: &[u8], before: usize) -> Option<usize> {
        let end = before.min(data.len());
        if self.unit() == 1 {
            return memchr::memrchr(b'\n', &data[..end]);
        }
        let lf = if self == Enc::Utf16Le { 0 } else { 1 };
        let mut stop = end;
        while let Some(i) = memchr::memrchr(b'\n', &data[..stop]) {
            if i >= lf {
                let p = i - lf;
                if p % 2 == 0 && p + 2 <= end && self.newline_at(data, p) {
                    return Some(p);
                }
            }
            stop = i;
        }
        None
    }

    /// Move `i` forward to the start of a character (at most 3 bytes for
    /// UTF-8; to the unit and past a low surrogate for UTF-16).
    pub fn char_start(self, data: &[u8], mut i: usize) -> usize {
        match self {
            Enc::Utf8 => {
                let mut n = 0;
                while n < 3 && i < data.len() && (data[i] & 0xC0) == 0x80 {
                    i += 1;
                    n += 1;
                }
                i
            }
            Enc::Utf16Le | Enc::Utf16Be => {
                i += i % 2;
                if let Some(u) = unit16(self, data, i) {
                    if (0xDC00..0xE000).contains(&u) {
                        return i + 2;
                    }
                }
                i
            }
            _ => i,
        }
    }

    /// Decode `data` (lossy: bad bytes become U+FFFD).
    pub fn decode(self, data: &[u8]) -> Cow<'_, str> {
        match self {
            Enc::Utf8 => String::from_utf8_lossy(data),
            Enc::Cp852 => Cow::Owned(data.iter().map(|&b| cp852(b)).collect()),
            _ => {
                let (s, _) = self.encoding_rs().unwrap().decode_without_bom_handling(data);
                s
            }
        }
    }

    /// Decode `data` and give for every character of the text the offset of
    /// its first byte in `data`.
    pub fn decode_mapped(self, data: &[u8]) -> (String, Vec<u32>) {
        let mut text = String::with_capacity(data.len());
        let mut src = Vec::with_capacity(data.len());
        match self {
            Enc::Utf8 => {
                let mut at = 0;
                while at < data.len() {
                    match std::str::from_utf8(&data[at..]) {
                        Ok(s) => {
                            push_str(&mut text, &mut src, s, at);
                            break;
                        }
                        Err(e) => {
                            let ok = e.valid_up_to();
                            // SAFETY-free: the prefix is valid UTF-8.
                            push_str(&mut text, &mut src, std::str::from_utf8(&data[at..at + ok]).unwrap(), at);
                            text.push('\u{FFFD}');
                            src.push((at + ok) as u32);
                            at += ok + e.error_len().unwrap_or(data.len() - at - ok);
                        }
                    }
                }
            }
            Enc::Utf16Le | Enc::Utf16Be => {
                let mut i = 0;
                while i + 1 < data.len() {
                    let u = unit16(self, data, i).unwrap();
                    if (0xD800..0xDC00).contains(&u) {
                        if let Some(lo) = unit16(self, data, i + 2).filter(|l| (0xDC00..0xE000).contains(l)) {
                            let c = 0x10000 + (((u as u32) - 0xD800) << 10) + (lo as u32 - 0xDC00);
                            text.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                            src.push(i as u32);
                            i += 4;
                            continue;
                        }
                    }
                    text.push(char::from_u32(u as u32).unwrap_or('\u{FFFD}'));
                    src.push(i as u32);
                    i += 2;
                }
                if i < data.len() {
                    text.push('\u{FFFD}');
                    src.push(i as u32);
                }
            }
            _ => {
                // Single-byte: one character per byte.
                let s = self.decode(data);
                for (i, c) in s.chars().enumerate() {
                    text.push(c);
                    src.push(i as u32);
                }
            }
        }
        (text, src)
    }

    /// Encode `s` (for searching in the bytes); `None` if a character has no
    /// code in this encoding.
    pub fn encode(self, s: &str) -> Option<Vec<u8>> {
        match self {
            Enc::Utf8 => Some(s.as_bytes().to_vec()),
            Enc::Utf16Le => Some(s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()),
            Enc::Utf16Be => Some(s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()),
            Enc::Cp852 => s.chars().map(|c| if (c as u32) < 0x80 { Some(c as u8) } else { CP852.chars().position(|x| x == c).map(|p| 0x80 + p as u8) }).collect(),
            _ => {
                let (b, _, bad) = self.encoding_rs().unwrap().encode(s);
                (!bad).then(|| b.into_owned())
            }
        }
    }

    fn encoding_rs(self) -> Option<&'static encoding_rs::Encoding> {
        Some(match self {
            Enc::Utf8 => encoding_rs::UTF_8,
            Enc::Utf16Le => encoding_rs::UTF_16LE,
            Enc::Utf16Be => encoding_rs::UTF_16BE,
            Enc::Win1250 => encoding_rs::WINDOWS_1250,
            Enc::Iso8859_2 => encoding_rs::ISO_8859_2,
            Enc::Win1252 => encoding_rs::WINDOWS_1252,
            Enc::Cp852 => return None,
        })
    }
}

fn push_str(text: &mut String, src: &mut Vec<u32>, s: &str, base: usize) {
    for (i, c) in s.char_indices() {
        text.push(c);
        src.push((base + i) as u32);
    }
}

fn unit16(enc: Enc, data: &[u8], i: usize) -> Option<u16> {
    let b = [*data.get(i)?, *data.get(i + 1)?];
    Some(if enc == Enc::Utf16Le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
}

/// The upper half of code page 852 (DOS Central European).
const CP852: &str = "ÇüéâäůćçłëŐőîŹÄĆÉĹĺôöĽľŚśÖÜŤťŁ×čáíóúĄąŽžĘę¬źČş«»░▒▓│┤ÁÂĚŞ╣║╗╝Żż┐└┴┬├─┼Ăă╚╔╩╦╠═╬¤đĐĎËďŇÍÎě┘┌█▄ŢŮ▀ÓßÔŃńňŠšŔÚŕŰýÝţ´\u{00AD}˝˛ˇ˘§÷¸°¨˙űŘř■\u{00A0}"; // i18n-ok: a code page table, not text

fn cp852(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else {
        CP852.chars().nth((b - 0x80) as usize).unwrap_or('\u{FFFD}')
    }
}

/// What the start of a file says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub enc: Enc,
    /// Bytes of the byte order mark (skipped).
    pub bom: usize,
    /// Looks like binary data (the hex view is offered).
    pub binary: bool,
}

/// Detect the encoding from the first bytes of a file (`whole` = the
/// sample is the whole file, so a cut character at its end is an error).
pub fn detect(sample: &[u8], whole: bool) -> Detected {
    let d = |enc, bom, binary| Detected { enc, bom, binary };
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return d(Enc::Utf8, 3, false);
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return d(Enc::Utf16Le, 2, false);
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return d(Enc::Utf16Be, 2, false);
    }
    if let Some(e) = utf16_without_bom(sample) {
        return d(e, 0, false);
    }
    if looks_binary(sample) {
        return d(Enc::Utf8, 0, true);
    }
    match std::str::from_utf8(sample) {
        Ok(_) => return d(Enc::Utf8, 0, false),
        Err(e) if !whole && e.error_len().is_none() => return d(Enc::Utf8, 0, false),
        Err(_) => {}
    }
    // Damaged UTF-8: mostly valid multi-byte characters, a few bad bytes.
    let (good, bad) = utf8_counts(sample, whole);
    if good > 0 && bad * 4 <= good {
        return d(Enc::Utf8, 0, false);
    }
    d(guess_single_byte(sample), 0, false)
}

/// Valid multi-byte UTF-8 characters and invalid sequences in `s`.
fn utf8_counts(s: &[u8], whole: bool) -> (usize, usize) {
    let (mut good, mut bad) = (0, 0);
    let mut at = 0;
    while at < s.len() {
        match std::str::from_utf8(&s[at..]) {
            Ok(t) => {
                good += t.chars().filter(|c| !c.is_ascii()).count();
                break;
            }
            Err(e) => {
                let ok = e.valid_up_to();
                good += std::str::from_utf8(&s[at..at + ok]).map_or(0, |t| t.chars().filter(|c| !c.is_ascii()).count());
                match e.error_len() {
                    Some(n) => {
                        bad += 1;
                        at += ok + n;
                    }
                    None => {
                        bad += usize::from(whole);
                        break;
                    }
                }
            }
        }
    }
    (good, bad)
}

/// Text in UTF-16 without a BOM: in an ASCII-heavy sample every other byte is 0.
fn utf16_without_bom(s: &[u8]) -> Option<Enc> {
    let n = s.len() / 2;
    if n < 8 {
        return None;
    }
    let even0 = s.iter().step_by(2).take(n).filter(|&&b| b == 0).count();
    let odd0 = s.iter().skip(1).step_by(2).take(n).filter(|&&b| b == 0).count();
    if odd0 * 10 >= n * 9 && even0 * 20 < n {
        Some(Enc::Utf16Le)
    } else if even0 * 10 >= n * 9 && odd0 * 20 < n {
        Some(Enc::Utf16Be)
    } else {
        None
    }
}

/// NUL bytes, or many control characters other than the usual whitespace.
pub fn looks_binary(s: &[u8]) -> bool {
    if s.is_empty() {
        return false;
    }
    if memchr::memchr(0, s).is_some() {
        return true;
    }
    let ctl = s.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B)).count();
    ctl * 20 > s.len()
}

/// The single-byte encoding under which the text reads most like a language
/// of its region: letters of the region weigh most, other letters a little;
/// a capital inside a word in small letters, C1 controls, undefined bytes
/// and box drawing weigh against. A tie goes to Windows-1250.
fn guess_single_byte(s: &[u8]) -> Enc {
    let central = "áäčďéěíĺľňóôöőŕřšťúůüűýžąćęłńśźżÁÄČĎÉĚÍĹĽŇÓÔÖŐŔŘŠŤÚŮÜŰÝŽĄĆĘŁŃŚŹŻ"; // i18n-ok: letters for the guess
    let western = "àáâäçèéêëíîïñóôöùúûüÿæœøåßÀÁÂÄÇÈÉÊËÍÎÏÑÓÔÖÙÚÛÜŸÆŒØÅ"; // i18n-ok: letters for the guess
    let mut best = (Enc::Win1250, i64::MIN);
    for enc in [Enc::Win1250, Enc::Iso8859_2, Enc::Cp852, Enc::Win1252] {
        let native = if enc == Enc::Win1252 { western } else { central };
        let text = enc.decode(s);
        let mut score = 0i64;
        let mut prev = ' ';
        for c in text.chars() {
            if !c.is_ascii() {
                score += if c.is_alphabetic() {
                    if c.is_uppercase() && prev.is_lowercase() {
                        -4
                    } else if native.contains(c) {
                        3
                    } else {
                        1
                    }
                } else if c == '\u{FFFD}' || ('\u{80}'..'\u{A0}').contains(&c) {
                    -6
                } else if ('\u{2500}'..'\u{2600}').contains(&c) {
                    -2
                } else if "–—„“”‚‘’…«»°§×÷€\u{A0}".contains(c) {
                    0
                } else {
                    -1
                };
            }
            prev = c;
        }
        if score > best.1 {
            best = (enc, score);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boms_are_recognised() {
        assert_eq!(detect(b"\xEF\xBB\xBFahoj", true), Detected { enc: Enc::Utf8, bom: 3, binary: false });
        assert_eq!(detect(b"\xFF\xFEa\0h\0", true).enc, Enc::Utf16Le);
        assert_eq!(detect(b"\xFE\xFF\0a\0h", true).enc, Enc::Utf16Be);
        let le: Vec<u8> = "hello world, this is text\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(detect(&le, true), Detected { enc: Enc::Utf16Le, bom: 0, binary: false });
    }

    #[test]
    fn utf8_cut_at_the_end_of_the_sample_is_still_utf8() {
        let s = "Příliš žluťoučký kůň".as_bytes();
        let cut = &s[..s.len() - 1]; // inside "ň"
        assert_eq!(detect(cut, false).enc, Enc::Utf8);
        // As a whole file it is damaged UTF-8, still UTF-8.
        assert_eq!(detect(cut, true).enc, Enc::Utf8);
    }

    #[test]
    fn single_byte_encodings_are_guessed() {
        let text = "Příliš žluťoučký kůň úpěl ďábelské ódy. Šťastný Žirafák.";
        for enc in [Enc::Win1250, Enc::Iso8859_2, Enc::Cp852] {
            let bytes = enc.encode(text).unwrap();
            assert_eq!(detect(&bytes, true).enc, enc, "{enc:?}");
            assert_eq!(enc.decode(&bytes), text);
        }
        let fr = Enc::Win1252.encode("Ça déjà été très bien, garçon – où êtes-vous?").unwrap();
        assert_eq!(detect(&fr, true).enc, Enc::Win1252);
    }

    #[test]
    fn damaged_utf8_stays_utf8() {
        let mut d = "Příliš žluťoučký kůň úpěl ďábelské ódy.\n".repeat(3).into_bytes();
        d.extend(b"bad \xff\xfe bytes, cut \xe2\x82 sequence\n");
        assert_eq!(detect(&d, true).enc, Enc::Utf8);
        let (t, _) = Enc::Utf8.decode_mapped(&d);
        assert!(t.contains("bad \u{FFFD}\u{FFFD} bytes, cut \u{FFFD} sequence"));
    }

    #[test]
    fn binary_is_recognised() {
        assert!(detect(b"\x7FELF\x02\x01\x01\0\0\0\0\0\0\0", true).binary);
        assert!(!detect(b"plain\ttext\r\n", true).binary);
    }

    #[test]
    fn broken_utf8_is_decoded_with_positions() {
        let data = b"a\xFFb\xC3\xA9c\xE2\x82";
        let (t, src) = Enc::Utf8.decode_mapped(data);
        assert_eq!(t, "a\u{FFFD}b\u{e9}c\u{FFFD}");
        assert_eq!(src, vec![0, 1, 2, 3, 5, 6]);
        // A lone continuation byte and a truncated sequence in the middle.
        let (t, src) = Enc::Utf8.decode_mapped(b"\x80x\xE2\x82y");
        assert_eq!(t, "\u{FFFD}x\u{FFFD}y");
        assert_eq!(src, vec![0, 1, 2, 4]);
    }

    #[test]
    fn utf16_is_decoded_with_positions() {
        let s = "a😀b";
        let le: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let (t, src) = Enc::Utf16Le.decode_mapped(&le);
        assert_eq!(t, s);
        assert_eq!(src, vec![0, 2, 6]);
        // Odd length and a lone surrogate.
        let (t, _) = Enc::Utf16Le.decode_mapped(&[0x00, 0xD8, b'x', 0, 7]);
        assert_eq!(t, "\u{FFFD}x\u{FFFD}");
    }

    #[test]
    fn line_ends_in_utf16_are_found_only_on_unit_boundaries() {
        // "\u{0A00}" is not a line end: its bytes are 00 0A in LE.
        let le: Vec<u8> = "a\u{0A00}b\nc".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(Enc::Utf16Le.find_newline(&le, 0), Some(6));
        assert_eq!(Enc::Utf16Le.rfind_newline(&le, le.len()), Some(6));
        let be: Vec<u8> = "a\u{0A00}b\nc".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(Enc::Utf16Be.find_newline(&be, 0), Some(6));
        assert_eq!(Enc::Utf16Be.rfind_newline(&be, be.len()), Some(6));
        assert_eq!(Enc::Utf8.find_newline(b"ab\ncd\n", 3), Some(5));
        assert_eq!(Enc::Utf8.rfind_newline(b"ab\ncd\n", 5), Some(2));
    }

    #[test]
    fn char_starts() {
        let s = "aé€😀".as_bytes();
        assert_eq!(Enc::Utf8.char_start(s, 2), 3);
        assert_eq!(Enc::Utf8.char_start(s, 4), 6);
        assert_eq!(Enc::Utf8.char_start(s, 7), 10);
    }
}
