//! A server whose shell does not answer (a login script waiting for input, a
//! stuck profile): the short check commands end after `PROBE_TIMEOUT` and the
//! work is done over SFTP instead. Against a real OpenSSH server.

mod sshd;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rolid_ssh::{SshConnection, PROBE_TIMEOUT};

/// Every command hangs; only SFTP works.
fn hanging_shell(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("force.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$SSH_ORIGINAL_COMMAND\" in\n  internal-sftp|*sftp-server*) exec /usr/lib/openssh/sftp-server ;;\nesac\nsleep 60\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hanging_shell_falls_back_to_sftp() {
    if !std::path::Path::new("/usr/lib/openssh/sftp-server").exists() {
        eprintln!("SKIP: /usr/lib/openssh/sftp-server not available");
        return;
    }
    let base = std::env::temp_dir().join(format!("rolid-probe-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let script = hanging_shell(&base);
    let Some(server) = sshd::Sshd::start_with("probe", &format!("ForceCommand {}", script.display())) else { return };
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    let no = AtomicBool::new(false);

    // The check itself: an error after the limit, not a wait for the command.
    let t = Instant::now();
    let r = conn.exec_probe("find / -maxdepth 0 -printf F", &no, &mut |_| true).await;
    let took = t.elapsed();
    assert!(r.is_err(), "{r:?}");
    assert!(took >= PROBE_TIMEOUT && took < PROBE_TIMEOUT + Duration::from_secs(3), "{took:?}");

    // A search: find/grep on the server cannot be checked, so it walks the
    // folders over SFTP and still finds the file.
    let root = base.join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/needle.txt"), "x").unwrap();
    let fs = conn.sftp().await.unwrap();
    let q = rolid_search::Query { root: root.display().to_string(), name: "needle*".into(), ..Default::default() };
    let hits = Mutex::new(Vec::new());
    let emit = |h: rolid_search::Hit| hits.lock().unwrap().push(h.rel);
    let sink = rolid_search::Sink::new(&emit, 100, &no);
    let t = Instant::now();
    let out = rolid_search::remote::search_remote(&conn, &fs, &q, &sink).await.unwrap();
    assert!(t.elapsed() < PROBE_TIMEOUT + Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(out.method, rolid_search::Method::Walk);
    assert_eq!(hits.lock().unwrap().len(), 1, "{:?}", hits.lock().unwrap());
    let _ = std::fs::remove_dir_all(&base);
}
