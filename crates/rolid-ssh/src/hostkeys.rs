//! Known server keys: the program's own list and `~/.ssh/known_hosts`.
//!
//! Since 0.9.1 the program reads two files: its own list (`known_hosts` in
//! its configuration folder), where the keys you accept are written, and
//! OpenSSH's `~/.ssh/known_hosts`, which it only reads. When the two files
//! disagree about a server (one knows the key the server sent, the other has
//! a different key of the same type), the connection is refused: one of them
//! is wrong, and the user has to find out which.
//!
//! The reader follows the format of sshd(8) "SSH_KNOWN_HOSTS FILE FORMAT":
//! comment and empty lines, fields separated by spaces or tabs, the markers
//! `@revoked` (the key is refused for every host it matches) and
//! `@cert-authority` (host certificates, not supported: skipped), host
//! patterns separated by commas with `*`, `?` and `!` (negation), hashed
//! names (`|1|salt|hash`), and `[host]:port` for ports other than 22. Lines
//! with a key type the program does not know are skipped, not an error, so one
//! unusual line does not make the whole file unusable.

use std::io::Write;
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::{Algorithm, PublicKey};

/// What one file says about the key a server sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The file has this key for the server.
    Match { line: usize },
    /// The file has a different key of the same type for the server (and not this one).
    Changed { line: usize },
    /// The key is marked `@revoked`.
    Revoked { line: usize },
    /// Nothing about this key type for the server.
    Unknown,
}

/// The decision over all files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Known,
    Unknown,
    Changed { file: PathBuf, line: usize },
    Revoked { file: PathBuf, line: usize },
    /// One file knows the key, the other has a different one.
    Disagree { knows: (PathBuf, usize), other: (PathBuf, usize) },
}

/// The name a host has in a known_hosts file (`host` or `[host]:port`).
pub fn host_port(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// The key type without the signature hash (`ssh-rsa` for all RSA keys).
fn kind(k: &PublicKey) -> Algorithm {
    k.key_data().algorithm()
}

/// A wildcard pattern of OpenSSH (`*` any run, `?` one character), case-insensitive.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p].eq_ignore_ascii_case(&text[t])) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

fn hashed(entry: &str, name: &str) -> bool {
    use hmac::{Hmac, KeyInit, Mac};
    let mut parts = entry.split('|').skip(2);
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (data_encoding::BASE64.decode(salt.as_bytes()), data_encoding::BASE64.decode(hash.as_bytes())) else {
        return false;
    };
    let Ok(mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.chain_update(name.as_bytes()).verify_slice(&hash).is_ok()
}

/// The host patterns of a line match `name` (a negated pattern that matches wins).
fn hosts_match(patterns: &str, name: &str) -> bool {
    let mut yes = false;
    for p in patterns.split(',') {
        if let Some(neg) = p.strip_prefix('!') {
            if wildcard(neg.as_bytes(), name.as_bytes()) {
                return false;
            }
        } else if p.starts_with("|1|") {
            yes |= hashed(p, name);
        } else {
            yes |= wildcard(p.as_bytes(), name.as_bytes());
        }
    }
    yes
}

/// One line: (revoked, host patterns, key), or `None` for a line to skip.
fn parse_line(line: &str) -> Option<(bool, &str, PublicKey)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut f = line.split_ascii_whitespace();
    let mut first = f.next()?;
    let mut revoked = false;
    if let Some(marker) = first.strip_prefix('@') {
        match marker {
            "revoked" => revoked = true,
            _ => return None, // @cert-authority and unknown markers
        }
        first = f.next()?;
    }
    let (kt, b64) = (f.next()?, f.next()?);
    let key = PublicKey::from_openssh(&format!("{kt} {b64}")).ok()?;
    Some((revoked, first, key))
}

/// What the text of a known_hosts file says about `key` of server `name`
/// (see [`host_port`]).
pub fn check_text(text: &str, name: &str, key: &PublicKey) -> Found {
    let mut matched = None;
    let mut changed = None;
    for (i, line) in text.lines().enumerate() {
        let Some((revoked, hosts, k)) = parse_line(line) else { continue };
        if revoked {
            // A revoked key is refused whatever host it is offered by, as OpenSSH does.
            if k.key_data() == key.key_data() {
                return Found::Revoked { line: i + 1 };
            }
            continue;
        }
        if !hosts_match(hosts, name) {
            continue;
        }
        if k.key_data() == key.key_data() {
            matched.get_or_insert(i + 1);
        } else if kind(&k) == kind(key) {
            changed.get_or_insert(i + 1);
        }
    }
    match (matched, changed) {
        (Some(line), _) => Found::Match { line },
        (None, Some(line)) => Found::Changed { line },
        (None, None) => Found::Unknown,
    }
}

/// The key types a file has for server `name`, in the order of the file.
pub fn known_types(text: &str, name: &str) -> Vec<Algorithm> {
    let mut v: Vec<Algorithm> = Vec::new();
    for line in text.lines() {
        if let Some((false, hosts, k)) = parse_line(line) {
            if hosts_match(hosts, name) && !v.contains(&kind(&k)) {
                v.push(kind(&k));
            }
        }
    }
    v
}

/// The host key types to ask the server for: the types already known for it
/// first (as OpenSSH does), so a server with a known RSA or ECDSA key and a new
/// Ed25519 key is not taken for an unknown one.
pub fn preferred(known: &[Algorithm]) -> Vec<Algorithm> {
    let norm = |a: &Algorithm| match a {
        Algorithm::Rsa { .. } => Algorithm::Rsa { hash: None },
        a => a.clone(),
    };
    let all = russh::Preferred::DEFAULT.key;
    let (mut first, rest): (Vec<Algorithm>, Vec<Algorithm>) = all.iter().cloned().partition(|a| known.contains(&norm(a)));
    first.extend(rest);
    first
}

/// Read a known_hosts file; a missing file is empty.
pub fn read(path: &Path) -> std::io::Result<String> {
    match std::fs::read(path) {
        Ok(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// The decision over the files, in order (the program's own list first).
pub fn decide(found: &[(PathBuf, Found)]) -> Verdict {
    if let Some((f, Found::Revoked { line })) = found.iter().find(|(_, x)| matches!(x, Found::Revoked { .. })) {
        return Verdict::Revoked { file: f.clone(), line: *line };
    }
    let knows = found.iter().find_map(|(f, x)| match x {
        Found::Match { line } => Some((f.clone(), *line)),
        _ => None,
    });
    let other = found.iter().find_map(|(f, x)| match x {
        Found::Changed { line } => Some((f.clone(), *line)),
        _ => None,
    });
    match (knows, other) {
        (Some(knows), Some(other)) => Verdict::Disagree { knows, other },
        (Some(_), None) => Verdict::Known,
        (None, Some((file, line))) => Verdict::Changed { file, line },
        (None, None) => Verdict::Unknown,
    }
}

/// Add the key of a server to the program's own list (never to `~/.ssh/known_hosts`).
pub fn learn(path: &Path, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    use std::os::unix::fs::FileExt;
    let line = key.to_openssh().map_err(std::io::Error::other)?;
    let mut f = std::fs::OpenOptions::new().read(true).create(true).append(true).mode(0o600).open(path)?;
    // Another running copy may be adding its line now: the end of the file
    // is looked at and written under one lock.
    f.lock()?;
    let len = f.metadata()?.len();
    let mut last = *b"\n";
    if len > 0 {
        f.read_exact_at(&mut last, len - 1)?;
    }
    let mut text = String::new();
    if last[0] != b'\n' {
        text.push('\n');
    }
    text.push_str(&format!("{} {line}\n", host_port(host, port)));
    f.write_all(text.as_bytes())?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAvS+okP3RK3Lywolf/o8y55rt0wUOJ4y8A0rPOMMQPa";
    const ED_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIC3LTwq3N7B8YMooXhow99gBCYWnzbx/ajG9Q+1XKafk";
    const EC: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBII+p2mmWzu5OtlkCQbQ2hqnFPkn+eRBClIs7q9yR5iI/JE9diMOPjyLwp9F5kuqfyqX5sJUKtnglScHYM3Y0R8=";

    fn key(s: &str) -> PublicKey {
        PublicKey::from_openssh(s).unwrap()
    }

    #[test]
    fn a_plain_line_matches_its_host_and_port() {
        let t = format!("# comment\n\nweb.example {ED_A}\n[web.example]:2222 {ED_B}\n");
        assert_eq!(check_text(&t, &host_port("web.example", 22), &key(ED_A)), Found::Match { line: 3 });
        assert_eq!(check_text(&t, &host_port("WEB.example", 22), &key(ED_B)), Found::Changed { line: 3 });
        assert_eq!(check_text(&t, &host_port("web.example", 2222), &key(ED_B)), Found::Match { line: 4 });
        assert_eq!(check_text(&t, &host_port("other", 22), &key(ED_A)), Found::Unknown);
        // A key of another type is not a change.
        assert_eq!(check_text(&t, &host_port("web.example", 22), &key(EC)), Found::Unknown);
    }

    #[test]
    fn a_second_key_of_the_same_type_is_not_a_change_when_one_matches() {
        let t = format!("web {ED_B}\nweb {ED_A}\n");
        assert_eq!(check_text(&t, "web", &key(ED_A)), Found::Match { line: 2 });
    }

    #[test]
    fn tabs_several_names_wildcards_and_negation() {
        let t = format!("a.example,web\t{ED_A}\n*.lan,!secret.lan {ED_B}\n[db?.lan]:2200 {EC}\n");
        assert!(matches!(check_text(&t, "web", &key(ED_A)), Found::Match { .. }));
        assert!(matches!(check_text(&t, "nas.lan", &key(ED_B)), Found::Match { .. }));
        assert_eq!(check_text(&t, "secret.lan", &key(ED_B)), Found::Unknown);
        assert!(matches!(check_text(&t, &host_port("db1.lan", 2200), &key(EC)), Found::Match { .. }));
        assert_eq!(check_text(&t, &host_port("db12.lan", 2200), &key(EC)), Found::Unknown);
    }

    #[test]
    fn a_hashed_name_matches() {
        // `ssh-keygen -H` of "web.example" with this salt.
        use hmac::{Hmac, KeyInit, Mac};
        let salt = [7u8; 20];
        let mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap().chain_update(b"web.example").finalize().into_bytes();
        let b = data_encoding::BASE64;
        let t = format!("|1|{}|{} {ED_A}\n", b.encode(&salt), b.encode(&mac));
        assert!(matches!(check_text(&t, "web.example", &key(ED_A)), Found::Match { .. }));
        assert_eq!(check_text(&t, "web2.example", &key(ED_A)), Found::Unknown);
    }

    #[test]
    fn a_revoked_key_is_refused_for_any_host() {
        let t = format!("web {ED_A}\n@revoked * {ED_A}\n");
        assert_eq!(check_text(&t, "web", &key(ED_A)), Found::Revoked { line: 2 });
        assert_eq!(check_text(&t, "web", &key(ED_B)), Found::Changed { line: 1 });
    }

    #[test]
    fn unusual_lines_are_skipped() {
        let t = format!(
            "@cert-authority *.example {ED_B}\nweb sk-ssh-ed25519@openssh.com AAAAbroken\nweb\nweb ssh-ed25519 !!!\nweb {ED_A} comment here\n"
        );
        assert_eq!(check_text(&t, "web", &key(ED_A)), Found::Match { line: 5 });
        assert_eq!(check_text(&t, "x.example", &key(ED_A)), Found::Unknown);
    }

    #[test]
    fn the_known_types_come_in_the_order_of_the_file() {
        let t = format!("web {EC}\nweb {ED_A}\nweb {ED_B}\nother {ED_A}\n");
        let v = known_types(&t, "web");
        assert_eq!(v, vec![kind(&key(EC)), kind(&key(ED_A))]);
    }

    #[test]
    fn known_types_are_asked_for_first() {
        let p = preferred(&[kind(&key(EC)), Algorithm::Rsa { hash: None }]);
        assert_eq!(p.len(), russh::Preferred::DEFAULT.key.len());
        assert_eq!(p[0], kind(&key(EC)));
        assert!(matches!(p[1], Algorithm::Rsa { hash: Some(_) }));
        assert!(matches!(p[2], Algorithm::Rsa { hash: Some(_) }));
        assert_eq!(p[3], Algorithm::Rsa { hash: None });
        assert_eq!(p[4], Algorithm::Ed25519);
        assert_eq!(preferred(&[]), russh::Preferred::DEFAULT.key.to_vec());
    }

    #[test]
    fn the_files_decide_together() {
        let own = PathBuf::from("own");
        let ssh = PathBuf::from("ssh");
        let d = |a: Found, b: Found| decide(&[(own.clone(), a), (ssh.clone(), b)]);
        use Found::*;
        assert_eq!(d(Match { line: 1 }, Unknown), Verdict::Known);
        assert_eq!(d(Unknown, Match { line: 4 }), Verdict::Known);
        assert_eq!(d(Match { line: 1 }, Match { line: 2 }), Verdict::Known);
        assert_eq!(d(Unknown, Unknown), Verdict::Unknown);
        assert_eq!(d(Unknown, Changed { line: 3 }), Verdict::Changed { file: ssh.clone(), line: 3 });
        assert_eq!(d(Changed { line: 2 }, Changed { line: 3 }), Verdict::Changed { file: own.clone(), line: 2 });
        assert_eq!(
            d(Match { line: 1 }, Changed { line: 9 }),
            Verdict::Disagree { knows: (own.clone(), 1), other: (ssh.clone(), 9) }
        );
        assert_eq!(
            d(Changed { line: 2 }, Match { line: 9 }),
            Verdict::Disagree { knows: (ssh.clone(), 9), other: (own.clone(), 2) }
        );
        assert_eq!(d(Match { line: 1 }, Revoked { line: 5 }), Verdict::Revoked { file: ssh.clone(), line: 5 });
    }

    #[test]
    fn learning_writes_only_the_given_file_and_keeps_what_is_there() {
        let dir = std::env::temp_dir().join(format!("rolid-hostkeys-{}", std::process::id()));
        let p = dir.join("sub").join("known_hosts");
        learn(&p, "Web", 22, &key(ED_A)).unwrap();
        std::fs::write(&p, format!("{}no-newline {ED_B}", read(&p).unwrap())).unwrap();
        learn(&p, "db", 2222, &key(EC)).unwrap();
        let t = read(&p).unwrap();
        assert!(matches!(check_text(&t, "web", &key(ED_A)), Found::Match { line: 1 }));
        assert!(matches!(check_text(&t, "no-newline", &key(ED_B)), Found::Match { line: 2 }));
        assert!(matches!(check_text(&t, &host_port("db", 2222), &key(EC)), Found::Match { line: 3 }));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two running copies trusting keys at the same moment: every line is
    /// kept whole (one append each).
    #[test]
    fn keys_learned_at_once_by_several_copies_are_all_kept() {
        let dir = std::env::temp_dir().join(format!("rolid-hostkeys-many-{}", std::process::id()));
        let p = dir.join("known_hosts");
        std::thread::scope(|s| {
            for t in 0..8 {
                let p = &p;
                s.spawn(move || {
                    for i in 0..25 {
                        let k = if (t + i) % 2 == 0 { ED_A } else { EC };
                        learn(p, &format!("h{t}-{i}"), 22, &key(k)).unwrap();
                    }
                });
            }
        });
        let text = read(&p).unwrap();
        assert_eq!(text.lines().count(), 200);
        for t in 0..8 {
            for i in 0..25 {
                let k = if (t + i) % 2 == 0 { ED_A } else { EC };
                assert!(matches!(check_text(&text, &format!("h{t}-{i}"), &key(k)), Found::Match { .. }), "h{t}-{i}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
