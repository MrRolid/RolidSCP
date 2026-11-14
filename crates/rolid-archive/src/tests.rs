use std::fs;
use std::io::Write;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use secrecy::SecretString;

use super::*;

fn ctl_run<T>(f: impl FnOnce(&Ctl) -> T) -> T {
    let cancel = AtomicBool::new(false);
    let on = |_: Tick<'_>| {};
    f(&Ctl { cancel: &cancel, on: &on })
}

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

fn ropts(p: Option<&str>) -> ReadOpts {
    ReadOpts { password: p.map(pw), ..ReadOpts::new() }
}

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777
}

/// A small tree with the cases packing must keep.
fn tree(root: &Path) -> PathBuf {
    let d = root.join("data");
    fs::create_dir_all(d.join("sub/deep")).unwrap();
    fs::write(d.join("a.txt"), "hello world\n").unwrap();
    fs::set_permissions(d.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    fs::write(d.join("sub/b.bin"), &big).unwrap();
    fs::write(d.join("empty.txt"), "").unwrap();
    fs::write(d.join("exec.sh"), "#!/bin/sh\necho ok\n").unwrap();
    fs::set_permissions(d.join("exec.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(d.join("žluťoučký kůň.txt"), "úpěl ďábelské ódy").unwrap();
    fs::write(d.join("sub/deep/c.txt"), "c").unwrap();
    symlink("a.txt", d.join("link")).unwrap();
    symlink("../a.txt", d.join("sub/back")).unwrap();
    d
}

fn check_tree(src: &Path, out: &Path) {
    for f in ["a.txt", "sub/b.bin", "empty.txt", "exec.sh", "žluťoučký kůň.txt", "sub/deep/c.txt"] {
        assert_eq!(fs::read(src.join(f)).unwrap(), fs::read(out.join(f)).unwrap(), "{f}");
    }
    assert_eq!(mode(&out.join("a.txt")), 0o640);
    assert_eq!(mode(&out.join("exec.sh")), 0o755);
    assert_eq!(fs::read_link(out.join("link")).unwrap(), Path::new("a.txt"));
    assert_eq!(fs::read_link(out.join("sub/back")).unwrap(), Path::new("../a.txt"));
}

fn roundtrip(format: Format, password: Option<&str>) {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let out = t.path().join(format!("arch{}", format.ext()));
    for level in Level::ALL {
        let rep = ctl_run(|c| pack(std::slice::from_ref(&src), &out, format, level, password.map(pw).as_ref(), c)).unwrap();
        assert!(rep.failures.is_empty(), "{:?}", rep.failures);
        assert_eq!(rep.files, 8, "{format:?}");
        assert!(!part_path(&out).exists(), "no .filepart left");
    }
    let l = list(&out, format, &ropts(password)).unwrap();
    assert!(l.rejected.is_empty());
    let names: Vec<&str> = l.items.iter().map(|i| i.path.as_str()).collect();
    for n in ["data", "data/a.txt", "data/sub/b.bin", "data/link", "data/žluťoučký kůň.txt"] {
        assert!(names.contains(&n), "{format:?}: {n} in {names:?}");
    }
    let link = l.items.iter().find(|i| i.path == "data/link").unwrap();
    assert_eq!(link.kind, Kind::Symlink);
    assert_eq!(l.root_names(), vec!["data".to_string()]);

    let dest = t.path().join("extracted");
    fs::create_dir(&dest).unwrap();
    let rep = ctl_run(|c| extract(&out, format, &dest, &ropts(password), Limits::default(), Exist::Overwrite, None, c))
        .unwrap();
    assert!(rep.failures.is_empty(), "{format:?}: {:?}", rep.failures);
    check_tree(&src, &dest.join("data"));
    let tr = ctl_run(|c| test(&out, format, &ropts(password), c)).unwrap();
    assert!(tr.bad.is_empty(), "{:?}", tr.bad);
    assert!(tr.bytes >= 300_000);
}

#[test]
fn zip_roundtrip() {
    roundtrip(Format::Zip, None);
}

#[test]
fn sevenz_roundtrip() {
    roundtrip(Format::SevenZ, None);
}

#[test]
fn tar_gz_roundtrip() {
    roundtrip(Format::TarGz, None);
}

#[test]
fn tar_zst_roundtrip() {
    roundtrip(Format::TarZst, None);
}

#[test]
fn tar_roundtrip() {
    roundtrip(Format::Tar, None);
}

#[test]
fn encrypted_zip_roundtrip() {
    roundtrip(Format::Zip, Some("Secret password 123"));
    encrypted_needs_the_right_password(Format::Zip);
}

#[test]
fn encrypted_7z_roundtrip() {
    roundtrip(Format::SevenZ, Some("Secret password 123"));
    encrypted_needs_the_right_password(Format::SevenZ);
}

fn encrypted_needs_the_right_password(format: Format) {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let out = t.path().join(format!("x{}", format.ext()));
    ctl_run(|c| pack(&[src], &out, format, Level::Fast, Some(&pw("correct")), c)).unwrap();
    let raw = fs::read(&out).unwrap();
    assert!(!raw.windows(10).any(|w| w == b"hello worl"), "content is not readable without the key");
    let dest = t.path().join("extracted");
    fs::create_dir(&dest).unwrap();
    for (p, want) in [(None, "need"), (Some("wrong"), "bad")] {
        let e = ctl_run(|c| extract(&out, format, &dest, &ropts(p), Limits::default(), Exist::Overwrite, None, c))
            .unwrap_err();
        match (want, &e) {
            ("need", Error::NeedPassword) | ("bad", Error::BadPassword) => {}
            _ => panic!("{format:?} {p:?}: {e:?}"),
        }
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0, "nothing left after a wrong password");
        let e = ctl_run(|c| test(&out, format, &ropts(p), c)).unwrap_err();
        assert!(matches!(e, Error::NeedPassword | Error::BadPassword), "{e:?}");
    }
    if format == Format::SevenZ {
        // Names are encrypted as well: listing asks for the password.
        assert!(matches!(list(&out, format, &ropts(None)), Err(Error::NeedPassword)));
    } else {
        let l = list(&out, format, &ropts(None)).unwrap();
        assert!(l.encrypted);
    }
}

#[test]
fn tar_formats_cannot_be_encrypted() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let out = t.path().join("x.tar.gz");
    let e = ctl_run(|c| pack(&[src], &out, Format::TarGz, Level::Fast, Some(&pw("x")), c)).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)));
    assert!(!Format::TarGz.can_encrypt() && !Format::TarZst.can_encrypt());
    assert!(Format::Zip.can_encrypt() && Format::SevenZ.can_encrypt());
}

#[test]
fn formats_by_name() {
    assert_eq!(Format::from_name("a.ZIP"), Some(Format::Zip));
    assert_eq!(Format::from_name("a.tar.gz"), Some(Format::TarGz));
    assert_eq!(Format::from_name("a.tgz"), Some(Format::TarGz));
    assert_eq!(Format::from_name("a.tar.zst"), Some(Format::TarZst));
    assert_eq!(Format::from_name("a.7z"), Some(Format::SevenZ));
    assert_eq!(Format::from_name("a.tar"), Some(Format::Tar));
    assert_eq!(Format::from_name(".zip"), None);
    assert_eq!(Format::from_name("a.gz"), None);
    assert_eq!(strip_archive_ext("Photos.tar.gz"), "Photos");
    assert_eq!(strip_archive_ext("x.TGZ"), "x");
    assert_eq!(strip_archive_ext("x.txt"), "x.txt");
    for f in Format::PACK {
        assert_eq!(Format::from_key(f.key()), Some(f));
    }
}

// ---- security --------------------------------------------------------------

#[test]
fn unsafe_names_are_refused() {
    use Unsafe::*;
    for (n, e) in [
        ("../evil", Parent),
        ("a/../../evil", Parent),
        ("a/..", Parent),
        ("/etc/passwd", Absolute),
        ("\\windows\\x", Absolute),
        ("C:\\Windows\\x", Drive),
        ("c:/x", Drive),
        ("..\\..\\evil", Parent),
        ("a\\..\\..\\evil", Parent),
        ("", Empty),
        ("./", Empty),
        ("a\0b", Nul),
    ] {
        assert_eq!(safe_rel(n), Err(e), "{n:?}");
    }
    assert_eq!(safe_rel("a\\b.txt"), Ok("a/b.txt".into()));
    assert_eq!(safe_rel("./a//b/"), Ok("a/b".into()));
    assert_eq!(safe_rel("dir/"), Ok("dir".into()));
    assert_eq!(safe_rel("..a/b..").as_deref(), Ok("..a/b.."), "dots inside names are fine");
}

#[test]
fn links_must_stay_inside() {
    assert!(link_inside("a/link", "b.txt"));
    assert!(link_inside("a/link", "../b.txt"));
    assert!(link_inside("a/b/link", "../../c"));
    assert!(!link_inside("a/link", "../../c"));
    assert!(!link_inside("link", ".."));
    assert!(!link_inside("link", "/etc/passwd"));
    assert!(!link_inside("link", "C:\\x"));
    assert!(!link_inside("a/link", "..\\..\\x"));
    assert!(!link_inside("link", ""));
}

#[test]
fn setuid_and_setgid_are_dropped() {
    assert_eq!(safe_mode(0o4755), 0o755);
    assert_eq!(safe_mode(0o2750), 0o750);
    assert_eq!(safe_mode(0o1777), 0o1777, "sticky bit stays");
    assert_eq!(safe_mode(0o100644), 0o644);
}

/// Minimal stored zip with raw names (no UTF-8 flag), optional unix modes.
fn raw_zip(path: &Path, entries: &[(&[u8], &[u8], Option<u32>)]) {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, mode) in entries {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let off = out.len() as u32;
        let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
        let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        le32(&mut out, 0x04034b50);
        for x in [20u16, 0, 0, 0, 0x21] {
            le16(&mut out, x);
        }
        for x in [crc, data.len() as u32, data.len() as u32] {
            le32(&mut out, x);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        le32(&mut central, 0x02014b50);
        le16(&mut central, if mode.is_some() { (3 << 8) | 20 } else { 20 });
        for x in [20u16, 0, 0, 0, 0x21] {
            le16(&mut central, x);
        }
        for x in [crc, data.len() as u32, data.len() as u32] {
            le32(&mut central, x);
        }
        for x in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, mode.map(|m| m << 16).unwrap_or(0));
        le32(&mut central, off);
        central.extend_from_slice(name);
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    fs::write(path, out).unwrap();
}

fn extract_all(arch: &Path, format: Format, dest: &Path) -> ExtractReport {
    ctl_run(|c| extract(arch, format, dest, &ReadOpts::new(), Limits::default(), Exist::Overwrite, None, c)).unwrap()
}

/// Nothing appeared outside `dest` (in its parent `t`), apart from `allowed`.
fn nothing_outside(t: &Path, allowed: &[&str]) {
    for e in fs::read_dir(t).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        assert!(allowed.contains(&n.as_str()), "unexpected {n} outside the target");
    }
}

#[test]
fn zip_slip_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let arch = t.path().join("evil.zip");
    raw_zip(
        &arch,
        &[
            (b"../evil.txt", b"x", None),
            (b"/tmp/abs.txt", b"x", None),
            (b"C:/drive.txt", b"x", None),
            (b"a\\..\\..\\evil2.txt", b"x", None),
            (b"ok/fine.txt", b"fine", None),
            (b"out", b"/etc", Some(0o120777)),
            (b"out2", b"../../", Some(0o120777)),
            (b"inside", b"ok/fine.txt", Some(0o120777)),
            (b"setuid", b"#!/bin/sh", Some(0o104755)),
            (b"pipe", b"", Some(0o010644)),
        ],
    );
    let l = list(&arch, Format::Zip, &ReadOpts::new()).unwrap();
    assert_eq!(l.rejected.len(), 4, "{:?}", l.rejected);
    assert!(!l.items.iter().any(|i| i.path.contains("evil")));
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let rep = extract_all(&arch, Format::Zip, &dest);
    nothing_outside(t.path(), &["evil.zip", "dest"]);
    assert_eq!(fs::read(dest.join("ok/fine.txt")).unwrap(), b"fine");
    assert!(!dest.join("out").exists() && fs::symlink_metadata(dest.join("out")).is_err(), "link outside not created");
    assert!(fs::symlink_metadata(dest.join("out2")).is_err());
    assert_eq!(fs::read_link(dest.join("inside")).unwrap(), Path::new("ok/fine.txt"));
    assert_eq!(mode(&dest.join("setuid")), 0o755, "setuid dropped");
    assert!(fs::symlink_metadata(dest.join("pipe")).is_err(), "FIFO not created");
    // 4 bad names + 2 links outside + 1 FIFO in the summary.
    assert_eq!(rep.failures.len(), 7, "{:?}", rep.failures);
}

#[test]
fn no_writing_through_links() {
    // A link inside the target pointing out (created before) must not be
    // followed by later entries; so is a link the archive itself created.
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    symlink(&outside, dest.join("trap")).unwrap();
    let arch = t.path().join("a.zip");
    raw_zip(
        &arch,
        &[(b"here", b".", Some(0o120777)), (b"here/x.txt", b"x", None), (b"trap/y.txt", b"y", None)],
    );
    let rep = extract_all(&arch, Format::Zip, &dest);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0, "nothing written through the link");
    assert_eq!(rep.failures.len(), 2, "{:?}", rep.failures);
}

/// Tar header with raw name / link bytes (bypasses the tar crate's checks).
fn raw_tar_entry(b: &mut tar::Builder<Vec<u8>>, name: &[u8], ty: tar::EntryType, link: &[u8], data: &[u8], mode: u32) {
    let mut h = tar::Header::new_gnu();
    {
        let old = h.as_old_mut();
        old.name[..name.len()].copy_from_slice(name);
        old.linkname[..link.len()].copy_from_slice(link);
    }
    h.set_entry_type(ty);
    h.set_size(data.len() as u64);
    h.set_mode(mode);
    h.set_mtime(1_700_000_000);
    h.set_cksum();
    b.append(&h, data).unwrap();
}

#[test]
fn tar_links_devices_and_hardlinks() {
    use tar::EntryType as T;
    let t = tempfile::tempdir().unwrap();
    let mut b = tar::Builder::new(Vec::new());
    raw_tar_entry(&mut b, b"../evil.txt", T::Regular, b"", b"x", 0o644);
    raw_tar_entry(&mut b, b"/abs.txt", T::Regular, b"", b"x", 0o644);
    raw_tar_entry(&mut b, b"f.txt", T::Regular, b"", b"content", 0o6755);
    raw_tar_entry(&mut b, b"hard", T::Link, b"f.txt", b"", 0o644);
    raw_tar_entry(&mut b, b"hard-out", T::Link, b"../../etc/passwd", b"", 0o644);
    raw_tar_entry(&mut b, b"hard-abs", T::Link, b"/etc/passwd", b"", 0o644);
    raw_tar_entry(&mut b, b"soft-out", T::Symlink, b"/etc/passwd", b"", 0o777);
    raw_tar_entry(&mut b, b"device", T::Char, b"", b"", 0o644);
    raw_tar_entry(&mut b, b"pipe", T::Fifo, b"", b"", 0o644);
    let arch = t.path().join("a.tar");
    fs::write(&arch, b.into_inner().unwrap()).unwrap();
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let rep = extract_all(&arch, Format::Tar, &dest);
    nothing_outside(t.path(), &["a.tar", "dest"]);
    let mut names: Vec<String> = fs::read_dir(&dest).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["f.txt", "hard"]);
    assert_eq!(mode(&dest.join("f.txt")), 0o755, "setuid and setgid dropped");
    assert_eq!(fs::read(dest.join("hard")).unwrap(), b"content");
    assert_eq!(rep.failures.len(), 7, "{:?}", rep.failures);
}

#[test]
fn size_limit_stops_and_cleans_up() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arch = t.path().join("a.tar.zst");
    ctl_run(|c| pack(&[src], &arch, Format::TarZst, Level::Fast, None, c)).unwrap();
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("original.txt"), "stays").unwrap();
    let limits = Limits { max_total: 100_000, ..Limits::default() };
    let e = ctl_run(|c| extract(&arch, Format::TarZst, &dest, &ReadOpts::new(), limits, Exist::Overwrite, None, c))
        .unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e:?}");
    let left: Vec<_> = fs::read_dir(&dest).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["original.txt"], "only what was there before");
}

#[test]
fn entry_count_limit() {
    let t = tempfile::tempdir().unwrap();
    let arch = t.path().join("many.zip");
    let names: Vec<String> = (0..20).map(|i| format!("f{i}.txt")).collect();
    let entries: Vec<(&[u8], &[u8], Option<u32>)> = names.iter().map(|n| (n.as_bytes(), &b"x"[..], None)).collect();
    raw_zip(&arch, &entries);
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let limits = Limits { max_entries: 10, ..Limits::default() };
    let e = ctl_run(|c| extract(&arch, Format::Zip, &dest, &ReadOpts::new(), limits, Exist::Overwrite, None, c))
        .unwrap_err();
    assert!(matches!(e, Error::Limit(_)));
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    let o = ReadOpts { max_entries: 10, ..ReadOpts::new() };
    assert!(matches!(list(&arch, Format::Zip, &o), Err(Error::Limit(_))));
}

#[test]
fn compression_ratio_is_checked_while_writing() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("nuly");
    fs::create_dir(&d).unwrap();
    let mut f = fs::File::create(d.join("nuly.bin")).unwrap();
    let mb = vec![0u8; 1 << 20];
    for _ in 0..80 {
        f.write_all(&mb).unwrap();
    }
    drop(f);
    let arch = t.path().join("bomb.tar.zst");
    ctl_run(|c| pack(&[d], &arch, Format::TarZst, Level::Fast, None, c)).unwrap();
    let l = list(&arch, Format::TarZst, &ReadOpts::new()).unwrap();
    assert!(l.ratio() > 1000, "declared ratio {}", l.ratio());
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let e = ctl_run(|c| extract(&arch, Format::TarZst, &dest, &ReadOpts::new(), Limits::default(), Exist::Overwrite, None, c))
        .unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e:?}");
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    // The user allowed it: goes through.
    let ok = Limits { max_ratio: None, ..Limits::default() };
    let rep = ctl_run(|c| extract(&arch, Format::TarZst, &dest, &ReadOpts::new(), ok, Exist::Overwrite, None, c)).unwrap();
    assert_eq!(rep.bytes, 80 << 20);
}

#[test]
fn cancel_removes_only_what_it_created() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arch = t.path().join("a.zip");
    ctl_run(|c| pack(std::slice::from_ref(&src), &arch, Format::Zip, Level::Fast, None, c)).unwrap();
    let dest = t.path().join("dest");
    fs::create_dir_all(dest.join("data")).unwrap();
    fs::write(dest.join("data/own.txt"), "mine").unwrap();
    let cancel = AtomicBool::new(false);
    let seen = AtomicU64::new(0);
    let on = |tk: Tick<'_>| {
        if let Tick::Bytes(n) = tk {
            if seen.fetch_add(n, Ordering::Relaxed) + n > 100_000 {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    };
    let ctl = Ctl { cancel: &cancel, on: &on };
    let e = extract(&arch, Format::Zip, &dest, &ReadOpts::new(), Limits::default(), Exist::Overwrite, None, &ctl).unwrap_err();
    assert!(e.is_cancel());
    let mut left: Vec<String> = walkdir(&dest);
    left.sort();
    assert_eq!(left, ["data", "data/own.txt"], "the folder and the file that were there stay");

    // Packing: nothing left behind, not even the .filepart.
    let out = t.path().join("nove.7z");
    cancel.store(false, Ordering::Relaxed);
    seen.store(0, Ordering::Relaxed);
    let e = pack(&[src], &out, Format::SevenZ, Level::Fast, None, &ctl).unwrap_err();
    assert!(e.is_cancel());
    assert!(!out.exists() && !part_path(&out).exists());
}

fn walkdir(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    fn go(root: &Path, p: &Path, v: &mut Vec<String>) {
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            let path = e.path();
            v.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            if e.file_type().unwrap().is_dir() {
                go(root, &path, v);
            }
        }
    }
    go(root, root, &mut v);
    v
}

#[test]
fn existing_files_follow_the_rule() {
    let t = tempfile::tempdir().unwrap();
    let arch = t.path().join("a.zip");
    raw_zip(&arch, &[(b"x.txt", b"from archive", None)]);
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("x.txt"), "original").unwrap();
    let run = |ex| ctl_run(|c| extract(&arch, Format::Zip, &dest, &ReadOpts::new(), Limits::default(), ex, None, c)).unwrap();
    let r = run(Exist::Skip);
    assert_eq!(r.skipped, 1);
    assert_eq!(fs::read(dest.join("x.txt")).unwrap(), b"original");
    // The archive entry is from 1980: older, kept.
    run(Exist::Newer);
    assert_eq!(fs::read(dest.join("x.txt")).unwrap(), b"original");
    let r = run(Exist::Overwrite);
    assert!(r.created.is_empty(), "overwritten, not created");
    assert_eq!(fs::read(dest.join("x.txt")).unwrap(), b"from archive");
}

#[test]
fn cp852_names() {
    let name = "Příliš žluťoučký kůň.txt";
    let raw = oem_cp::encode_string_checked(name, &oem_cp::code_table::ENCODING_TABLE_CP852).unwrap();
    assert!(std::str::from_utf8(&raw).is_err());
    let t = tempfile::tempdir().unwrap();
    let arch = t.path().join("dos.zip");
    raw_zip(&arch, &[(&raw, b"x", None), (b"ascii.txt", b"y", None)]);
    let l = list(&arch, Format::Zip, &ReadOpts::new()).unwrap();
    assert_eq!(l.raw_names, vec![raw.clone()], "the user is offered an encoding");
    assert!(l.items.iter().any(|i| i.path == name), "CP852 is the default");
    assert_eq!(decode_name(&raw, Encoding::Cp852), name);
    assert_ne!(decode_name(&raw, Encoding::Cp437), name);
    assert_ne!(decode_name(&raw, Encoding::Cp1250), name);
    assert_eq!(decode_name("ž.txt".as_bytes(), Encoding::Cp437), "ž.txt", "valid UTF-8 stays");
    let o = ReadOpts { encoding: Encoding::Cp437, ..ReadOpts::new() };
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    ctl_run(|c| extract(&arch, Format::Zip, &dest, &ReadOpts::new(), Limits::default(), Exist::Overwrite, None, c)).unwrap();
    assert!(dest.join(name).exists());
    let l = list(&arch, Format::Zip, &o).unwrap();
    assert!(!l.items.iter().any(|i| i.path == name));
}

#[test]
fn damaged_archives_are_reported() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    for format in [Format::Zip, Format::SevenZ, Format::TarGz, Format::TarZst] {
        let arch = t.path().join(format!("a{}", format.ext()));
        ctl_run(|c| pack(std::slice::from_ref(&src), &arch, format, Level::Fast, None, c)).unwrap();
        let mut b = fs::read(&arch).unwrap();
        // Damage the middle of the data (the big pseudo-random file).
        let mid = b.len() / 2;
        for x in &mut b[mid..mid + 64] {
            *x ^= 0x5a;
        }
        fs::write(&arch, &b).unwrap();
        let r = ctl_run(|c| test(&arch, format, &ReadOpts::new(), c));
        match r {
            Ok(rep) => assert!(!rep.bad.is_empty(), "{format:?}: damage not found"),
            Err(e) => assert!(matches!(e, Error::Corrupt(_)), "{format:?}: {e:?}"),
        }
    }
    // Zip: the other items are still checked and fine.
    let arch = t.path().join("a.zip");
    let rep = ctl_run(|c| test(&arch, Format::Zip, &ReadOpts::new(), c)).unwrap();
    assert_eq!(rep.bad.len(), 1, "{:?}", rep.bad);
    assert_eq!(rep.bad[0].0, "data/sub/b.bin");
    assert!(rep.items > 5);
    // Not an archive at all.
    let junk = t.path().join("junk.zip");
    fs::write(&junk, b"this is not a zip").unwrap();
    let rep = ctl_run(|c| test(&junk, Format::Zip, &ReadOpts::new(), c)).unwrap();
    assert_eq!(rep.bad.len(), 1);
    assert_eq!(rep.bad[0].0, crate::WHOLE_ARCHIVE);
}

#[test]
fn extract_a_selection_without_the_base() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    let arch = t.path().join("a.7z");
    ctl_run(|c| pack(&[src], &arch, Format::SevenZ, Level::Fast, None, c)).unwrap();
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let sel = Select { base: "data".into(), items: vec!["data/sub".into(), "data/a.txt".into()] };
    assert_eq!(sel.map("data/sub/b.bin").as_deref(), Some("sub/b.bin"));
    assert_eq!(sel.map("data/subx"), None);
    assert_eq!(sel.map("data/exec.sh"), None);
    let rep = ctl_run(|c| extract(&arch, Format::SevenZ, &dest, &ReadOpts::new(), Limits::default(), Exist::Overwrite, Some(&sel), c))
        .unwrap();
    assert!(rep.failures.is_empty(), "{:?}", rep.failures);
    let mut got = walkdir(&dest);
    got.sort();
    assert_eq!(got, ["a.txt", "sub", "sub/b.bin", "sub/back", "sub/deep", "sub/deep/c.txt"]);
}

#[test]
fn level_mapping() {
    use Level::*;
    let v = |f, l| level_value(f, l);
    // Deflate (zip, tar.gz): 1 / 6 / 9; zip Store = Stored.
    assert_eq!([v(Format::Zip, Store), v(Format::Zip, Fast), v(Format::Zip, Normal), v(Format::Zip, Max)], [None, Some(1), Some(6), Some(9)]);
    assert_eq!([v(Format::TarGz, Fast), v(Format::TarGz, Normal), v(Format::TarGz, Max)], [Some(1), Some(6), Some(9)]);
    // LZMA2 presets: "Fast" is the lowest (0); Store = Copy.
    assert_eq!([v(Format::SevenZ, Store), v(Format::SevenZ, Fast), v(Format::SevenZ, Normal), v(Format::SevenZ, Max)], [None, Some(0), Some(5), Some(9)]);
    // Zstandard 1 / 3 / 19.
    assert_eq!([v(Format::TarZst, Fast), v(Format::TarZst, Normal), v(Format::TarZst, Max)], [Some(1), Some(3), Some(19)]);
    assert_eq!(v(Format::Tar, Max), None);
    // Store is offered only where the format has it.
    assert!(Format::Zip.levels().contains(&Store) && Format::SevenZ.levels().contains(&Store));
    assert!(!Format::TarGz.levels().contains(&Store) && !Format::TarZst.levels().contains(&Store));
    assert!(Format::Tar.levels().is_empty());
    for l in Level::ALL {
        assert_eq!(Level::from_key(l.key()), Some(l));
    }
    assert_eq!(Level::Store.label(), "No compression (store)");
}

#[test]
fn already_compressed_by_extension() {
    for n in ["film.mkv", "PHOTO.JPG", "a/b/music.Opus", "x.tar.gz", "book.epub", "app.apk", "d.docx", "z.7z", "s.zst"] {
        assert!(already_compressed(n), "{n}");
    }
    for n in ["text.txt", "mkv", ".jpg", "archive.tar", "image.bmp", "data.json", "jpg.txt", "a.mkv/b.txt"] {
        assert!(!already_compressed(n), "{n}");
    }
}

/// Compression method of each zip entry.
fn zip_methods(p: &Path) -> std::collections::BTreeMap<String, zip::CompressionMethod> {
    let mut z = zip::ZipArchive::new(fs::File::open(p).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let f = z.by_index_raw(i).unwrap();
            (f.name().unwrap().to_string(), f.compression())
        })
        .collect()
}

/// `tool args… archive` exits with 0 (skipped when the tool is missing).
fn external_ok(tool: &str, args: &[&str], arch: &Path) {
    let Ok(out) = std::process::Command::new(tool).args(args).arg(arch).output() else {
        eprintln!("{tool} missing – skipped");
        return;
    };
    assert!(out.status.success(), "{tool} {}: {}{}", arch.display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn zip_stores_compressed_files_and_the_store_level() {
    use zip::CompressionMethod::{Deflated, Stored};
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("d");
    fs::create_dir(&d).unwrap();
    let text = "Příliš žluťoučký kůň úpěl ďábelské ódy.\n".repeat(20_000);
    fs::write(d.join("text.txt"), &text).unwrap();
    fs::write(d.join("film.MKV"), &text).unwrap();
    fs::write(d.join("big.jpg"), text.repeat(2)).unwrap(); // above PAR_MAX in tests: written inline
    let out = t.path().join("a.zip");
    for (threads, store) in [(1, true), (4, true), (4, false)] {
        let o = PackOpts { threads, store_compressed: store, ..PackOpts::new(Format::Zip, Level::Normal) };
        ctl_run(|c| pack_with(std::slice::from_ref(&d), &out, &o, c)).unwrap();
        let m = zip_methods(&out);
        assert_eq!(m["d/text.txt"], Deflated);
        let want = if store { Stored } else { Deflated };
        assert_eq!((m["d/film.MKV"], m["d/big.jpg"]), (want, want), "threads {threads}");
        external_ok("unzip", &["-tq"], &out);
        external_ok("7z", &["t"], &out);
    }
    // "No compression (store)": everything stored.
    let o = PackOpts { threads: 4, ..PackOpts::new(Format::Zip, Level::Store) };
    ctl_run(|c| pack_with(std::slice::from_ref(&d), &out, &o, c)).unwrap();
    assert!(zip_methods(&out).iter().filter(|(n, _)| !n.ends_with('/')).all(|(_, m)| *m == Stored));
    assert!(fs::metadata(&out).unwrap().len() > 3 * text.len() as u64);
    external_ok("unzip", &["-tq"], &out);
    // 7z Copy: the archive is as big as the data, 7-Zip reads it.
    let out7 = t.path().join("a.7z");
    let o = PackOpts { threads: 4, ..PackOpts::new(Format::SevenZ, Level::Store) };
    ctl_run(|c| pack_with(std::slice::from_ref(&d), &out7, &o, c)).unwrap();
    assert!(fs::metadata(&out7).unwrap().len() > 4 * text.len() as u64);
    external_ok("7z", &["t"], &out7);
    let tr = ctl_run(|c| test(&out7, Format::SevenZ, &ReadOpts::new(), c)).unwrap();
    assert!(tr.bad.is_empty(), "{:?}", tr.bad);
}

#[test]
fn parallel_and_single_thread_give_the_same_content() {
    let t = tempfile::tempdir().unwrap();
    let src = tree(t.path());
    // More files than workers, of mixed sizes (small ones in workers, big inline).
    for i in 0..40u32 {
        let n = if i % 9 == 0 { 250_000 } else { 1_000 + i as usize * 37 };
        let data: Vec<u8> = (0..n as u32).map(|x| (x.wrapping_mul(i + 7) >> 5) as u8).collect();
        fs::write(src.join(format!("sub/f{i:02}.dat")), data).unwrap();
    }
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        for (threads, pass) in [(1, None), (6, None), (6, Some("secret"))] {
            if pass.is_some() && !format.can_encrypt() {
                continue;
            }
            let out = t.path().join(format!("p{threads}{}{}", if pass.is_some() { "-pw" } else { "" }, format.ext()));
            let o = PackOpts { threads, password: pass.map(pw), ..PackOpts::new(format, Level::Fast) };
            let rep = ctl_run(|c| pack_with(std::slice::from_ref(&src), &out, &o, c)).unwrap();
            assert_eq!(rep.files, 48, "{format:?}");
            let dest = t.path().join(format!("v{threads}{}{}", format.key(), pass.is_some()));
            fs::create_dir(&dest).unwrap();
            let rep = ctl_run(|c| extract(&out, format, &dest, &ropts(pass), Limits::default(), Exist::Overwrite, None, c)).unwrap();
            assert!(rep.failures.is_empty(), "{:?}", rep.failures);
            check_tree(&src, &dest.join("data"));
            for i in 0..40 {
                let f = format!("sub/f{i:02}.dat");
                assert_eq!(fs::read(src.join(&f)).unwrap(), fs::read(dest.join("data").join(&f)).unwrap());
            }
            if format == Format::Zip {
                // Stored like 7-Zip does (p7zip 16.02 rejects an encrypted
                // empty entry that is deflated).
                assert_eq!(zip_methods(&out)["data/empty.txt"], zip::CompressionMethod::Stored);
            }
            if let Some(pass) = pass {
                external_ok("7z", &["t", &format!("-p{pass}")], &out);
            } else {
                if format != Format::TarZst {
                    external_ok("7z", &["t"], &out);
                }
                if format == Format::Zip {
                    external_ok("unzip", &["-tq"], &out);
                }
            }
        }
    }
}

#[test]
fn cancel_while_workers_run() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("d");
    fs::create_dir(&d).unwrap();
    for i in 0..200 {
        fs::write(d.join(format!("{i}.txt")), "x".repeat(50_000)).unwrap();
    }
    let out = t.path().join("a.7z");
    let cancel = AtomicBool::new(false);
    let n = AtomicU64::new(0);
    let on = |tk: Tick<'_>| {
        if matches!(tk, Tick::Done) && n.fetch_add(1, Ordering::SeqCst) == 20 {
            cancel.store(true, Ordering::SeqCst);
        }
    };
    let o = PackOpts { threads: 4, ..PackOpts::new(Format::SevenZ, Level::Max) };
    let r = pack_with(&[d], &out, &o, &Ctl { cancel: &cancel, on: &on });
    assert!(matches!(r, Err(Error::Canceled)), "{r:?}");
    assert!(!out.exists() && !part_path(&out).exists());
}

/// ZIP64 cases (several GB on disk, minutes): `cargo test --release -p rolid-archive -- --ignored zip64`.
fn zip64_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/zip64-test").join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn zip64_check(out: &Path, items: usize) {
    external_ok("unzip", &["-tqq"], out);
    external_ok("7z", &["t", "-bso0", "-bsp0"], out);
    let tr = ctl_run(|c| test(out, Format::Zip, &ReadOpts { max_entries: 0, ..ReadOpts::new() }, c)).unwrap();
    assert!(tr.bad.is_empty(), "{:?}", tr.bad);
    let l = list(out, Format::Zip, &ReadOpts { max_entries: 0, ..ReadOpts::new() }).unwrap();
    assert_eq!(l.items.len(), items);
}

#[test]
#[ignore]
fn zip64_file_over_4_gib() {
    let d = zip64_dir("file");
    let src = d.join("src");
    fs::create_dir(&src).unwrap();
    // Sparse: 4.5 GiB of zeros (deflated to a small archive).
    let f = fs::File::create(src.join("image.img")).unwrap();
    f.set_len(4_500 << 20).unwrap();
    fs::write(src.join("after.txt"), "after the big file").unwrap();
    let out = d.join("a.zip");
    let o = PackOpts::new(Format::Zip, Level::Fast);
    ctl_run(|c| pack_with(std::slice::from_ref(&src), &out, &o, c)).unwrap();
    zip64_check(&out, 3);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
#[ignore]
fn zip64_archive_over_4_gib() {
    let d = zip64_dir("archive");
    let src = d.join("src");
    fs::create_dir(&src).unwrap();
    // Stored (by extension), so the archive itself passes 4 GiB; small files
    // after it get offsets above 4 GiB (written by the workers).
    let f = fs::File::create(src.join("a-film.mkv")).unwrap();
    f.set_len(4_200 << 20).unwrap();
    for i in 0..50 {
        fs::write(src.join(format!("b{i:02}.txt")), format!("file {i}\n").repeat(100)).unwrap();
    }
    let out = d.join("a.zip");
    let o = PackOpts::new(Format::Zip, Level::Normal);
    ctl_run(|c| pack_with(std::slice::from_ref(&src), &out, &o, c)).unwrap();
    assert!(fs::metadata(&out).unwrap().len() > 4 << 30);
    zip64_check(&out, 52);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
#[ignore]
fn zip64_over_65535_entries() {
    let d = zip64_dir("entries");
    let src = d.join("src");
    for i in 0..70 {
        let sub = src.join(format!("s{i:02}"));
        fs::create_dir_all(&sub).unwrap();
        for j in 0..1000 {
            fs::write(sub.join(format!("{j}.txt")), format!("{i}/{j}")).unwrap();
        }
    }
    let out = d.join("a.zip");
    let o = PackOpts::new(Format::Zip, Level::Fast);
    ctl_run(|c| pack_with(std::slice::from_ref(&src), &out, &o, c)).unwrap();
    zip64_check(&out, 1 + 70 + 70_000);
    fs::remove_dir_all(&d).unwrap();
}

/// A 255-byte name gets a part file that the file system accepts (roadmap 0.8.15: odd names).
#[test]
fn the_part_file_of_a_long_name_fits() {
    let d = std::path::Path::new("/d");
    assert_eq!(part_path(&d.join("a.txt")), d.join("a.txt.filepart"));
    for name in ["x".repeat(255), "日".repeat(85)] {
        let p = part_path(&d.join(&name));
        let file = p.file_name().unwrap().to_str().unwrap();
        assert!(file.len() <= 255 && file.ends_with(".filepart"), "{file}");
        assert_eq!(p.parent(), Some(d));
        assert_ne!(p, part_path(&d.join(format!("{name}z"))));
    }
}
