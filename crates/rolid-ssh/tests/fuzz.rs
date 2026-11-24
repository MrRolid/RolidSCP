//! Fuzzing of reading `known_hosts` files (the program's own and
//! `~/.ssh/known_hosts`): plain and hashed names, ports, markers, comments,
//! damaged keys. A damaged file must give "unknown", "changed" or an error,
//! never a panic.
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

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
    let file = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fuzz_known_hosts");
    let check = |b: &[u8]| {
        std::fs::write(&file, b).unwrap();
        for (host, port) in [("example.com", 22), ("example.com", 2222), ("192.0.2.1", 22), ("a.example.net", 22)] {
            let _ = keys::check_known_hosts_path(host, port, &key, &file);
        }
    };
    if mutate::replay(check) {
        return;
    }
    mutate::run("known-hosts", &seeds, dict, check);
}
