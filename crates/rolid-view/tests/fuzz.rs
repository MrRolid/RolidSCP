//! Fuzzing of reading texts in the viewer: encoding detection, decoding in
//! every encoding (also with the map back to bytes), moving by characters
//! and lines, and the line index. Any bytes must give some text, never a
//! panic or an endless loop.
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

use rolid_view::enc::{detect, looks_binary};
use rolid_view::index::{skip_lines, LineIndex};
use rolid_view::Enc;

fn utf16(s: &str, le: bool) -> Vec<u8> {
    let mut v = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
    for u in s.encode_utf16() {
        v.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
    }
    v
}

#[test]
fn fuzz_texts_in_every_encoding() {
    let text = "Příliš žluťoučký kůň\r\núpěl ďábelské ódy\nline 3 \u{1F600}\n\tend";
    let seeds = vec![
        text.as_bytes().to_vec(),
        [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        utf16(text, true),
        utf16(text, false),
        b"P\xf8\xedli\xb9 \x9elu\x9dou\xe8k\xfd k\xf9\xf2\r\n".to_vec(), // windows-1250
        b"\x00\x01\x02binary\xff\xfe\x00\x00".to_vec(),
    ];
    let dict: &[&[u8]] = &[b"\r\n", b"\n", b"\r", b"\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF", b"\xF0\x9F\x98", b"\xD8\x3D", b"\x00\x0A", b"\x0A\x00"];
    let check = |b: &[u8]| {
        let _ = looks_binary(b);
        for whole in [true, false] {
            let _ = detect(b, whole);
        }
        for enc in Enc::ALL {
            let _ = enc.decode(b);
            // One offset per character, rising, inside the data (callers
            // map hits and colors back to bytes with it).
            let (m, map) = enc.decode_mapped(b);
            assert_eq!(m.chars().count(), map.len(), "{enc:?}");
            assert!(map.windows(2).all(|w| w[0] < w[1]), "{enc:?}: offsets not rising");
            assert!(map.iter().all(|&o| (o as usize) < b.len()), "{enc:?}: map past the end");
            // Lines forward and back end.
            let mut at = 0;
            let mut n = 0;
            while let Some(p) = enc.find_newline(b, at) {
                assert!(p >= at && p < b.len());
                at = p + enc.unit();
                n += 1;
            }
            let mut before = b.len();
            while let Some(p) = enc.rfind_newline(b, before) {
                assert!(p < before);
                before = p;
            }
            // Forward to a character start, at most 3 bytes, never past the end
            // (callers pass positions inside the data).
            for i in [0, b.len() / 3, b.len() / 2, b.len().saturating_sub(1)].into_iter().filter(|&i| i < b.len()) {
                let c = enc.char_start(b, i);
                assert!(c >= i && c <= i + 3 && c <= b.len(), "{enc:?}: {i} -> {c} of {}", b.len());
            }
            let _ = skip_lines(enc, b, n / 2 + 1);
            let mut idx = LineIndex::new(enc, 0);
            idx.set_hint(b);
            let half = b.len() / 2;
            idx.feed(&b[..half]);
            idx.feed(&b[half..]);
            idx.finish();
            let _ = idx.total(b.len() as u64);
            let _ = idx.line_at(b.len() as u64 / 2, None);
            let _ = idx.seek_line(n / 2);
            let _ = idx.estimate_offset(n + 5);
        }
    };
    if mutate::replay(check) {
        return;
    }
    mutate::run("text", &seeds, dict, check);
}
