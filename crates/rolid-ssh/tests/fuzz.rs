//! Fuzzing of reading `known_hosts` files (the program's own and
//! `~/.ssh/known_hosts`): plain and hashed names, ports, markers, comments,
//! damaged keys. A damaged file must give "unknown", "changed" or "revoked",
//! never a panic. The reader is the program's own (`rolid_ssh::hostkeys`).
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

use rolid_ssh::hostkeys;
use russh::keys;

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const OTHER: &str = "AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XFSqti";

#[test]
fn fuzz_known_hosts_files() {
    let key = keys::parse_public_key_base64(KEY).unwrap();
    let seeds: Vec<Vec<u8>> = vec![
        format!("example.com ssh-ed25519 {KEY}\n[example.com]:2222 ssh-ed25519 {OTHER} comment\n").into_bytes(),
        format!("# comment\n\n|1|F1E1KeoE/eEWhi10WpGv4OdiO6Y=|3988QV0VE8wmZL7suNrYQLITLCg= ssh-ed25519 {KEY}\n").into_bytes(),
        format!("example.org,192.0.2.1 ssh-ed25519 {OTHER}\n@cert-authority *.example.net ssh-ed25519 {KEY}\n@revoked example.com ssh-ed25519 {OTHER}\n").into_bytes(),
        format!("example.com,[example.com]:22 ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ\nexample.com ssh-ed25519 {KEY}\n").into_bytes(),
    ];
    let dict: &[&[u8]] = &[b"|1|", b"[", b"]:", b",", b"@cert-authority ", b"@revoked ", b"ssh-ed25519 ", b"ecdsa-sha2-nistp256 ", b"ssh-rsa ", b"*", b"?", b"!", b"=", b"\n"];
    let check = |b: &[u8]| {
        let text = String::from_utf8_lossy(b);
        for (host, port) in [("example.com", 22), ("example.com", 2222), ("192.0.2.1", 22), ("a.example.net", 22)] {
            let name = hostkeys::host_port(host, port);
            let _ = hostkeys::check_text(&text, &name, &key);
            let _ = hostkeys::preferred(&hostkeys::known_types(&text, &name));
        }
    };
    if mutate::replay(check) {
        return;
    }
    mutate::run("known-hosts", &seeds, dict, check);
}
