//! What a copy keeps of a file besides its content, locally and to and from a
//! real OpenSSH server (described in the help, "What a copy keeps"):
//!
//! - kept: the permission bits rwx (setuid, setgid and sticky are left out)
//!   and the modification time;
//! - not kept: the owner and group (the copy belongs to whoever writes it),
//!   ACLs, extended attributes (`user.*`, also SELinux labels and
//!   capabilities), hard links;
//! - an existing file that is overwritten is replaced by a new file (the copy
//!   is written next to it and renamed over it), so its own ACLs, extended
//!   attributes and owner are gone too, and the new file gets what the
//!   folder gives new files (a default ACL of the folder).
//!
//! SFTP (version 3, which OpenSSH speaks) has no way to carry ACLs or
//! extended attributes at all. ACLs are set with `setfacl`, extended
//! attributes with Python's `os.setxattr`; a part whose tool or file system
//! support is missing is skipped with a note.

mod sshd;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rolid_core::copy::{Conflict, CopyJob, Progress};
use rolid_core::Vfs;
use rolid_local::LocalFs;
use rolid_ssh::SshConnection;

fn setfacl(path: &Path, spec: &str) -> bool {
    Command::new("setfacl").arg("-m").arg(spec).arg(path).status().is_ok_and(|s| s.success())
}

/// The ACL entries beyond the permission bits (`getfacl --skip-base`).
fn acl(path: &Path) -> String {
    let out = Command::new("getfacl").args(["--omit-header", "--skip-base", "--absolute-names"]).arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn py(code: &str, path: &Path) -> Option<String> {
    let out = Command::new("python3").args(["-I", "-c", code]).arg(path).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn set_xattr(path: &Path) -> bool {
    py("import os,sys; os.setxattr(sys.argv[1], 'user.rolid', b'tag')", path).is_some()
}

fn xattrs(path: &Path) -> String {
    // The user namespace only (an ACL is stored as system.posix_acl_access).
    py("import os,sys; print(' '.join(sorted(a for a in os.listxattr(sys.argv[1]) if a.startswith('user.'))))", path).unwrap_or_default()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A source file with mode 4750, a time in the past, an ACL entry and an
/// extended attribute (where they can be set). Returns (acl set, xattr set).
fn source(path: &Path) -> (bool, bool) {
    std::fs::write(path, "content\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o4750)).unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    let uid = rustix_uid();
    let has_acl = setfacl(path, &format!("u:{uid}:r"));
    let has_x = set_xattr(path);
    (has_acl, has_x)
}

fn rustix_uid() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

async fn copy(src_fs: &dyn Vfs, dst_fs: &dyn Vfs, dir: &Path, name: &str, dst: &Path) {
    let cb = |_: &Progress| {};
    let mut job = CopyJob::new(src_fs, dst_fs, Arc::new(AtomicBool::new(false)), &cb).with_conflict(Conflict::Overwrite);
    job.copy_items_retrying(&[(s(dir), name.to_string())], &s(dst), 0, |_| async { false }).await.unwrap();
}

/// What the copy `got` kept of `src` (ACL and attribute checked where they
/// could be set).
fn check(way: &str, src: &Path, got: &Path, (has_acl, has_x): (bool, bool)) {
    let m = std::fs::metadata(got).unwrap();
    assert_eq!(std::fs::read_to_string(got).unwrap(), "content\n", "{way}");
    assert_eq!(m.permissions().mode() & 0o7777, 0o750, "{way}: rwx kept, setuid left out");
    assert_eq!(m.modified().unwrap(), std::fs::metadata(src).unwrap().modified().unwrap(), "{way}: time kept");
    if has_acl {
        assert!(!acl(src).is_empty(), "{way}: the source has an ACL");
        assert_eq!(acl(got), "", "{way}: no ACL on the copy");
    } else {
        eprintln!("SKIP {way}: ACLs cannot be set here");
    }
    if has_x {
        assert_eq!(xattrs(src), "user.rolid", "{way}");
        assert_eq!(xattrs(got), "", "{way}: no extended attributes on the copy");
    } else {
        eprintln!("SKIP {way}: extended attributes cannot be set here");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_copy_keeps() {
    let Some(srv) = sshd::Sshd::start("metadata") else { return };
    let conn = SshConnection::connect(&srv.params(srv.port)).await.expect("connect");
    let sftp = conn.sftp().await.unwrap();
    let (a, b, c) = (srv.data_dir("a"), srv.data_dir("b"), srv.data_dir("c"));
    let src = a.join("f.txt");
    let set = source(&src);

    // Local to local, local to the server, the server to local.
    copy(&LocalFs, &LocalFs, &a, "f.txt", &b).await;
    check("local", &src, &b.join("f.txt"), set);
    copy(&LocalFs, &sftp, &a, "f.txt", &c).await;
    check("upload", &src, &c.join("f.txt"), set);
    std::fs::remove_file(b.join("f.txt")).unwrap();
    copy(&sftp, &LocalFs, &a, "f.txt", &b).await;
    check("download", &src, &b.join("f.txt"), set);

    // Overwriting a file that has its own ACL entry and attribute: the file
    // is replaced, they are gone.
    for (way, remote) in [("overwrite local", false), ("overwrite upload", true)] {
        let target = c.join("f.txt");
        let (had_acl, had_x) = (setfacl(&target, &format!("u:{}:rw", rustix_uid())), set_xattr(&target));
        let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap());
        if remote {
            copy(&LocalFs, &sftp, &a, "f.txt", &c).await;
        } else {
            copy(&LocalFs, &LocalFs, &a, "f.txt", &c).await;
        }
        assert_ne!(std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap()), inode, "{way}: a new file");
        check(way, &src, &target, (had_acl, had_x));
    }

    // A default ACL of the target folder is what the new file gets.
    let d = srv.data_dir("d");
    if setfacl(&d, &format!("d:u:{}:rwx", rustix_uid())) {
        copy(&LocalFs, &sftp, &a, "f.txt", &d).await;
        let got = acl(&d.join("f.txt"));
        assert!(got.contains(&format!("user:{}:rwx", rustix_uid())) || got.contains(":rwx"), "inherited: {got}");
    }
}
