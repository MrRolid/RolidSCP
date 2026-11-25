//! The server's clock: read with `date` over SSH, against a real OpenSSH
//! server whose `date` is replaced by a script (an hour ahead, garbage,
//! no shell at all).

mod sshd;

use rolid_ssh::{clock, SshConnection};

/// A server whose commands go through `script` (SFTP still works).
fn forced(dir: &std::path::Path, name: &str, date_answer: &str) -> std::path::PathBuf {
    let script = dir.join(format!("{name}.sh"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$SSH_ORIGINAL_COMMAND\" in\n  internal-sftp|*sftp-server*) exec /usr/lib/openssh/sftp-server ;;\n  'date -u +%s') {date_answer} ;;\n  *) exit 127 ;;\nesac\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn base(name: &str) -> std::path::PathBuf {
    let b = std::env::temp_dir().join(format!("rolid-clock-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&b).unwrap();
    b
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_with_the_same_clock_is_not_off() {
    let Some(server) = sshd::Sshd::start("clock-same") else { return };
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    let o = clock::measure(&conn).await.expect("a reading");
    assert!(o.secs.abs() <= 2 && !o.is_off(), "{o:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_an_hour_ahead_is_off() {
    if !std::path::Path::new("/usr/lib/openssh/sftp-server").exists() {
        eprintln!("SKIP: /usr/lib/openssh/sftp-server not available");
        return;
    }
    let b = base("ahead");
    let script = forced(&b, "ahead", "echo $(( $(date -u +%s) + 3600 ))");
    let Some(server) = sshd::Sshd::start_with("clock-ahead", &format!("ForceCommand {}", script.display())) else { return };
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    let o = clock::measure(&conn).await.expect("a reading");
    assert!((3598..=3602).contains(&o.secs), "{o:?}");
    assert!(o.is_off());
    let _ = std::fs::remove_dir_all(&b);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_shell_or_a_strange_answer_gives_no_reading() {
    if !std::path::Path::new("/usr/lib/openssh/sftp-server").exists() {
        eprintln!("SKIP: /usr/lib/openssh/sftp-server not available");
        return;
    }
    let b = base("bad");
    for (name, answer) in [
        ("words", "echo 'Thu Nov 26 10:00:00 UTC 2026'"),
        ("fails", "echo 1700000000; exit 1"),
        ("noshell", "exit 127"),
    ] {
        let script = forced(&b, name, answer);
        let Some(server) = sshd::Sshd::start_with(&format!("clock-{name}"), &format!("ForceCommand {}", script.display())) else { return };
        let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
        assert_eq!(clock::measure(&conn).await, None, "{name}");
    }
    let _ = std::fs::remove_dir_all(&b);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_lost_before_the_question_gives_no_reading() {
    let Some(server) = sshd::Sshd::start("clock-lost") else { return };
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    conn.abandon();
    let t = std::time::Instant::now();
    assert_eq!(clock::measure(&conn).await, None);
    assert!(t.elapsed() < rolid_ssh::PROBE_TIMEOUT + std::time::Duration::from_secs(2));
}
