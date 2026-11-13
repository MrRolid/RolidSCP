//! Editing files in an external editor with automatic upload (F4).
//!
//! - [`temp`]: private temporary directories (0700, one per edited file)
//! - [`watch`]: watching the directory for saves (also atomic saves), debounce,
//!   content fingerprints
//! - [`conflict`]: did the remote file change since it was downloaded?
//! - [`editor`]: the external editor command (`{file}` placeholder, defaults)
//! - [`transfer`]: download and safe upload (`.filepart`, never a truncated file)
//! - [`recover`]: leftovers after a crash (instance markers, per-edit metadata)

pub mod conflict;
pub mod editor;
pub mod apps;
pub mod recover;
pub mod temp;
pub mod transfer;
pub mod watch;

/// Files above this size need a confirmation before editing.
pub const LARGE_FILE: u64 = 50 * 1024 * 1024;

/// A file is treated as binary when its beginning contains a NUL byte.
pub fn looks_binary(data: &[u8]) -> bool {
    data.contains(&0)
}

/// Read the beginning of a local file and check it for NUL bytes.
pub fn file_looks_binary(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut buf = vec![0u8; 64 * 1024];
    let mut f = std::fs::File::open(path)?;
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(looks_binary(&buf[..n]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn binary_detection() {
        assert!(super::looks_binary(b"ELF\0\x01"));
        assert!(!super::looks_binary("Příliš žluťoučký kůň\n".as_bytes()));
    }
}
