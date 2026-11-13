//! Test SSH server for manual login testing (authentication only, no SFTP).
//!
//! `cargo run -p rolid-ssh --example auth_server -- <port> <password|kbd|otp> <password>`
//! Every login attempt is printed (methods only, never the secret itself).
#[path = "../tests/common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let port: u16 = a.get(1).and_then(|p| p.parse().ok()).unwrap_or(2223);
    let pw = a.get(3).cloned().unwrap_or_else(|| "password".into());
    let spec = match a.get(2).map(String::as_str) {
        Some("password") => common::ServerSpec::password_only(&pw),
        Some("otp") => {
            let mut s = common::ServerSpec::kbd_only(&pw);
            s.prompts = vec![("Verification code: ".into(), true)];
            s
        }
        _ => common::ServerSpec::kbd_only(&pw),
    };
    println!("listening on 127.0.0.1:{port}, methods {:?}", spec.methods);
    common::serve_forever(spec, port).await;
}
