//! SSH user authentication: agent, key files, password, keyboard-interactive.
//!
//! The server is first asked with the `none` method which methods it offers, and
//! only offered methods are tried, in this order: ssh-agent, key files, password,
//! keyboard-interactive. After every failed attempt the server's remaining
//! methods are logged and respected (this also handles multi-step logins with
//! `partial_success`).
//!
//! Secrets are never logged, not even their length.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::{self, PrivateKeyWithHashAlg};
use rolid_core::vfs::raw_text;
use rolid_text::{txt, txtn, Arg, Text};
use russh::{MethodKind, MethodSet};
use zeroize::Zeroizing;

use crate::{ClientHandler, ConnectParams, SshError};

/// One keyboard-interactive question round, to be answered by the user.
#[derive(Debug, Clone)]
pub struct KbdRequest {
    /// `user@host`, for the dialog title.
    pub label: String,
    pub name: String,
    pub instructions: String,
    /// (prompt text, echo the answer?)
    pub prompts: Vec<(String, bool)>,
    /// The password we sent automatically was rejected and the server asks again.
    pub retry: bool,
}

/// Answers keyboard-interactive questions the client cannot answer itself
/// (e.g. one-time codes, several prompts). Returning `None` cancels the login.
#[async_trait]
pub trait KbdPrompter: Send + Sync {
    async fn prompt(&self, req: KbdRequest) -> Option<Vec<Zeroizing<String>>>;
}

/// The authentication steps, in the order they are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Agent,
    /// Index into the loaded key files.
    Key(usize),
    Password,
    KeyboardInteractive,
}

impl Step {
    pub fn method(self) -> MethodKind {
        match self {
            Step::Agent | Step::Key(_) => MethodKind::PublicKey,
            Step::Password => MethodKind::Password,
            Step::KeyboardInteractive => MethodKind::KeyboardInteractive,
        }
    }
}

/// What the client has available for authentication.
#[derive(Debug, Clone, Copy, Default)]
pub struct Available {
    pub agent: bool,
    pub keys: usize,
    pub password: bool,
    /// Someone can answer keyboard-interactive prompts (the user via a dialog).
    pub prompter: bool,
}

/// Candidate steps in order, before looking at what the server offers.
pub fn plan(a: Available) -> Vec<Step> {
    let mut v = Vec::new();
    if a.agent {
        v.push(Step::Agent);
    }
    v.extend((0..a.keys).map(Step::Key));
    if a.password {
        v.push(Step::Password);
    }
    if a.password || a.prompter {
        v.push(Step::KeyboardInteractive);
    }
    v
}

/// Is `step` currently allowed by the server?
pub fn allowed(step: Step, offered: &[MethodKind]) -> bool {
    offered.contains(&step.method())
}

/// Does a keyboard-interactive prompt ask for the account password?
pub fn is_password_prompt(text: &str, echo: bool) -> bool {
    if echo {
        return false;
    }
    let t = text.to_lowercase();
    ["password", "heslo", "passwort", "mot de passe", "contraseña", "hasło"] // i18n-ok: words of server prompts
        .iter()
        .any(|w| t.contains(w))
}

/// The offered methods for the log ("password, publickey").
pub fn method_names(m: &[MethodKind]) -> String {
    if m.is_empty() {
        return "none (the server closed the connection)".into();
    }
    m.iter().map(<&str>::from).collect::<Vec<_>>().join(", ")
}

/// The offered methods for the user: the method names (not translated) or a message.
fn methods_arg(m: &[MethodKind]) -> Arg {
    if m.is_empty() {
        return txt!("nothing (the server closed the connection)").into();
    }
    Arg::Str(m.iter().map(<&str>::from).collect::<Vec<_>>().join(", "))
}

/// A list of messages as one message: "a, b, c", or "a; b" with `semicolon`.
fn join(items: &[Text], semicolon: bool) -> Text {
    let mut it = items.iter();
    let mut out = it.next().cloned().unwrap_or_else(|| raw_text(""));
    for t in it {
        out = if semicolon {
            txt!("{list}; {item}", list = out, item = t)
        } else {
            txt!("{list}, {item}", list = out, item = t)
        };
    }
    out
}

/// Human-readable summary of an authentication attempt.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub offered: Vec<MethodKind>,
    /// (what, outcome)
    pub attempts: Vec<(Text, Text)>,
    pub notes: Vec<Text>,
}

impl Report {
    /// The summary for the user (English template, translated by the UI).
    pub fn text(&self) -> Text {
        let methods = methods_arg(&self.offered);
        let attempts: Vec<Text> =
            self.attempts.iter().map(|(w, o)| txt!("{method} ({outcome})", method = w, outcome = o)).collect();
        let notes = join(&self.notes, true);
        match (self.attempts.is_empty(), self.notes.is_empty()) {
            (true, true) => txt!(
                "The server offers: {methods}. Tried: nothing – a credential the server accepts is missing (password or key).",
                methods = methods
            ),
            (true, false) => txt!(
                "The server offers: {methods}. Tried: nothing – a credential the server accepts is missing (password or key). Note: {notes}.",
                methods = methods,
                notes = notes
            ),
            (false, true) => txt!(
                "The server offers: {methods}. Tried: {attempts}.",
                methods = methods,
                attempts = join(&attempts, false)
            ),
            (false, false) => txt!(
                "The server offers: {methods}. Tried: {attempts}. Note: {notes}.",
                methods = methods,
                attempts = join(&attempts, false),
                notes = notes
            ),
        }
    }
}

impl std::fmt::Display for Report {
    /// English (the log).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text())
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn other(e: impl std::fmt::Display) -> SshError {
    SshError::Other(raw_text(e))
}

/// Result of one attempt.
enum Outcome {
    Success,
    /// Rejected; the server's remaining methods and whether this step counted.
    Rejected { remaining: MethodSet, partial: bool },
}

fn outcome(r: AuthResult) -> Outcome {
    match r {
        AuthResult::Success => Outcome::Success,
        AuthResult::Failure { remaining_methods, partial_success } => {
            Outcome::Rejected { remaining: remaining_methods, partial: partial_success }
        }
    }
}

/// Load key files (explicit one, or the default ones). Encrypted default keys
/// without a passphrase are skipped quietly (a note only).
fn load_keys(p: &ConnectParams, report: &mut Report) -> Result<Vec<(PathBuf, keys::PrivateKey)>, SshError> {
    let files: Vec<PathBuf> = match &p.key_path {
        Some(k) => vec![k.clone()],
        None if p.skip_default_keys => vec![],
        None => std::env::home_dir()
            .map(|h| {
                ["id_ed25519", "id_ecdsa", "id_rsa"]
                    .iter()
                    .map(|n| h.join(".ssh").join(n))
                    .filter(|p| p.exists())
                    .collect()
            })
            .unwrap_or_default(),
    };
    let mut out = Vec::new();
    for path in files {
        match keys::load_secret_key(&path, p.key_passphrase.as_deref()) {
            Ok(k) => out.push((path, k)),
            Err(e) => {
                let encrypted = matches!(e, keys::Error::KeyIsEncrypted);
                if p.key_path.is_some() {
                    // The user chose this key explicitly: that is a real error.
                    return Err(SshError::Other(if encrypted {
                        txt!("key {path} is encrypted – enter the key passphrase", path = &path)
                    } else {
                        txt!("cannot load key {path}: {error}", path = &path, error = e.to_string())
                    }));
                }
                let name = file_name(&path);
                let note = if encrypted {
                    txt!("key {name} skipped (encrypted, no key passphrase given)", name = name)
                } else {
                    txt!("key {name} skipped (cannot be loaded: {error})", name = name, error = e.to_string())
                };
                tracing::info!(key = %path.display(), reason = %note, "default key skipped");
                report.notes.push(note);
            }
        }
    }
    Ok(out)
}

/// Authenticate `p.user`. Returns the report on failure.
pub(crate) async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    p: &ConnectParams,
) -> Result<(), SshError> {
    let label = format!("{}@{}", p.user, p.host);
    let mut report = Report::default();

    // Ask the server what it accepts.
    let mut offered: Vec<MethodKind> = match handle.authenticate_none(&p.user).await.map_err(other)? {
        AuthResult::Success => {
            tracing::info!(%label, "logged in without authentication (method none)");
            return Ok(());
        }
        AuthResult::Failure { remaining_methods, .. } => remaining_methods.to_vec(),
    };
    tracing::info!(%label, offered = %method_names(&offered), "server offers methods");
    report.offered = offered.clone();

    let use_publickey = offered.contains(&MethodKind::PublicKey);
    let keys = if use_publickey { load_keys(p, &mut report)? } else { Vec::new() };

    // ssh-agent (only without an explicit key file).
    let mut agent = None;
    if use_publickey && p.key_path.is_none() && !p.skip_agent {
        #[cfg(unix)]
        let a = keys::agent::client::AgentClient::connect_env().await;
        #[cfg(windows)]
        let a = keys::agent::client::AgentClient::connect_pageant().await;
        match a {
            Ok(a) => agent = Some(a),
            Err(e) => tracing::info!(error = %e, "ssh-agent not available"),
        }
    }

    let avail = Available {
        agent: agent.is_some(),
        keys: keys.len(),
        password: p.password.is_some(),
        prompter: p.prompter.is_some(),
    };
    let rsa_hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    let mut password_rejected = false;

    for step in plan(avail) {
        if !allowed(step, &offered) {
            tracing::info!(?step, offered = %method_names(&offered), "step skipped: the server does not offer the method");
            continue;
        }
        let (what, result): (Text, Result<Outcome, Text>) = match step {
            Step::Agent => {
                let a = agent.as_mut().expect("planned only with an agent");
                let ids = a.request_identities().await.unwrap_or_default();
                let mut tried = 0;
                let mut res = Err(txt!("no key"));
                for id in ids {
                    let keys::agent::AgentIdentity::PublicKey { key, .. } = id else { continue };
                    tried += 1;
                    match handle.authenticate_publickey_with(&p.user, key, rsa_hash, a).await {
                        Ok(r) => {
                            let o = outcome(r);
                            let done = !matches!(&o, Outcome::Rejected { remaining, partial: false }
                                if remaining.contains(&MethodKind::PublicKey));
                            res = Ok(o);
                            if done {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::info!(error = %e, "agent: signing failed");
                        }
                    }
                }
                if tried > 0 && res.is_err() {
                    res = Err(txtn!("{n} key cannot be used", "{n} keys cannot be used", tried));
                }
                // Method names stay as they are (not translated).
                (Text::new("agent"), res)
            }
            Step::Key(i) => {
                let (path, key) = &keys[i];
                let key = PrivateKeyWithHashAlg::new(Arc::new(key.clone()), rsa_hash);
                let r = handle.authenticate_publickey(&p.user, key).await.map_err(other)?;
                (txt!("key {name}", name = file_name(path)), Ok(outcome(r)))
            }
            Step::Password => {
                let pw = p.password.as_deref().expect("planned only with a password");
                let r = handle.authenticate_password(&p.user, pw).await.map_err(other)?;
                let o = outcome(r);
                if matches!(o, Outcome::Rejected { partial: false, .. }) {
                    password_rejected = true;
                }
                (txt!("password"), Ok(o))
            }
            Step::KeyboardInteractive => {
                let o = keyboard_interactive(handle, p, &label, password_rejected).await?;
                match o {
                    Kbd::Done(o, how) => (how, Ok(o)),
                    Kbd::Canceled => {
                        report.attempts.push((Text::new("keyboard-interactive"), txt!("canceled")));
                        tracing::info!(%label, "keyboard-interactive canceled by the user");
                        return Err(SshError::AuthFailed(txt!("login canceled. {report}", report = report.text())));
                    }
                    Kbd::Unanswerable(q) => {
                        (Text::new("keyboard-interactive"), Err(txt!("cannot answer “{question}”", question = q)))
                    }
                }
            }
        };
        match result {
            Ok(Outcome::Success) => {
                tracing::info!(%label, method = %what, "login successful");
                return Ok(());
            }
            Ok(Outcome::Rejected { remaining, partial }) => {
                offered = remaining.to_vec();
                let o = if partial { txt!("accepted, the server wants another step") } else { txt!("rejected") };
                tracing::info!(%label, method = %what, result = %o, remaining = %method_names(&offered), "login attempt");
                report.attempts.push((what, o));
                if offered.is_empty() {
                    break; // the server gave up (e.g. too many attempts)
                }
            }
            Err(o) => {
                tracing::info!(%label, method = %what, result = %o, "login attempt");
                report.attempts.push((what, o));
            }
        }
    }
    tracing::info!(%label, report = %report, "login failed");
    Err(SshError::AuthFailed(report.text()))
}

enum Kbd {
    /// Finished with this outcome; the text describes how we answered.
    Done(Outcome, Text),
    Canceled,
    /// A prompt we cannot answer (no prompter): the server's question(s), or why.
    Unanswerable(Arg),
}

async fn keyboard_interactive(
    handle: &mut client::Handle<ClientHandler>,
    p: &ConnectParams,
    label: &str,
    password_rejected: bool,
) -> Result<Kbd, SshError> {
    let mut used_password = password_rejected; // never resend a rejected password
    let mut how = Text::new("keyboard-interactive");
    let mut resp = handle.authenticate_keyboard_interactive_start(&p.user, None).await.map_err(other)?;
    for _round in 0..20 {
        match resp {
            KeyboardInteractiveAuthResponse::Success => return Ok(Kbd::Done(Outcome::Success, how.clone())),
            KeyboardInteractiveAuthResponse::Failure { remaining_methods, partial_success } => {
                return Ok(Kbd::Done(
                    Outcome::Rejected { remaining: remaining_methods, partial: partial_success },
                    how.clone(),
                ));
            }
            KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                // Log prompt texts (they are server questions, not secrets).
                let texts: Vec<&str> = prompts.iter().map(|q| q.prompt.as_str()).collect();
                tracing::info!(%label, prompts = ?texts, "keyboard-interactive: server prompt");
                let answers: Vec<String> = if prompts.is_empty() {
                    Vec::new()
                } else if prompts.len() == 1
                    && is_password_prompt(&prompts[0].prompt, prompts[0].echo)
                    && !used_password
                    && p.password.is_some()
                {
                    used_password = true;
                    how = txt!("password via keyboard-interactive");
                    vec![p.password.clone().unwrap_or_default()]
                } else if let Some(pr) = &p.prompter {
                    let req = KbdRequest {
                        label: label.to_string(),
                        name,
                        instructions,
                        prompts: prompts.iter().map(|q| (q.prompt.clone(), q.echo)).collect(),
                        retry: used_password,
                    };
                    match pr.prompt(req).await {
                        Some(a) if a.len() == prompts.len() => {
                            how = txt!("keyboard-interactive with the user's answer");
                            a.iter().map(|s| s.to_string()).collect()
                        }
                        Some(_) | None => return Ok(Kbd::Canceled),
                    }
                } else {
                    return Ok(Kbd::Unanswerable(Arg::Str(texts.join(" / "))));
                };
                let mut answers = Zeroizing::new(answers);
                resp = handle
                    .authenticate_keyboard_interactive_respond(std::mem::take(&mut *answers))
                    .await
                    .map_err(other)?;
            }
        }
    }
    Ok(Kbd::Unanswerable(txt!("too many rounds of prompts").into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use MethodKind::*;

    fn run(a: Available, offered: &[MethodKind]) -> Vec<Step> {
        plan(a).into_iter().filter(|s| allowed(*s, offered)).collect()
    }

    #[test]
    fn order_is_agent_keys_password_kbd() {
        let a = Available { agent: true, keys: 2, password: true, prompter: true };
        assert_eq!(
            plan(a),
            [Step::Agent, Step::Key(0), Step::Key(1), Step::Password, Step::KeyboardInteractive]
        );
    }

    #[test]
    fn only_offered_methods_are_tried() {
        let a = Available { agent: true, keys: 1, password: true, prompter: true };
        assert_eq!(run(a, &[KeyboardInteractive]), [Step::KeyboardInteractive]);
        assert_eq!(run(a, &[Password]), [Step::Password]);
        assert_eq!(run(a, &[PublicKey]), [Step::Agent, Step::Key(0)]);
        assert_eq!(
            run(a, &[PublicKey, KeyboardInteractive, Password]),
            [Step::Agent, Step::Key(0), Step::Password, Step::KeyboardInteractive]
        );
        assert!(run(a, &[]).is_empty());
    }

    #[test]
    fn keyboard_interactive_needs_password_or_prompter() {
        let a = Available { password: false, prompter: false, ..Default::default() };
        assert!(run(a, &[KeyboardInteractive]).is_empty());
        let a = Available { prompter: true, ..Default::default() };
        assert_eq!(run(a, &[KeyboardInteractive, Password]), [Step::KeyboardInteractive]);
    }

    #[test]
    fn password_prompt_detection() {
        assert!(is_password_prompt("Password: ", false));
        assert!(is_password_prompt("user@server's password:", false));
        assert!(is_password_prompt("Heslo: ", false));
        assert!(!is_password_prompt("Password: ", true)); // echoed = not a secret
        assert!(!is_password_prompt("Verification code: ", false));
    }

    #[test]
    fn report_text() {
        let r = Report {
            offered: vec![KeyboardInteractive, Password],
            attempts: vec![
                (Text::new("agent"), Text::new("no key")),
                (Text::new("password"), Text::new("rejected")),
            ],
            notes: vec![Text::new("key {name} skipped (encrypted, no key passphrase given)").arg("name", "id_ed25519")],
        };
        assert_eq!(
            r.to_string(),
            "The server offers: keyboard-interactive, password. Tried: agent (no key), password (rejected). \
             Note: key id_ed25519 skipped (encrypted, no key passphrase given)."
        );
        let empty = Report { offered: vec![], ..Default::default() };
        assert_eq!(
            empty.to_string(),
            "The server offers: nothing (the server closed the connection). \
             Tried: nothing – a credential the server accepts is missing (password or key)."
        );
    }

    /// An encrypted default key without a passphrase is recognized as such
    /// (so it is skipped with a note, not reported as the reason of failure).
    #[test]
    #[cfg(unix)]
    fn encrypted_key_is_detected() {
        let d = std::env::temp_dir().join(format!("rolid-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let k = d.join("id_ed25519");
        let ok = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "secret", "-f"])
            .arg(&k)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("ssh-keygen not available, test skipped");
            return;
        }
        let p = ConnectParams { skip_agent: true, ..Default::default() };
        let mut report = Report::default();
        // Explicit key without passphrase = clear error.
        let explicit = ConnectParams { key_path: Some(k.clone()), ..p.clone() };
        let e = load_keys(&explicit, &mut report).expect_err("error").to_string();
        assert!(e.contains("encrypted"), "{e}");
        // With the passphrase it loads.
        let with = ConnectParams { key_path: Some(k.clone()), key_passphrase: Some("secret".into()), ..p };
        assert_eq!(load_keys(&with, &mut report).unwrap().len(), 1);
        assert!(matches!(keys::load_secret_key(&k, Option::None), Err(keys::Error::KeyIsEncrypted)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
