//! Ctrl+P: a terminal in the active panel's folder; on a server `ssh` into
//! the folder, with the saved password passed by the SSH_ASKPASS helper
//! (see `crate::terminal`).

use rolid_config::PasswordStore;

use super::{App, Dialog};
use crate::dialog;
use crate::look::Role;
use crate::terminal::{self, SshHop, SshTarget};

/// A terminal to open on a server.
#[derive(Clone)]
pub struct TermRequest {
    target: SshTarget,
    /// Saved profiles of the server and its jump hosts, in the order of
    /// `target` then `target.jumps` (their passwords may be in the vault).
    profiles: Vec<Option<String>>,
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
            target: SshTarget {
                host: r.target.host.clone(),
                port: r.target.port,
                user: r.target.user.clone(),
                key: r.target.key_path.clone(),
                path,
                jumps: r
                    .target
                    .jumps()
                    .iter()
                    .map(|j| SshHop { host: j.host.clone(), port: j.port, user: j.user.clone(), key: j.key_path.clone() })
                    .collect(),
            },
            profiles: std::iter::once(r.target.profile.clone()).chain(r.target.jumps().iter().map(|j| j.profile.clone())).collect(),
            name,
        };
        if r.production {
            self.dialog = Dialog::TermConfirm(Box::new(TermConfirm { req }));
        } else {
            self.terminal_with_password(req);
        }
    }

    /// The saved passwords (of the server and its jump hosts) from the vault,
    /// unlocking it first; without them ssh asks itself.
    fn terminal_with_password(&mut self, req: TermRequest) {
        let saved = |a: &App, id: &Option<String>| {
            id.as_deref().and_then(|id| a.profiles.get(id)).is_some_and(|p| p.save_password && p.password_store == PasswordStore::Vault)
        };
        if !req.profiles.iter().any(|id| saved(self, id)) {
            self.launch_ssh(req, Vec::new());
            return;
        }
        let (ok_req, cancel_req) = (req.clone(), req.clone());
        let reason = tr!("The saved password for “{name}” will be passed to the terminal.", name = req.name.as_str());
        self.with_vault(
            reason,
            Box::new(move |a: &mut App| {
                let hosts = std::iter::once((ok_req.target.user.clone(), ok_req.target.host.clone()))
                    .chain(ok_req.target.jumps.iter().map(|j| (j.user.clone(), j.host.clone())));
                let answers = ok_req
                    .profiles
                    .iter()
                    .zip(hosts)
                    .filter(|(id, _)| saved(a, id))
                    .filter_map(|(id, (user, host))| {
                        a.vault_password(id.as_deref()?).map(|password| terminal::Answer { user, host, password })
                    })
                    .collect();
                a.launch_ssh(ok_req, answers);
            }),
            Some(Box::new(move |a: &mut App| a.launch_ssh(cancel_req, Vec::new()))),
        );
    }

    fn launch_ssh(&mut self, req: TermRequest, answers: Vec<terminal::Answer>) {
        if terminal::find_program("ssh").is_none() {
            self.last_error = Some(tr!("Cannot open a terminal on the server: the ssh program is not installed."));
            return;
        }
        let kh = terminal::known_hosts();
        let offer = if answers.is_empty() {
            None
        } else {
            match terminal::offer(answers) {
                Ok(o) => Some(o),
                Err(e) => {
                    // The terminal still opens; ssh asks for the password itself.
                    tracing::warn!("askpass: {e}");
                    None
                }
            }
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
