//! Fuzzing of reading archives: damaged zip, 7z, tar, tar.gz and tar.zst
//! files (also encrypted ones) must give an error or a partial result,
//! never a panic, a hang or a file written outside the target. Each
//! mutated archive is listed, tested and extracted (with the bomb limits
//! lowered so a damaged size field cannot fill the disk).
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rolid_archive::*;
use secrecy::SecretString;

fn ctl_run<T>(f: impl FnOnce(&Ctl) -> T) -> T {
    let cancel = AtomicBool::new(false);
    let on = |_: Tick<'_>| {};
    f(&Ctl { cancel: &cancel, on: &on })
}

fn ropts(pw: Option<&str>) -> ReadOpts {
    ReadOpts { password: pw.map(|p| SecretString::from(p.to_string())), ..ReadOpts::new() }
}

/// A small tree: text, a compressible file, an empty file, a subfolder, a link.
fn tree(root: &Path) -> PathBuf {
    let d = root.join("data");
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("a.txt"), "hello world\n").unwrap();
    fs::write(d.join("sub/b.txt"), "b".repeat(3000)).unwrap();
    fs::write(d.join("empty"), "").unwrap();
    std::os::unix::fs::symlink("a.txt", d.join("link")).unwrap();
    d
}

/// Archives of the tree in `format` (plain, and encrypted when it can be).
fn seeds(format: Format) -> Vec<(Vec<u8>, Option<&'static str>)> {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let mut out = Vec::new();
    let pws: &[Option<&'static str>] = if format.can_encrypt() { &[None, Some("pw")] } else { &[None] };
    for &pw in pws {
        let file = t.path().join(format!("seed{}", format.ext()));
        let pass = pw.map(|p| SecretString::from(p.to_string()));
        ctl_run(|c| pack(std::slice::from_ref(&src), &file, format, Level::Fast, pass.as_ref(), c)).unwrap();
        out.push((fs::read(&file).unwrap(), pw));
    }
    out
}

/// Remove the target of the last input: folders extracted with the mode
/// from the archive (e.g. r-x) are made writable first.
fn wipe(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fn open_up(d: &Path) {
        let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o700));
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                open_up(&e.path());
            }
        }
    }
    if dir.exists() {
        open_up(dir);
        fs::remove_dir_all(dir).unwrap();
    }
}

fn fuzz(format: Format, dict: &[&[u8]]) {
    let t = tempfile::tempdir().unwrap();
    let seeds = seeds(format);
    let passwords: Vec<Option<&str>> = seeds.iter().map(|s| s.1).collect();
    let bytes: Vec<Vec<u8>> = seeds.into_iter().map(|s| s.0).collect();
    let file = t.path().join(format!("in{}", format.ext()));
    let dest = t.path().join("out");
    let limits = Limits { max_total: 50 << 20, max_entries: 10_000, max_ratio: Some(1000) };
    let mut k = 0usize;
    let mut check = |input: &[u8]| {
        // Try with no password and with the right one of an encrypted seed.
        let pw = passwords[k % passwords.len()];
        k += 1;
        fs::write(&file, input).unwrap();
        let o = ropts(pw);
        let _ = list(&file, format, &o);
        let _ = ctl_run(|c| test(&file, format, &o, c));
        wipe(&dest);
        fs::create_dir(&dest).unwrap();
        let _ = ctl_run(|c| extract(&file, format, &dest, &o, limits, Exist::Overwrite, None, c));
        // Nothing outside the target, and no part file left behind.
        let outside: Vec<_> = fs::read_dir(t.path()).unwrap().flatten().map(|e| e.file_name()).filter(|n| n != "out" && n != file.file_name().unwrap()).collect();
        assert!(outside.is_empty(), "written outside the target: {outside:?}");
    };
    if mutate::replay(&mut check) {
        return;
    }
    mutate::run(&format!("archive-{}", format.ext().trim_start_matches('.')), &bytes, dict, check);
}

#[test]
fn fuzz_zip() {
    fuzz(Format::Zip, &[b"PK\x03\x04", b"PK\x01\x02", b"PK\x05\x06", b"PK\x06\x06", b"PK\x06\x07", b"UT", b"\x01\x99"]);
}

#[test]
fn fuzz_sevenz() {
    fuzz(Format::SevenZ, &[b"7z\xbc\xaf\x27\x1c", b"\x17\x06", b"\x01\x04\x06", b"\x06\xf1\x07\x01"]);
}

#[test]
fn fuzz_tar() {
    fuzz(Format::Tar, &[b"ustar\x0000", b"ustar  \x00", b"././@LongLink", b"PaxHeader", b"path=", b"linkpath=", b"0000777"]);
}

#[test]
fn fuzz_tar_gz() {
    fuzz(Format::TarGz, &[b"\x1f\x8b\x08", b"\x1f\x8b\x08\x00\x00\x00\x00\x00"]);
}

#[test]
fn fuzz_tar_zst() {
    fuzz(Format::TarZst, &[b"\x28\xb5\x2f\xfd", b"\x50\x2a\x4d\x18"]);
}
