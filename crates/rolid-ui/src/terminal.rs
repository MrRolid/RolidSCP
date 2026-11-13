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
}

/// The known_hosts file the application checks servers against.
pub fn known_hosts() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

/// `ssh` with the application's host key checking, into `t.path` with a login shell.
/// `askpass`: (helper program, socket, token) when a password is offered.
pub fn ssh_command(t: &SshTarget, known_hosts: &Path, askpass: Option<(&Path, &Path, &str)>) -> Vec<String> {
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
    c.extend(["-o".into(), format!("UserKnownHostsFile={}", known_hosts.display())]);
    c.extend(["-o".into(), "GlobalKnownHostsFile=/dev/null".into()]);
    if let Some(k) = &t.key {
        c.extend(["-i".into(), k.clone()]);
    }
    c.push("--".into());
    c.push(t.host.clone());
    // Run by the user's login shell (sh, bash, fish, csh…): only plain quoting.
    let q = rolid_search::remote::sh_quote(&t.path);
    c.push(format!("exec sh -c 'cd \"$1\" 2>/dev/null; exec \"${{SHELL:-/bin/sh}}\" -l' sh {q}"));
    c
}

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

/// Offer `password` for the password question of `user`@`host`: once, for
/// [`OFFER_FOR`]; then the socket and its folder are removed.
pub fn offer(user: &str, host: &str, password: Zeroizing<String>) -> std::io::Result<Offer> {
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
    let (user, host, tok) = (user.to_string(), host.to_string(), token.clone());
    std::thread::Builder::new().name("rolid-askpass".into()).spawn(move || {
        serve(&listener, &tok, &user, &host, &password, Instant::now() + OFFER_FOR);
        drop(listener);
        let _ = std::fs::remove_file(dir.join("s"));
        let _ = std::fs::remove_dir(&dir);
    })?;
    Ok(Offer { socket, token })
}

fn serve(listener: &UnixListener, token: &str, user: &str, host: &str, password: &Zeroizing<String>, until: Instant) {
    while Instant::now() < until {
        match listener.accept() {
            Ok((s, _)) => {
                if answer(s, token, user, host, password) {
                    return; // answered once: no more
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
}

/// One question from the helper; `true` when the password was given.
fn answer(mut s: UnixStream, token: &str, user: &str, host: &str, password: &Zeroizing<String>) -> bool {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    // Only a process of this user.
    let same_user = rustix::net::sockopt::socket_peercred(&s).is_ok_and(|c| c.uid == rustix::process::getuid());
    let mut buf = Vec::new();
    let _ = (&mut s).take(8192).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let (got_token, prompt) = text.split_once('\n').unwrap_or((&text, ""));
    let ok = same_user && constant_eq(got_token.as_bytes(), token.as_bytes()) && is_password_prompt(prompt, user, host);
    if ok {
        let mut reply = Zeroizing::new(Vec::with_capacity(password.len() + 1));
        reply.push(b'P');
        reply.extend_from_slice(password.as_bytes());
        let _ = s.write_all(&reply);
    } else {
        let _ = s.write_all(b"N");
    }
    ok
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
    fn ssh_command_has_no_password() {
        let t = SshTarget { host: "srv".into(), port: 2222, user: "u".into(), key: Some("/k/id".into()), path: "/var/www/it's".into() };
        let c = ssh_command(&t, Path::new("/h/.ssh/known_hosts"), Some((Path::new("/usr/bin/rolidscp"), Path::new("/run/x/s"), "tok")));
        let joined = c.join(" ");
        assert!(joined.contains("StrictHostKeyChecking=yes"));
        assert!(joined.contains("UserKnownHostsFile=/h/.ssh/known_hosts"));
        assert!(joined.contains("SSH_ASKPASS=/usr/bin/rolidscp"));
        assert!(joined.contains("-i /k/id"));
        assert_eq!(c[c.len() - 2], "srv");
        assert!(c.last().unwrap().ends_with(r"sh '/var/www/it'\''s'"));
        let plain = ssh_command(&t, Path::new("/kh"), None);
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
        let o = offer("user", "127.0.0.1", Zeroizing::new("secret password".into())).unwrap();
        let sock = o.socket.clone();
        // A wrong token, another question: nothing.
        assert!(ask_application(&sock, "bad", "user@127.0.0.1's password: ").is_none());
        assert!(ask_application(&sock, &o.token, "Enter passphrase for key 'x': ").is_none());
        // The password question: the password, once.
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
