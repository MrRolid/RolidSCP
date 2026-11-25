//! Server keys checked against two lists: the program's own (written) and
//! `~/.ssh/known_hosts` (only read). Against an in-process russh server.
mod common;

use std::path::{Path, PathBuf};

use rolid_ssh::{ConnectParams, SshConnection, SshError};

const PW: &str = "pw";
/// A key that is not the server's.
const OTHER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIC3LTwq3N7B8YMooXhow99gBCYWnzbx/ajG9Q+1XKafk";

fn server_line(port: u16) -> String {
    let k = common::host_key().public_key().to_openssh().unwrap();
    format!("[127.0.0.1]:{port} {k}\n")
}

fn params(port: u16, own: &Path, ssh: &Path, accept: Option<String>) -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        password: Some(PW.into()),
        accept_fingerprint: accept,
        known_hosts: Some(own.to_path_buf()),
        known_hosts_read: vec![ssh.to_path_buf()],
        skip_agent: true,
        skip_default_keys: true,
        ..Default::default()
    }
}

async fn case(name: &str, own: Option<&str>, ssh: Option<&str>, accept: bool) -> (Result<(), SshError>, PathBuf, PathBuf) {
    let dir = common::tmpdir(&format!("kh-{name}"));
    let srv = common::start(common::ServerSpec::password_only(PW), &dir).await;
    let (own_p, ssh_p) = (dir.join("own_known_hosts"), dir.join("ssh_known_hosts"));
    let fill = |t: &str| t.replace("SERVER\n", &server_line(srv.port)).replace("OTHER", &format!("[127.0.0.1]:{} {OTHER}", srv.port));
    if let Some(t) = own {
        std::fs::write(&own_p, fill(t)).unwrap();
    }
    if let Some(t) = ssh {
        std::fs::write(&ssh_p, fill(t)).unwrap();
    }
    let accept = accept.then(|| srv.fingerprint.clone());
    let r = SshConnection::connect(&params(srv.port, &own_p, &ssh_p, accept)).await.map(|_| ());
    (r, own_p, ssh_p)
}

#[tokio::test]
async fn a_key_in_known_hosts_only_is_enough_and_nothing_is_written() {
    let (r, own, ssh) = case("ssh-only", None, Some("SERVER\n"), false).await;
    r.expect("connect");
    assert!(!own.exists());
    assert_eq!(std::fs::read_to_string(ssh).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn a_key_in_the_own_list_only_is_enough() {
    let (r, _, ssh) = case("own-only", Some("SERVER\n"), None, false).await;
    r.expect("connect");
    assert!(!ssh.exists());
}

#[tokio::test]
async fn an_accepted_key_goes_only_to_the_own_list() {
    let (r, own, ssh) = case("accept", None, Some("# mine\n"), true).await;
    r.expect("connect");
    assert!(std::fs::read_to_string(&own).unwrap().contains("ssh-ed25519 "));
    assert_eq!(std::fs::read_to_string(&ssh).unwrap(), "# mine\n");
}

#[tokio::test]
async fn an_unknown_server_asks() {
    let (r, own, _) = case("unknown", None, None, false).await;
    assert!(matches!(r, Err(SshError::UnknownHostKey { .. })), "{r:?}");
    assert!(!own.exists());
}

#[tokio::test]
async fn a_changed_key_in_known_hosts_is_refused() {
    let (r, own, _) = case("changed", None, Some("# x\nOTHER\n"), true).await;
    match r {
        Err(SshError::HostKeyChanged { file, line, .. }) => {
            assert!(file.ends_with("ssh_known_hosts"));
            assert_eq!(line, 2);
        }
        r => panic!("{r:?}"),
    }
    // Even with the fingerprint accepted, a changed key is never written anywhere.
    assert!(!own.exists());
}

#[tokio::test]
async fn lists_that_disagree_are_refused() {
    let (r, _, _) = case("disagree", Some("SERVER\n"), Some("OTHER\n"), false).await;
    match r {
        Err(e @ SshError::HostKeysDisagree { .. }) => {
            let SshError::HostKeysDisagree { knows, other, .. } = &e else { unreachable!() };
            assert!(knows.0.ends_with("own_known_hosts") && knows.1 == 1);
            assert!(other.0.ends_with("ssh_known_hosts") && other.1 == 1);
            assert!(e.to_string().contains("disagree"));
        }
        r => panic!("{r:?}"),
    }
    let (r, _, _) = case("disagree2", Some("OTHER\n"), Some("SERVER\n"), false).await;
    assert!(matches!(r, Err(SshError::HostKeysDisagree { .. })), "{r:?}");
}

#[tokio::test]
async fn a_revoked_key_is_refused() {
    let k = common::host_key().public_key().to_openssh().unwrap();
    let (r, _, _) = case("revoked", Some("SERVER\n"), Some(&format!("@revoked * {k}\n")), false).await;
    assert!(matches!(r, Err(SshError::HostKeyRevoked { line: 1, .. })), "{r:?}");
}

#[tokio::test]
async fn an_unreadable_list_refuses() {
    let dir = common::tmpdir("kh-unreadable");
    let srv = common::start(common::ServerSpec::password_only(PW), &dir).await;
    // A folder in place of the file cannot be read.
    let ssh = dir.join("ssh_known_hosts");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(dir.join("own"), server_line(srv.port)).unwrap();
    let r = SshConnection::connect(&params(srv.port, &dir.join("own"), &ssh, None)).await;
    match r {
        Err(SshError::Other(t)) => assert!(t.to_string().contains("cannot read the list of known servers"), "{t}"),
        r => panic!("{:?}", r.map(|_| ())),
    }
}

#[path = "sshd/mod.rs"]
mod sshd;

/// A real OpenSSH server with an Ed25519 and an ECDSA key: when only the ECDSA
/// key is known, it is asked for, and the server is not taken for an unknown one.
#[tokio::test]
async fn the_known_key_type_is_asked_for_first() {
    let keys = common::tmpdir("kh-ecdsa");
    let ec = keys.join("host_ecdsa");
    let ok = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ecdsa", "-b", "256", "-N", "", "-f"])
        .arg(&ec)
        .stdout(std::process::Stdio::null())
        .status();
    if !ok.is_ok_and(|s| s.success()) {
        eprintln!("SKIP: ssh-keygen not available");
        return;
    }
    let Some(srv) = sshd::Sshd::start_with("kh-ecdsa", &format!("HostKey {}", ec.display())) else { return };
    let ec_pub = std::fs::read_to_string(ec.with_extension("pub")).unwrap();
    let ec_pub = ec_pub.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    let own = srv.data_dir("lists").join("own");
    let none = srv.data_dir("lists").join("none");
    let p = |own: &Path| ConnectParams { known_hosts: Some(own.to_path_buf()), known_hosts_read: vec![none.clone()], ..srv.params(srv.port) };

    // Nothing known: the server offers its preferred key, Ed25519.
    match SshConnection::connect(&p(&own)).await.map(|_| ()) {
        Err(SshError::UnknownHostKey { algorithm, .. }) => assert_eq!(algorithm, "ssh-ed25519"),
        r => panic!("{r:?}"),
    }
    // Only the ECDSA key known (in the list that is only read): connects.
    std::fs::write(&none, format!("[127.0.0.1]:{} {ec_pub}\n", srv.port)).unwrap();
    SshConnection::connect(&p(&own)).await.expect("connect with the known ECDSA key");
    assert!(!own.exists(), "nothing is written when the key is known");
}
