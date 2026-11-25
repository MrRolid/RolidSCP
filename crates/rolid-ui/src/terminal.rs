//! Ctrl+P: a terminal in the panel's folder; on a server `ssh` into it.
//!
//! The saved password is passed by an `SSH_ASKPASS` helper, which is this
//! same program started by ssh:
//! - the application listens on a Unix socket in a private folder (0700)
//!   of `$XDG_RUNTIME_DIR`, only for a while and only for one answer;
//! - the helper sends the one-time token and ssh's question; the
//!   application answers only the password question of that user@server,
//!   once; anything else (a key passphrase, a second try) the helper asks
//!   the user on the terminal itself;
//! - ssh runs with `StrictHostKeyChecking=yes` and the known_hosts file of
//!   the application, so an unknown or changed host key ends the connection
//!   instead of asking;
//! - the password never goes into a command line, an environment variable,
//!   a file or a log (only the socket path and the token do).

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

/// Environment variables of the helper (no secrets in them).
pub const ENV_SOCKET: &str = "ROLIDSCP_ASKPASS";
pub const ENV_TOKEN: &str = "ROLIDSCP_ASKPASS_TOKEN";
/// How long the password is offered after the terminal opens.
pub const OFFER_FOR: Duration = Duration::from_secs(120);

// ------------------------------------------------------------ the terminal

/// How a terminal program takes the command to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// `-e cmd args…` (x-terminal-emulator, xterm, konsole, alacritty…).
    DashE,
    /// `-x cmd args…` (xfce4-terminal, mate-terminal, terminator).
    DashX,
    /// `-- cmd args…` (gnome-terminal, GNOME Console).
    DoubleDash,
    /// `cmd args…` (kitty, foot).
    Plain,
    /// `start -- cmd args…` (wezterm).
    WezTerm,
}

fn style(program: &str) -> Style {
    let base = Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or(program);
    match base {
        "gnome-terminal" | "kgx" | "ptyxis" => Style::DoubleDash,
        "xfce4-terminal" | "mate-terminal" | "terminator" => Style::DashX,
        "kitty" | "foot" => Style::Plain,
        "wezterm" => Style::WezTerm,
        _ => Style::DashE,
    }
}

/// Terminals tried after `$TERMINAL` and `x-terminal-emulator`.
const KNOWN: &[&str] = &[
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "mate-terminal",
    "kgx",
    "ptyxis",
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "terminator",
    "xterm",
];

/// A program by name in `$PATH` (or a path as it is).
pub fn find_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The system terminal: `$TERMINAL`, `x-terminal-emulator`, then known ones.
pub fn find_terminal() -> Option<PathBuf> {
    let env = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty());
    env.iter().map(|s| s.as_str()).chain(std::iter::once("x-terminal-emulator")).chain(KNOWN.iter().copied()).find_map(find_program)
}

/// Arguments for `terminal` to run `cmd` (empty: just a shell).
pub fn terminal_args(terminal: &str, cmd: &[String]) -> Vec<String> {
    if cmd.is_empty() {
        return Vec::new();
    }
    let mut a: Vec<String> = match style(terminal) {
        Style::DashE => vec!["-e".into()],
        Style::DashX => vec!["-x".into()],
        Style::DoubleDash => vec!["--".into()],
        Style::Plain => vec![],
        Style::WezTerm => vec!["start".into(), "--".into()],
    };
    a.extend(cmd.iter().cloned());
    a
}

/// What ssh connects to.
#[derive(Clone, Debug)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key: Option<String>,
    /// The folder to start in.
    pub path: String,
    /// The jump hosts, the nearest to the server first.
    pub jumps: Vec<SshHop>,
}

/// A jump host on the way to the server.
#[derive(Clone, Debug)]
pub struct SshHop {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key: Option<String>,
}

/// `host:port` for `ssh -W` (an IPv6 address in brackets).
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The `ProxyCommand` that reaches `dest` through `hops[0]` (which is itself
/// reached through `hops[1..]`): `ssh -W` with the same strict key check.
/// ssh runs it with `sh -c` and expands `%` in it first, so the words are
/// quoted for sh and every `%` is doubled (once more for each level).
fn proxy_command(hops: &[SshHop], dest: (&str, u16), known_hosts: &[PathBuf]) -> String {
    let j = &hops[0];
    let mut argv: Vec<String> = vec!["ssh".into(), "-p".into(), j.port.to_string(), "-l".into(), j.user.clone()];
    argv.extend(["-o".into(), "StrictHostKeyChecking=yes".into()]);
    argv.extend(["-o".into(), known_hosts_option(known_hosts)]);
    argv.extend(["-o".into(), "GlobalKnownHostsFile=/dev/null".into()]);
    if let Some(k) = &j.key {
        argv.extend(["-i".into(), k.clone()]);
    }
    if hops.len() > 1 {
        argv.extend(["-o".into(), format!("ProxyCommand={}", proxy_command(&hops[1..], (&j.host, j.port), known_hosts))]);
    }
    argv.extend(["-W".into(), host_port(dest.0, dest.1), "--".into(), j.host.clone()]);
    let line: Vec<String> = argv.iter().map(|a| rolid_search::remote::sh_quote(a)).collect();
    line.join(" ").replace('%', "%%")
}

/// OpenSSH's list of known servers: read, never written by the program.
pub fn ssh_known_hosts() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

/// The program's own list of known servers, where the keys you accept go
/// (since 0.9.1; before, they went to `~/.ssh/known_hosts`).
pub fn own_known_hosts() -> PathBuf {
    rolid_config::config_dir().join("known_hosts")
}

/// The lists the application checks servers against, its own first.
pub fn known_hosts() -> Vec<PathBuf> {
    std::iter::once(own_known_hosts()).chain(ssh_known_hosts()).collect()
}

/// `UserKnownHostsFile=…` for ssh: the files quoted, as its configuration
/// parser splits the value at spaces and expands `%` and `~`.
fn known_hosts_option(files: &[PathBuf]) -> String {
    let quoted: Vec<String> = files
        .iter()
        .map(|f| {
            let s = f.display().to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
            format!("\"{s}\"")
        })
        .collect();
    format!("UserKnownHostsFile={}", quoted.join(" "))
}

/// `ssh` with the application's host key checking, into `t.path` with a login shell.
/// `askpass`: (helper program, socket, token) when a password is offered.
pub fn ssh_command(t: &SshTarget, known_hosts: &[PathBuf], askpass: Option<(&Path, &Path, &str)>) -> Vec<String> {
    let mut c: Vec<String> = Vec::new();
    if let Some((helper, sock, token)) = askpass {
        c.push("env".into());
        c.push(format!("SSH_ASKPASS={}", helper.display()));
        c.push("SSH_ASKPASS_REQUIRE=force".into());
        c.push(format!("{ENV_SOCKET}={}", sock.display()));
        c.push(format!("{ENV_TOKEN}={token}"));
    }
    c.push("ssh".into());
    c.push("-t".into());
    c.extend(["-p".into(), t.port.to_string(), "-l".into(), t.user.clone()]);
    c.extend(["-o".into(), "StrictHostKeyChecking=yes".into()]);
    c.extend(["-o".into(), known_hosts_option(known_hosts)]);
    c.extend(["-o".into(), "GlobalKnownHostsFile=/dev/null".into()]);
    if let Some(k) = &t.key {
        c.extend(["-i".into(), k.clone()]);
    }
    if !t.jumps.is_empty() {
        c.extend(["-o".into(), format!("ProxyCommand={}", proxy_command(&t.jumps, (&t.host, t.port), known_hosts))]);
    }
    c.push("--".into());
    c.push(t.host.clone());
    // Run by the user's login shell (sh, bash, fish, csh…): only plain quoting.
    let q = rolid_search::remote::sh_quote(&t.path);
    c.push(format!("exec sh -c 'cd \"$1\" 2>/dev/null; exec \"${{SHELL:-/bin/sh}}\" -l' sh {q}"));
    c
}

#[cfg(test)]
#[path = "../../rolid-ssh/tests/sshd/mod.rs"]
mod sshd_for_tests;

// ------------------------------------------------------------ the password question

/// Is this ssh's question for the password of `user`@`host` (and nothing else)?
/// OpenSSH asks "user@host's password: " (password) or "(user@host) Password: "
/// (keyboard-interactive); it writes the host name in lower case.
pub fn is_password_prompt(prompt: &str, user: &str, host: &str) -> bool {
    let p = prompt.trim();
    let who = format!("{user}@{}", host.to_lowercase());
    if let Some(rest) = p.strip_prefix(&who) {
        return rest == "'s password:";
    }
    if let Some(rest) = p.strip_prefix(&format!("({who})")) {
        let r = rest.trim().to_lowercase();
        return r == "password:";
    }
    false
}

/// The one-time answer offered to the helper.
pub struct Offer {
    pub socket: PathBuf,
    pub token: String,
}

fn random_hex(bytes: usize) -> std::io::Result<String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// A password offered for the password question of `user`@`host`.
pub struct Answer {
    pub user: String,
    pub host: String,
    pub password: Zeroizing<String>,
}

/// Offer passwords for the password questions of their servers (the server
/// and its jump hosts): each once, for [`OFFER_FOR`]; then the socket and its
/// folder are removed. A question the program has no password for is asked
/// in the terminal by the helper itself.
pub fn offer(answers: Vec<Answer>) -> std::io::Result<Offer> {
    use std::os::unix::fs::DirBuilderExt;
    // A socket path has at most 107 bytes: a long $XDG_RUNTIME_DIR falls back
    // to the temporary folder (the private folder is 0700 there too).
    let fits = |p: &PathBuf| p.as_os_str().len() + 40 < 100;
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir() && fits(p)).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("rolidscp-askpass-{}", random_hex(8)?));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let socket = dir.join("s");
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            let _ = std::fs::remove_dir(&dir);
            return Err(e);
        }
    };
    listener.set_nonblocking(true)?;
    let token = random_hex(32)?;
    let tok = token.clone();
    std::thread::Builder::new().name("rolid-askpass".into()).spawn(move || {
        serve(&listener, &tok, answers, Instant::now() + OFFER_FOR);
        drop(listener);
        let _ = std::fs::remove_file(dir.join("s"));
        let _ = std::fs::remove_dir(&dir);
    })?;
    Ok(Offer { socket, token })
}

fn serve(listener: &UnixListener, token: &str, mut answers: Vec<Answer>, until: Instant) {
    while Instant::now() < until && !answers.is_empty() {
        match listener.accept() {
            Ok((s, _)) => {
                if let Some(i) = answer(s, token, &answers) {
                    answers.remove(i); // answered once: no more
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
}

/// One question from the helper; which answer was given.
fn answer(mut s: UnixStream, token: &str, answers: &[Answer]) -> Option<usize> {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    // Only a process of this user.
    let same_user = rustix::net::sockopt::socket_peercred(&s).is_ok_and(|c| c.uid == rustix::process::getuid());
    let mut buf = Vec::new();
    let _ = (&mut s).take(8192).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let (got_token, prompt) = text.split_once('\n').unwrap_or((&text, ""));
    let ok = same_user && constant_eq(got_token.as_bytes(), token.as_bytes());
    let found = answers.iter().position(|a| ok && is_password_prompt(prompt, &a.user, &a.host));
    match found {
        Some(i) => {
            let password = &answers[i].password;
            let mut reply = Zeroizing::new(Vec::with_capacity(password.len() + 1));
            reply.push(b'P');
            reply.extend_from_slice(password.as_bytes());
            let _ = s.write_all(&reply);
        }
        None => {
            let _ = s.write_all(b"N");
        }
    }
    found
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ------------------------------------------------------------ the helper

/// Run as the `SSH_ASKPASS` helper when ssh started this program so;
/// returns the exit code. Nothing is logged here.
pub fn helper_if_asked() -> Option<i32> {
    let sock = std::env::var_os(ENV_SOCKET)?;
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let kind = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
    Some(helper(Path::new(&sock), &token, &prompt, &kind))
}

fn helper(sock: &Path, token: &str, prompt: &str, kind: &str) -> i32 {
    // Confirmations and notices are never answered by the application.
    if kind.is_empty() {
        if let Some(pw) = ask_application(sock, token, prompt) {
            let mut out = std::io::stdout().lock();
            let ok = out.write_all(pw.as_bytes()).and_then(|_| out.write_all(b"\n")).and_then(|_| out.flush());
            return if ok.is_ok() { 0 } else { 1 };
        }
    }
    ask_terminal(prompt, kind)
}

fn ask_application(sock: &Path, token: &str, prompt: &str) -> Option<Zeroizing<String>> {
    let mut s = UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(token.as_bytes()).ok()?;
    s.write_all(b"\n").ok()?;
    s.write_all(prompt.as_bytes()).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut buf = Zeroizing::new(Vec::new());
    s.take(64 * 1024).read_to_end(&mut buf).ok()?;
    let pw = buf.strip_prefix(b"P")?;
    Some(Zeroizing::new(String::from_utf8_lossy(pw).into_owned()))
}

/// Ask the user on the terminal itself (as ssh would without the helper).
fn ask_terminal(prompt: &str, kind: &str) -> i32 {
    let Ok(mut tty) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty") else { return 1 };
    let _ = tty.write_all(prompt.as_bytes());
    let _ = tty.flush();
    if kind == "none" {
        let _ = tty.write_all(b"\n");
        return 0;
    }
    let secret = kind != "confirm";
    let old = rustix::termios::tcgetattr(&tty).ok();
    if secret {
        if let Some(o) = &old {
            let mut t = o.clone();
            t.local_modes.remove(rustix::termios::LocalModes::ECHO);
            let _ = rustix::termios::tcsetattr(&tty, rustix::termios::OptionalActions::Flush, &t);
        }
    }
    let mut line = Zeroizing::new(Vec::new());
    let mut b = [0u8; 1];
    while let Ok(1) = tty.read(&mut b) {
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    if secret {
        if let Some(o) = &old {
            let _ = rustix::termios::tcsetattr(&tty, rustix::termios::OptionalActions::Flush, o);
        }
        let _ = tty.write_all(b"\n");
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    let mut out = std::io::stdout().lock();
    let ok = out.write_all(&line).and_then(|_| out.write_all(b"\n")).and_then(|_| out.flush());
    if ok.is_ok() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_password_question_of_that_server() {
        let ok = |p: &str| is_password_prompt(p, "user", "Server.Example");
        assert!(ok("user@server.example's password: "));
        assert!(ok("(user@server.example) Password: "));
        assert!(!ok("root@server.example's password: "));
        assert!(!ok("user@other.example's password: "));
        assert!(!ok("user@server.example.evil's password: "));
        assert!(!ok("Are you sure you want to continue connecting (yes/no/[fingerprint])? "));
        assert!(!ok("Enter passphrase for key '/home/user/.ssh/id_ed25519': "));
        assert!(!ok("(user@server.example) Verification code: "));
        assert!(!ok(""));
    }

    #[test]
    fn the_proxy_command_quotes_for_sh_and_doubles_percent() {
        let hop = |h: &str, p: u16, k: Option<&str>| SshHop { host: h.into(), port: p, user: "it's me".into(), key: k.map(Into::into) };
        let kh = [PathBuf::from("/c/50%/kh")];
        let one = proxy_command(&[hop("jump.example", 2200, Some("/k/my key"))], ("10.0.0.5", 22), &kh);
        assert_eq!(
            one,
            r#"'ssh' '-p' '2200' '-l' 'it'\''s me' '-o' 'StrictHostKeyChecking=yes' '-o' 'UserKnownHostsFile="/c/50%%%%/kh"' '-o' 'GlobalKnownHostsFile=/dev/null' '-i' '/k/my key' '-W' '10.0.0.5:22' '--' 'jump.example'"#
        );
        let two = proxy_command(&[hop("inner", 22, None), hop("outer", 22, None)], ("fe80::1", 2222), &kh);
        assert!(two.contains("'-W' '[fe80::1]:2222' '--' 'inner'"), "{two}");
        // The inner command is quoted inside the outer one, its % doubled twice.
        assert!(two.contains("ProxyCommand="), "{two}");
        assert!(two.contains("%%%%%%%%"), "{two}");
    }

    /// The command is run for real: two and three OpenSSH servers on this
    /// computer, the server reached through one and two jump hosts, with the
    /// lists of known servers in a folder whose name has a space and a `%`.
    #[test]
    fn ssh_goes_through_the_jump_hosts() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let (Some(a), Some(b), Some(c)) = (
            sshd_for_tests::Sshd::start("term-a"),
            sshd_for_tests::Sshd::start("term-b"),
            sshd_for_tests::Sshd::start("term-c"),
        ) else {
            return;
        };
        if find_program("ssh").is_none() {
            eprintln!("SKIP: no ssh");
            return;
        }
        let dir = b.dir.join("lists 50%");
        std::fs::create_dir_all(&dir).unwrap();
        let kh = dir.join("known_hosts");
        let text: String = [&a, &b, &c].iter().map(|s| std::fs::read_to_string(&s.known_hosts).unwrap()).collect();
        std::fs::write(&kh, text).unwrap();
        let hop = |s: &sshd_for_tests::Sshd| SshHop {
            host: "127.0.0.1".into(),
            port: s.port,
            user: s.user.clone(),
            key: Some(s.client_key.display().to_string()),
        };
        for jumps in [vec![hop(&b)], vec![hop(&b), hop(&c)]] {
            let t = SshTarget {
                host: "127.0.0.1".into(),
                port: a.port,
                user: a.user.clone(),
                key: Some(a.client_key.display().to_string()),
                path: "/".into(),
                jumps,
            };
            let mut cmd = ssh_command(&t, std::slice::from_ref(&kh), None);
            // Instead of the login shell: say where we are.
            *cmd.last_mut().unwrap() = "echo arrived-$((40+2))".into();
            cmd.retain(|x| x != "-t");
            let out = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .env("SSH_AUTH_SOCK", "")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(stdout.contains("arrived-42"), "{} hops: {stdout} {}", t.jumps.len(), String::from_utf8_lossy(&out.stderr));
        }
        // A jump host whose key is not known: refused, never asked.
        std::fs::write(&kh, std::fs::read_to_string(&a.known_hosts).unwrap()).unwrap();
        let t = SshTarget { host: "127.0.0.1".into(), port: a.port, user: a.user.clone(), key: Some(a.client_key.display().to_string()), path: "/".into(), jumps: vec![hop(&b)] };
        let mut cmd = ssh_command(&t, std::slice::from_ref(&kh), None);
        *cmd.last_mut().unwrap() = "echo arrived".into();
        cmd.retain(|x| x != "-t");
        let out = std::process::Command::new(&cmd[0]).args(&cmd[1..]).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(!String::from_utf8_lossy(&out.stdout).contains("arrived"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("verification failed"), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn ssh_command_has_no_password() {
        let t = SshTarget { host: "srv".into(), port: 2222, user: "u".into(), key: Some("/k/id".into()), path: "/var/www/it's".into(), jumps: Vec::new() };
        let kh = [PathBuf::from("/c/rolid scp/known_hosts"), PathBuf::from("/h/.ssh/known_hosts")];
        let c = ssh_command(&t, &kh, Some((Path::new("/usr/bin/rolidscp"), Path::new("/run/x/s"), "tok")));
        let joined = c.join(" ");
        assert!(joined.contains("StrictHostKeyChecking=yes"));
        assert!(joined.contains(r#"UserKnownHostsFile="/c/rolid scp/known_hosts" "/h/.ssh/known_hosts""#));
        assert!(joined.contains("SSH_ASKPASS=/usr/bin/rolidscp"));
        assert!(joined.contains("-i /k/id"));
        assert_eq!(c[c.len() - 2], "srv");
        assert!(c.last().unwrap().ends_with(r"sh '/var/www/it'\''s'"));
        let plain = ssh_command(&t, &[PathBuf::from("/kh")], None);
        assert_eq!(known_hosts_option(&[PathBuf::from(r#"/a"b\c%d"#)]), r#"UserKnownHostsFile="/a\"b\\c%%d""#);
        assert_eq!(plain[0], "ssh");
        assert!(!plain.join(" ").contains("SSH_ASKPASS"));
    }

    #[test]
    fn terminal_arguments() {
        let cmd = vec!["ssh".to_string(), "x".to_string()];
        assert_eq!(terminal_args("/usr/bin/x-terminal-emulator", &cmd), ["-e", "ssh", "x"]);
        assert_eq!(terminal_args("gnome-terminal", &cmd), ["--", "ssh", "x"]);
        assert_eq!(terminal_args("xfce4-terminal", &cmd), ["-x", "ssh", "x"]);
        assert_eq!(terminal_args("kitty", &cmd), ["ssh", "x"]);
        assert_eq!(terminal_args("wezterm", &cmd), ["start", "--", "ssh", "x"]);
        assert!(terminal_args("xterm", &[]).is_empty());
    }

    #[test]
    fn answers_once_and_only_the_right_question() {
        let a = |user: &str, host: &str, pw: &str| Answer { user: user.into(), host: host.into(), password: Zeroizing::new(pw.into()) };
        let o = offer(vec![a("user", "127.0.0.1", "secret password"), a("hop", "jump.example", "jump password")]).unwrap();
        let sock = o.socket.clone();
        // A wrong token, another question: nothing.
        assert!(ask_application(&sock, "bad", "user@127.0.0.1's password: ").is_none());
        assert!(ask_application(&sock, &o.token, "Enter passphrase for key 'x': ").is_none());
        assert!(ask_application(&sock, &o.token, "other@jump.example's password: ").is_none());
        // The password questions: each password, once.
        assert_eq!(ask_application(&sock, &o.token, "hop@jump.example's password: ").as_deref().map(|s| s.as_str()), Some("jump password"));
        assert!(ask_application(&sock, &o.token, "hop@jump.example's password: ").is_none());
        assert_eq!(ask_application(&sock, &o.token, "user@127.0.0.1's password: ").as_deref().map(|s| s.as_str()), Some("secret password"));
        assert!(ask_application(&sock, &o.token, "user@127.0.0.1's password: ").is_none());
        // The socket and its folder are gone.
        for _ in 0..50 {
            if !sock.parent().unwrap().exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!sock.parent().unwrap().exists());
    }
}
