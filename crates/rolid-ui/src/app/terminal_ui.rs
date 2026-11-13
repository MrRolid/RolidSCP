//! Ctrl+P: a terminal in the active panel's folder; on a server `ssh` into
//! the folder, with the saved password passed by the SSH_ASKPASS helper
//! (see `crate::terminal`).

use rolid_config::PasswordStore;
use zeroize::Zeroizing;

use super::{App, Dialog};
use crate::dialog;
use crate::look::Role;
use crate::terminal::{self, SshTarget};

/// A terminal to open on a server.
#[derive(Clone)]
pub struct TermRequest {
    target: SshTarget,
    /// Saved profile (its password may be in the vault).
    profile: Option<String>,
    /// For the dialogs.
    name: String,
}

/// The production server question before the terminal opens.
pub struct TermConfirm {
    req: TermRequest,
}

impl App {
    /// Ctrl+P.
    pub(super) fn open_terminal(&mut self) {
        let at = self.active_ref();
        let Some(p) = self.pane(at) else { return };
        // Inside an archive: the folder the archive is in.
        let mut outer = p.archive.as_deref();
        while let Some(b) = outer.and_then(|a| a.back_archive.as_deref()) {
            outer = Some(b);
        }
        let (remote, path) = match outer {
            Some(a) => (a.back_remote.clone(), a.back_path.clone()),
            None => (p.remote.clone(), p.path.clone()),
        };
        let Some(r) = remote else {
            let dir = if std::path::Path::new(&path).is_dir() { path } else { dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()) };
            self.launch_terminal(Vec::new(), Some(dir));
            return;
        };
        let name = r.name.clone().unwrap_or_else(|| format!("{}@{}", r.target.user, r.target.host));
        let req = TermRequest {
            target: SshTarget { host: r.target.host.clone(), port: r.target.port, user: r.target.user.clone(), key: r.target.key_path.clone(), path },
            profile: r.target.profile.clone(),
            name,
        };
        if r.production {
            self.dialog = Dialog::TermConfirm(Box::new(TermConfirm { req }));
        } else {
            self.terminal_with_password(req);
        }
    }

    /// The saved password from the vault (unlocking it first); without it ssh asks itself.
    fn terminal_with_password(&mut self, req: TermRequest) {
        let saved = req.profile.as_deref().and_then(|id| self.profiles.get(id)).is_some_and(|p| p.save_password && p.password_store == PasswordStore::Vault);
        let Some(id) = req.profile.clone().filter(|_| saved) else {
            self.launch_ssh(req, None);
            return;
        };
        let (ok_req, cancel_req) = (req.clone(), req.clone());
        let reason = tr!("The saved password for “{name}” will be passed to the terminal.", name = req.name.as_str());
        self.with_vault(
            reason,
            Box::new(move |a: &mut App| {
                let pw = a.vault_password(&id);
                a.launch_ssh(ok_req, pw);
            }),
            Some(Box::new(move |a: &mut App| a.launch_ssh(cancel_req, None))),
        );
    }

    fn launch_ssh(&mut self, req: TermRequest, password: Option<Zeroizing<String>>) {
        if terminal::find_program("ssh").is_none() {
            self.last_error = Some(tr!("Cannot open a terminal on the server: the ssh program is not installed."));
            return;
        }
        let Some(kh) = terminal::known_hosts() else {
            self.launch_ssh_failed(tr!("no home folder"));
            return;
        };
        let offer = match password {
            Some(pw) => match terminal::offer(&req.target.user, &req.target.host, pw) {
                Ok(o) => Some(o),
                Err(e) => {
                    // The terminal still opens; ssh asks for the password itself.
                    tracing::warn!("askpass: {e}");
                    None
                }
            },
            None => None,
        };
        let exe = std::env::current_exe().ok();
        let askpass = match (&offer, &exe) {
            (Some(o), Some(exe)) => Some((exe.as_path(), o.socket.as_path(), o.token.as_str())),
            _ => None,
        };
        let cmd = terminal::ssh_command(&req.target, &kh, askpass);
        self.launch_terminal(cmd, None);
    }

    fn launch_ssh_failed(&mut self, why: String) {
        self.last_error = Some(tr!("Cannot open a terminal: {error}", error = why));
    }

    /// Start the system terminal (with `cmd`, or a shell in `dir`).
    fn launch_terminal(&mut self, cmd: Vec<String>, dir: Option<String>) {
        let Some(term) = terminal::find_terminal() else {
            self.last_error = Some(tr!("No terminal program found. Set the TERMINAL environment variable or install one (e.g. xterm)."));
            return;
        };
        let mut c = std::process::Command::new(&term);
        c.args(terminal::terminal_args(&term.to_string_lossy(), &cmd));
        if let Some(d) = &dir {
            c.current_dir(d);
        }
        c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        match c.spawn() {
            Ok(mut child) => {
                // Collect the exit status (no zombie process).
                std::thread::spawn(move || child.wait());
            }
            Err(e) => self.launch_ssh_failed(format!("{}: {e}", term.display())),
        }
    }

    pub(super) fn term_confirm_ui(&mut self, ctx: &eframe::egui::Context) {
        let Dialog::TermConfirm(d) = &self.dialog else { return };
        let name = d.req.name.clone();
        let mut choice = None;
        dialog::window("term-production", tr!("Terminal on a production server")).icon(Role::Open).danger(true).show(ctx, |ui| {
            dialog::warn(ui, tr!("“{name}” is a production server. Commands typed in the terminal take effect immediately.", name = name.as_str()), true);
            let b = dialog::buttons(ui, [dialog::btn(Role::Open, tr!("Open terminal")).default(true), dialog::cancel()]);
            if b[0] {
                choice = Some(true);
            } else if b[1] {
                choice = Some(false);
            }
        });
        if let Some(open) = choice {
            let Dialog::TermConfirm(d) = std::mem::replace(&mut self.dialog, Dialog::None) else { return };
            if open {
                self.terminal_with_password(d.req);
            }
        }
    }
}
