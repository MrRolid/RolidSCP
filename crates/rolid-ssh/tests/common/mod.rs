//! In-process SSH test server (russh server) with configurable auth methods.
#![allow(dead_code)]

use std::sync::Arc;

use russh::keys::{HashAlg, PrivateKey};
use russh::server::{self, Auth, Response};
use russh::{MethodKind, MethodSet};

/// Host key of the test server: a fresh Ed25519 key generated at the first use
/// in each test run and written to a private temporary folder (no key is kept
/// in the repository).
pub fn host_key() -> PrivateKey {
    static KEY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let path = KEY.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rolid-test-hostkey-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = dir.join("host_key");
        generate_host_key(&path);
        path
    });
    PrivateKey::read_openssh_file(path).unwrap()
}

/// Generate a new Ed25519 key with a random seed from the OS and save it in the
/// OpenSSH format (mode 0600) at `path`.
pub fn generate_host_key(path: &std::path::Path) {
    use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
    use russh::keys::ssh_key::LineEnding;
    use std::io::Read;
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    std::fs::File::open("/dev/urandom").unwrap().read_exact(seed.as_mut()).unwrap();
    let pair = Ed25519Keypair::from_seed(&seed);
    let key = PrivateKey::new(KeypairData::Ed25519(pair), "rolid-test-host").unwrap();
    key.write_openssh_file(path, LineEnding::LF).unwrap();
}

#[derive(Clone, Debug)]
pub struct ServerSpec {
    /// Methods the server offers.
    pub methods: Vec<MethodKind>,
    pub password: String,
    /// Keyboard-interactive prompts (text, echo); answers must all equal `answers`.
    pub prompts: Vec<(String, bool)>,
    pub answers: Vec<String>,
}

impl ServerSpec {
    pub fn password_only(pw: &str) -> Self {
        Self { methods: vec![MethodKind::Password], password: pw.into(), prompts: vec![], answers: vec![] }
    }

    pub fn kbd_only(pw: &str) -> Self {
        Self {
            methods: vec![MethodKind::KeyboardInteractive],
            password: String::new(),
            prompts: vec![("Password: ".into(), false)],
            answers: vec![pw.into()],
        }
    }
}

/// What the server saw (for assertions).
#[derive(Default, Debug)]
pub struct Seen {
    pub password_attempts: Vec<String>,
    pub kbd_responses: Vec<Vec<String>>,
    pub methods_tried: Vec<&'static str>,
}

pub struct Running {
    pub port: u16,
    pub fingerprint: String,
    pub seen: Arc<std::sync::Mutex<Seen>>,
    pub known_hosts: std::path::PathBuf,
}

struct H {
    spec: ServerSpec,
    seen: Arc<std::sync::Mutex<Seen>>,
}

impl H {
    fn reject(&self) -> Auth {
        Auth::Reject { proceed_with_methods: Some(MethodSet::from(&self.spec.methods[..])), partial_success: false }
    }
}

impl server::Handler for H {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        self.seen.lock().unwrap().methods_tried.push("none");
        Ok(self.reject())
    }

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        let mut s = self.seen.lock().unwrap();
        s.methods_tried.push("password");
        s.password_attempts.push(password.to_string());
        if self.spec.methods.contains(&MethodKind::Password) && password == self.spec.password {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        _offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        self.seen.lock().unwrap().methods_tried.push("publickey");
        Ok(self.reject())
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if !self.spec.methods.contains(&MethodKind::KeyboardInteractive) {
            self.seen.lock().unwrap().methods_tried.push("keyboard-interactive");
            return Ok(self.reject());
        }
        match response {
            None => {
                self.seen.lock().unwrap().methods_tried.push("keyboard-interactive");
                let prompts: Vec<(std::borrow::Cow<'static, str>, bool)> =
                    self.spec.prompts.iter().map(|(p, e)| (p.clone().into(), *e)).collect();
                Ok(Auth::Partial { name: "".into(), instructions: "".into(), prompts: prompts.into() })
            }
            Some(r) => {
                let got: Vec<String> = r.map(|b| String::from_utf8_lossy(&b).into_owned()).collect();
                let ok = got == self.spec.answers;
                self.seen.lock().unwrap().kbd_responses.push(got);
                Ok(if ok { Auth::Accept } else { self.reject() })
            }
        }
    }
}

fn config(spec: &ServerSpec) -> (Arc<server::Config>, String) {
    let key = host_key();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let mut methods = MethodSet::empty();
    for m in &spec.methods {
        methods.push(*m);
    }
    let config = Arc::new(server::Config {
        methods,
        keys: vec![key],
        auth_rejection_time: std::time::Duration::ZERO,
        auth_rejection_time_initial: Some(std::time::Duration::ZERO),
        ..Default::default()
    });
    (config, fingerprint)
}

/// Serve connections on `port` until the process ends (manual / GUI testing).
pub async fn serve_forever(spec: ServerSpec, port: u16) -> Arc<std::sync::Mutex<Seen>> {
    let (config, fingerprint) = config(&spec);
    println!("fingerprint {fingerprint}");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Seen::default()));
    loop {
        let (sock, _) = listener.accept().await.unwrap();
        let h = H { spec: spec.clone(), seen: seen.clone() };
        let config = config.clone();
        tokio::spawn(async move {
            if let Ok(s) = server::run_stream(config, sock, h).await {
                let _ = s.await;
            }
        });
    }
}

/// Start a server for one connection on 127.0.0.1 (random port).
pub async fn start(spec: ServerSpec, dir: &std::path::Path) -> Running {
    let (config, fingerprint) = config(&spec);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(std::sync::Mutex::new(Seen::default()));
    let h = H { spec, seen: seen.clone() };
    tokio::spawn(async move {
        if let Ok((sock, _)) = listener.accept().await {
            if let Ok(s) = server::run_stream(config, sock, h).await {
                let _ = s.await;
            }
        }
    });
    std::fs::create_dir_all(dir).unwrap();
    Running { port, fingerprint, seen, known_hosts: dir.join("known_hosts") }
}

pub fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rolid-auth-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn host_key_is_generated_fresh_and_private() {
        let d = tmpdir("keygen");
        let (a, b) = (d.join("a"), d.join("b"));
        generate_host_key(&a);
        generate_host_key(&b);
        let ka = PrivateKey::read_openssh_file(&a).unwrap();
        let kb = PrivateKey::read_openssh_file(&b).unwrap();
        assert_eq!(ka.algorithm(), russh::keys::Algorithm::Ed25519);
        assert_ne!(ka.public_key().fingerprint(HashAlg::Sha256), kb.public_key().fingerprint(HashAlg::Sha256));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o077, 0, "key readable by others");
        }
        // The shared key is the same within one run.
        assert_eq!(host_key().public_key(), host_key().public_key());
        let _ = std::fs::remove_dir_all(&d);
    }
}
