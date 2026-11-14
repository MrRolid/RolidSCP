//! A folder with many files over SFTP: every name exactly once, and the time
//! grows with the number of files, not with its square (the server sends ~100
//! names per answer; rebuilding the list for each answer made a folder with a
//! million files take many minutes). Against a real OpenSSH server.

mod sshd;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rolid_core::Vfs;
use rolid_ssh::SshConnection;

fn fill(dir: &std::path::Path, n: usize) {
    std::fs::create_dir_all(dir).unwrap();
    for i in 0..n {
        std::fs::File::create(dir.join(format!("f{i:07}"))).unwrap();
    }
}

async fn list_time(fs: &dyn Vfs, dir: &std::path::Path, n: usize) -> Duration {
    let t = Instant::now();
    let entries = fs.list(&dir.display().to_string()).await.unwrap();
    let took = t.elapsed();
    assert_eq!(entries.len(), n);
    let names: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), n, "a name came twice");
    assert!(!names.contains(".") && !names.contains(".."));
    took
}

#[tokio::test(flavor = "multi_thread")]
async fn a_big_folder_lists_in_linear_time() {
    let Some(server) = sshd::Sshd::start("big-listing") else { return };
    let (small, big) = (30_000, 120_000);
    let (ds, db) = (server.data_dir("small"), server.data_dir("big"));
    fill(&ds, small);
    fill(&db, big);
    // A symlink to a folder is still recognized.
    std::os::unix::fs::symlink(&ds, db.join("link-to-dir")).unwrap();
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    let fs = conn.sftp().await.unwrap();
    let entries = fs.list(&db.display().to_string()).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "link-to-dir" && e.link_to_dir));
    std::fs::remove_file(db.join("link-to-dir")).unwrap();
    // Best of two runs each (warm caches, less noise).
    let mut ts = Duration::MAX;
    let mut tb = Duration::MAX;
    for _ in 0..2 {
        ts = ts.min(list_time(&fs, &ds, small).await);
        tb = tb.min(list_time(&fs, &db, big).await);
    }
    let ratio = tb.as_secs_f64() / ts.as_secs_f64();
    eprintln!("{small} files: {ts:?}, {big} files: {tb:?}, ratio {ratio:.1} (linear: 4)");
    assert!(ratio < 6.5, "4× more files took {ratio:.1}× longer: {ts:?} → {tb:?}");
}
