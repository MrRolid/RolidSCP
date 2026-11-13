//! The hexadecimal view: rows of [`WIDTH`] bytes with their position, the
//! bytes and the characters (printable ASCII, a dot for the rest).

/// Bytes per row.
pub const WIDTH: usize = 16;

/// Digits of the position for a file of `size` bytes (at least 8).
pub fn offset_digits(size: u64) -> usize {
    let need = (64 - size.max(1).leading_zeros()).div_ceil(4) as usize;
    need.max(8)
}

/// One row: position, bytes, characters. A short last row is padded so the
/// characters stay in their column.
pub fn row(offset: u64, bytes: &[u8], digits: usize) -> (String, String, String) {
    let pos = format!("{offset:0digits$X}");
    let mut hex = String::with_capacity(WIDTH * 3 + 1);
    for i in 0..WIDTH {
        if i == WIDTH / 2 {
            hex.push(' ');
        }
        match bytes.get(i) {
            Some(b) => hex.push_str(&format!("{b:02X} ")),
            None => hex.push_str("   "),
        }
    }
    hex.pop();
    let chars = bytes.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }).collect();
    (pos, hex, chars)
}

/// The byte under column `col` of the hex part (`None` = a gap).
pub fn byte_at_column(col: usize) -> Option<usize> {
    // "00 01 … 07  08 … 0F": three characters a byte, one more after the eighth.
    let (half, c) = if col > WIDTH / 2 * 3 { (WIDTH / 2, col - (WIDTH / 2 * 3 + 1)) } else { (0, col) };
    let i = c / 3;
    (c % 3 != 2 && i < WIDTH / 2).then_some(half + i)
}

/// Column of byte `i` in the hex part.
pub fn column_of_byte(i: usize) -> usize {
    if i < WIDTH / 2 {
        i * 3
    } else {
        i * 3 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_show_position_bytes_and_characters() {
        let (p, h, c) = row(0x1234, b"Hello\0\xFFworld!\n\t~", 8);
        assert_eq!(p, "00001234");
        assert_eq!(h, "48 65 6C 6C 6F 00 FF 77  6F 72 6C 64 21 0A 09 7E");
        assert_eq!(c, "Hello..world!..~");
        // A short last row keeps the columns.
        let (_, h, c) = row(0, b"AB", 8);
        assert_eq!(h.len(), WIDTH * 3);
        assert!(h.starts_with("41 42 "));
        assert_eq!(c, "AB");
    }

    #[test]
    fn position_digits_grow_with_the_file() {
        assert_eq!(offset_digits(0), 8);
        assert_eq!(offset_digits(0xFFFF_FFFF), 8);
        assert_eq!(offset_digits(0x1_0000_0000), 9);
        assert_eq!(offset_digits(5 << 40), 11);
    }

    #[test]
    fn columns_and_bytes_match() {
        for i in 0..WIDTH {
            let col = column_of_byte(i);
            assert_eq!(byte_at_column(col), Some(i));
            assert_eq!(byte_at_column(col + 1), Some(i));
            assert_eq!(byte_at_column(col + 2), None, "{i}");
        }
    }
}
