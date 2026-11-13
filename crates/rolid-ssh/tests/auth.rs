//! Login against an in-process russh server offering only selected methods.
mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rolid_ssh::{ConnectParams, KbdPrompter, KbdRequest, SshConnection, SshError};
use russh::MethodKind;
use zeroize::Zeroizing;

/// Leading/trailing spaces and non-Latin characters must reach the server unchanged.
const PW: &str = " Žluťoučký kůň – пароль 1 ";

fn params(srv: &common::Running, password: Option<&str>) -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port: srv.port,
        user: "tester".into(),
        password: password.map(Into::into),
        accept_fingerprint: Some(srv.fingerprint.clone()),
        known_hosts: Some(srv.known_hosts.clone()),
        skip_agent: true,
        skip_default_keys: true,
        ..Default::default()
    }
}

/// Prompter that records requests and answers with fixed values (or cancels).
struct Mock {
    answers: Option<Vec<String>>,
    seen: Mutex<Vec<KbdRequest>>,
}

#[async_trait]
impl KbdPrompter for Mock {
    async fn prompt(&self, req: KbdRequest) -> Option<Vec<Zeroizing<String>>> {
        self.seen.lock().unwrap().push(req);
        self.answers.as_ref().map(|a| a.iter().cloned().map(Zeroizing::new).collect())
    }
}

#[tokio::test]
async fn password_only_server() {
    let srv = common::start(common::ServerSpec::password_only(PW), &common::tmpdir("pw")).await;
    SshConnection::connect(&params(&srv, Some(PW))).await.expect("login");
    let seen = srv.seen.lock().unwrap();
    assert_eq!(seen.methods_tried, ["none", "password"]);
    assert_eq!(seen.password_attempts, [PW]);
}

#[tokio::test]
async fn keyboard_interactive_only_server_uses_the_password() {
    let srv = common::start(common::ServerSpec::kbd_only(PW), &common::tmpdir("kbd")).await;
    SshConnection::connect(&params(&srv, Some(PW))).await.expect("login");
    let seen = srv.seen.lock().unwrap();
    // The password method is not offered, so it must not be tried.
    assert_eq!(seen.methods_tried, ["none", "keyboard-interactive"]);
    assert!(seen.password_attempts.is_empty());
    assert_eq!(seen.kbd_responses, [vec![PW.to_string()]]);
}

#[tokio::test]
async fn several_prompts_go_to_the_user() {
    let mut spec = common::ServerSpec::kbd_only(PW);
    spec.prompts = vec![("Password: ".into(), false), ("Verification code: ".into(), true)];
    spec.answers = vec![PW.into(), "123456".into()];
    let srv = common::start(spec, &common::tmpdir("2fa")).await;
    let mock = Arc::new(Mock { answers: Some(vec![PW.into(), "123456".into()]), seen: Mutex::new(vec![]) });
    let mut p = params(&srv, Some(PW));
    p.prompter = Some(mock.clone());
    SshConnection::connect(&p).await.expect("login");
    let reqs = mock.seen.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].prompts.len(), 2);
    assert_eq!(reqs[0].label, "tester@127.0.0.1");
}

#[tokio::test]
async fn unknown_prompt_goes_to_the_user() {
    let mut spec = common::ServerSpec::kbd_only("000111");
    spec.prompts = vec![("One-time code: ".into(), false)];
    let srv = common::start(spec, &common::tmpdir("otp")).await;
    let mock = Arc::new(Mock { answers: Some(vec!["000111".into()]), seen: Mutex::new(vec![]) });
    let mut p = params(&srv, Some(PW));
    p.prompter = Some(mock.clone());
    SshConnection::connect(&p).await.expect("login");
    assert_eq!(mock.seen.lock().unwrap().len(), 1, "the password must not be sent to an OTP prompt");
}

#[tokio::test]
async fn rejected_password_is_not_resent_via_keyboard_interactive() {
    let mut spec = common::ServerSpec::kbd_only("correct");
    spec.methods = vec![MethodKind::Password, MethodKind::KeyboardInteractive];
    spec.password = "correct".into();
    let srv = common::start(spec, &common::tmpdir("wrong")).await;
    let mock = Arc::new(Mock { answers: None, seen: Mutex::new(vec![]) }); // user cancels
    let mut p = params(&srv, Some("wrong"));
    p.prompter = Some(mock.clone());
    let err = SshConnection::connect(&p).await.err().expect("must fail");
    let msg = err.to_string();
    assert!(matches!(err, SshError::AuthFailed(_)), "{msg}");
    assert!(msg.contains("The server offers: password, keyboard-interactive."), "{msg}");
    assert!(msg.contains("password (rejected)"), "{msg}");
    assert!(msg.contains("canceled"), "{msg}");
    let seen = srv.seen.lock().unwrap();
    assert_eq!(seen.password_attempts, ["wrong"]);
    assert!(seen.kbd_responses.is_empty(), "rejected password must not be sent again");
    assert!(mock.seen.lock().unwrap()[0].retry);
}

#[tokio::test]
async fn nothing_to_offer_gives_a_clear_message() {
    let srv = common::start(common::ServerSpec::kbd_only(PW), &common::tmpdir("none")).await;
    let err = SshConnection::connect(&params(&srv, None)).await.err().expect("must fail");
    assert_eq!(
        err.to_string(),
        "login failed. The server offers: keyboard-interactive. Tried: nothing – a credential the server \
         accepts is missing (password or key)."
    );
}
