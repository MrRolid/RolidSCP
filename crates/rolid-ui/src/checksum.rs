//! Checksums of files: algorithms, the output formats of the usual tools
//! (`sha256sum`, `b3sum`, SFV for CRC32) and the server commands.

use sha1::Digest as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Algo {
    Md5,
    Sha1,
    #[default]
    Sha256,
    Sha512,
    Blake3,
    Crc32,
}

impl Algo {
    pub const ALL: [Algo; 6] = [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512, Algo::Blake3, Algo::Crc32];

    pub fn label(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::Sha512 => "SHA-512",
            Algo::Blake3 => "BLAKE3",
            Algo::Crc32 => "CRC32",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
            Algo::Sha256 => "sha256",
            Algo::Sha512 => "sha512",
            Algo::Blake3 => "blake3",
            Algo::Crc32 => "crc32",
        }
    }

    pub fn from_key(k: &str) -> Option<Algo> {
        Algo::ALL.into_iter().find(|a| a.key() == k)
    }

    /// The server tool computing the same (`None` = none, streamed over SFTP).
    pub fn tool(self) -> Option<&'static str> {
        match self {
            Algo::Md5 => Some("md5sum"),
            Algo::Sha1 => Some("sha1sum"),
            Algo::Sha256 => Some("sha256sum"),
            Algo::Sha512 => Some("sha512sum"),
            Algo::Blake3 => Some("b3sum"),
            Algo::Crc32 => None,
        }
    }

    /// Default name of the saved list.
    pub fn file_name(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5SUMS.txt",
            Algo::Sha1 => "SHA1SUMS.txt",
            Algo::Sha256 => "SHA256SUMS.txt",
            Algo::Sha512 => "SHA512SUMS.txt",
            Algo::Blake3 => "B3SUMS.txt",
            Algo::Crc32 => "CRC32.sfv",
        }
    }

    pub fn hasher(self) -> Hasher {
        match self {
            Algo::Md5 => Hasher::Md5(md5::Context::new()),
            Algo::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            Algo::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            Algo::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
            Algo::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            Algo::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
        }
    }
}

pub enum Hasher {
    Md5(md5::Context),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
    Crc32(crc32fast::Hasher),
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Hasher {
    pub fn update(&mut self, d: &[u8]) {
        match self {
            Hasher::Md5(h) => h.consume(d),
            Hasher::Sha1(h) => h.update(d),
            Hasher::Sha256(h) => h.update(d),
            Hasher::Sha512(h) => h.update(d),
            Hasher::Blake3(h) => {
                h.update(d);
            }
            Hasher::Crc32(h) => h.update(d),
        }
    }

    /// Lower-case hex (CRC32: 8 upper-case digits, as in SFV files).
    pub fn finish(self) -> String {
        match self {
            Hasher::Md5(h) => hex(&h.finalize().0),
            Hasher::Sha1(h) => hex(&h.finalize()),
            Hasher::Sha256(h) => hex(&h.finalize()),
            Hasher::Sha512(h) => hex(&h.finalize()),
            Hasher::Blake3(h) => h.finalize().to_hex().to_string(),
            Hasher::Crc32(h) => format!("{:08X}", h.finalize()),
        }
    }
}

/// Checksum of a local file; `on_bytes` gets each chunk's length, `cancel` stops it.
pub fn file_sum(
    path: &std::path::Path,
    algo: Algo,
    cancel: &std::sync::atomic::AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = algo.hasher();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::other(tr!("canceled")));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        on_bytes(n as u64);
    }
    Ok(h.finish())
}

/// One line of the saved list, as the tool writes it: `sum  path` (GNU
/// `*sum` and `b3sum`; a name with a backslash or a newline is escaped and the
/// line starts with `\`), for CRC32 SFV: `path SUM`.
pub fn line(algo: Algo, sum: &str, path: &str) -> String {
    if algo == Algo::Crc32 {
        return format!("{path} {sum}");
    }
    if path.contains(['\\', '\n', '\r']) {
        let esc = path.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");
        format!("\\{sum}  {esc}")
    } else {
        format!("{sum}  {path}")
    }
}

/// The whole list (each line ends with a newline; SFV starts with a comment).
pub fn listing(algo: Algo, rows: &[(String, String)]) -> String {
    let mut out = String::new();
    if algo == Algo::Crc32 {
        out += "; Created with RolidSCP\n";
    }
    for (path, sum) in rows {
        out += &line(algo, sum, path);
        out.push('\n');
    }
    out
}

/// Probe for the server tool: prints `GNU` (coreutils, `-z` works), `B3` or nothing.
pub fn probe_command(algo: Algo) -> Option<String> {
    let tool = algo.tool()?;
    Some(if algo == Algo::Blake3 {
        "command -v b3sum >/dev/null 2>&1 && echo B3".to_string()
    } else {
        format!("{tool} --version 2>/dev/null | head -n 1 | grep -q 'GNU coreutils' && echo GNU")
    })
}

/// Server command for several files (absolute paths): GNU `*sum -z` prints
/// `sum  path\0` per file, without escaping. `b3sum` gets one file at a time
/// (`--no-names`).
pub fn remote_command(algo: Algo, paths: &[String]) -> String {
    let q: Vec<String> = paths.iter().map(|p| rolid_search::remote::sh_quote(p)).collect();
    match algo {
        Algo::Blake3 => format!("b3sum --no-names -- {}", q.join(" ")),
        _ => format!("LC_ALL=C {} -z -- {}", algo.tool().unwrap_or("sha256sum"), q.join(" ")),
    }
}

/// Parse one `-z` record `sum  path` (`None` = not one).
pub fn parse_z_record(rec: &[u8]) -> Option<(String, String)> {
    let s = String::from_utf8_lossy(rec);
    let (sum, path) = s.split_once("  ")?;
    (!sum.is_empty() && sum.bytes().all(|b| b.is_ascii_hexdigit())).then(|| (sum.to_string(), path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sum_of(algo: Algo, data: &[u8]) -> String {
        let mut h = algo.hasher();
        h.update(data);
        h.finish()
    }

    #[test]
    fn known_values() {
        let d = b"abc";
        assert_eq!(sum_of(Algo::Md5, d), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(sum_of(Algo::Sha1, d), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sum_of(Algo::Sha256, d), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(sum_of(Algo::Sha512, d).starts_with("ddaf35a193617aba"));
        assert_eq!(sum_of(Algo::Blake3, d), "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");
        assert_eq!(sum_of(Algo::Crc32, d), "352441C2");
        for a in Algo::ALL {
            assert_eq!(Algo::from_key(a.key()), Some(a));
        }
    }

    /// The saved list is accepted by `sha256sum -c` / `md5sum -c` (names with
    /// spaces, quotes, diacritics, a backslash and a newline).
    #[test]
    fn listing_passes_the_check_tools() {
        let t = tempfile::tempdir().unwrap();
        let names = ["a.txt", "with space.txt", "quo'tes\"$(x).txt", "žluťoučký kůň.txt", "back\\slash", "new\nline", "sub/in subfolder"];
        std::fs::create_dir(t.path().join("sub")).unwrap();
        for (algo, tool) in [(Algo::Sha256, "sha256sum"), (Algo::Md5, "md5sum"), (Algo::Sha1, "sha1sum"), (Algo::Sha512, "sha512sum")] {
            let mut rows = Vec::new();
            for (i, n) in names.iter().enumerate() {
                let p = t.path().join(n);
                std::fs::write(&p, format!("content {i} {n}")).unwrap();
                let no = std::sync::atomic::AtomicBool::new(false);
                rows.push((n.to_string(), file_sum(&p, algo, &no, &mut |_| {}).unwrap()));
            }
            let list = t.path().join(algo.file_name());
            std::fs::write(&list, listing(algo, &rows)).unwrap();
            let out = Command::new(tool).arg("-c").arg("--strict").arg(&list).current_dir(t.path()).output().unwrap();
            assert!(out.status.success(), "{tool}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            // A changed file is found.
            std::fs::write(t.path().join("a.txt"), "changed").unwrap();
            let out = Command::new(tool).arg("-c").arg(&list).current_dir(t.path()).output().unwrap();
            assert!(!out.status.success());
            // And our sums equal the tool's.
            let out = Command::new(tool).arg("--").arg("with space.txt").current_dir(t.path()).output().unwrap();
            let theirs = String::from_utf8_lossy(&out.stdout);
            assert!(theirs.starts_with(&rows[1].1), "{theirs}");
        }
        assert_eq!(line(Algo::Crc32, "352441C2", "a b.txt"), "a b.txt 352441C2");
        assert!(listing(Algo::Crc32, &[]).starts_with(';'));
    }

    #[test]
    fn server_command_quoting_and_parsing() {
        let t = tempfile::tempdir().unwrap();
        let names = ["with space", "quo'tes\"`$(echo x)`", "new\nline", "žluťoučký kůň", "-dash", "*"];
        let paths: Vec<String> = names
            .iter()
            .map(|n| {
                let p = t.path().join(n);
                std::fs::write(&p, n.as_bytes()).unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        let out = Command::new("sh").arg("-c").arg(remote_command(Algo::Sha256, &paths)).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let recs: Vec<(String, String)> = out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()).filter_map(parse_z_record).collect();
        assert_eq!(recs.len(), names.len());
        for ((sum, path), (want_path, n)) in recs.iter().zip(paths.iter().zip(names)) {
            assert_eq!(path, want_path);
            assert_eq!(sum, &sum_of(Algo::Sha256, n.as_bytes()), "{n:?}");
        }
        assert!(probe_command(Algo::Crc32).is_none());
        let p = Command::new("sh").arg("-c").arg(probe_command(Algo::Md5).unwrap()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&p.stdout).trim(), "GNU");
        assert_eq!(parse_z_record(b"xyz  a"), None);
    }
}
