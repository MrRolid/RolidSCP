//! Sessions: connecting panels to servers, the site manager (saved profiles),
//! saved passwords (vault, legacy keyring) and workspace persistence.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use eframe::egui::{self, Color32, Stroke, Vec2};
use rolid_config::{secrets, Location, PanelTab, Profile, RemoteTarget, SideState, Workspace};
use rolid_core::color::{Rgb, PASTEL_PRESETS};
use rolid_core::vfs::{Reader, Writer};
use rolid_core::{Entry, Vfs, VfsError, VfsResult};
use rolid_i18n::rolid_text::txt;
use rolid_local::LocalFs;
use rolid_ssh::{ConnectParams, KbdPrompter, KbdRequest, SftpFs, SshConnection, SshError};
use zeroize::Zeroizing;

use super::{rgb32, Action, App, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::Role;
use crate::pane::{text_on, Pane, RemoteInfo};

/// Placeholder backend for a remote panel that is not connected (yet).
pub struct OfflineFs {
    pub label: String,
}

fn offline() -> VfsError {
    VfsError::Backend(txt!("not connected"))
}

#[async_trait]
impl Vfs for OfflineFs {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn join(&self, dir: &str, name: &str) -> String {
        rolid_ssh::posix_join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        rolid_ssh::posix_parent(path)
    }
    async fn home(&self) -> VfsResult<String> {
        Err(offline())
    }
    async fn canonicalize(&self, _: &str) -> VfsResult<String> {
        Err(offline())
    }
    async fn list(&self, _: &str) -> VfsResult<Vec<Entry>> {
        Err(offline())
    }
    async fn stat(&self, _: &str) -> VfsResult<Entry> {
        Err(offline())
    }
    async fn mkdir(&self, _: &str) -> VfsResult<()> {
        Err(offline())
    }
    async fn remove_file(&self, _: &str) -> VfsResult<()> {
        Err(offline())
    }
    async fn remove_dir(&self, _: &str) -> VfsResult<()> {
        Err(offline())
    }
    async fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
        Err(offline())
    }
    async fn open_read_at(&self, _: &str, _: u64) -> VfsResult<Reader> {
        Err(offline())
    }
    async fn open_write_at(&self, _: &str, _: u64) -> VfsResult<Writer> {
        Err(offline())
    }
    async fn set_attrs(&self, _: &str, _: Option<u32>, _: Option<SystemTime>) -> VfsResult<()> {
        Err(offline())
    }
}

pub fn offline_pane(
    target: RemoteTarget,
    name: Option<String>,
    production: bool,
    path: String,
) -> Pane {
    let fs = Arc::new(OfflineFs {
        label: format!("{}@{}", target.user, target.host),
    });
    let mut p = Pane::new(fs, path);
    p.remote = Some(RemoteInfo {
        target,
        name,
        production,
    });
    p.offline = true;
    p
}

/// Answers keyboard-interactive questions through a dialog in the UI thread.
struct UiPrompter {
    tx: std::sync::mpsc::Sender<Msg>,
    ctx: egui::Context,
    at: PaneRef,
}

#[async_trait]
impl KbdPrompter for UiPrompter {
    async fn prompt(&self, req: KbdRequest) -> Option<Vec<Zeroizing<String>>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let answers = (0..req.prompts.len()).map(|_| Zeroizing::default()).collect();
        let form = AuthPromptForm { at: self.at, req, answers, reply: Some(reply), focus: true };
        self.tx.send(Msg::AuthPrompt(Box::new(form))).ok()?;
        self.ctx.request_repaint();
        rx.await.ok().flatten()
    }
}

/// A keyboard-interactive question waiting for the user's answer.
pub struct AuthPromptForm {
    pub at: PaneRef,
    pub req: KbdRequest,
    answers: Vec<Zeroizing<String>>,
    reply: Option<tokio::sync::oneshot::Sender<Option<Vec<Zeroizing<String>>>>>,
    focus: bool,
}

impl Drop for AuthPromptForm {
    fn drop(&mut self) {
        self.answers.iter_mut().for_each(crate::secret::wipe);
        // Dropping an unanswered form cancels the login (the sender is dropped).
    }
}

/// Everything needed to (re)connect one panel.
pub struct ConnectRequest {
    pub target: RemoteTarget,
    pub name: Option<String>,
    pub production: bool,
    pub color: Option<Rgb>,
    pub path: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub passphrase: Option<Zeroizing<String>>,
    pub accept_fingerprint: Option<String>,
    /// Use the profile's saved password (vault, or the keyring for 0.3/0.4 profiles).
    pub use_saved: bool,
    /// After success, save `password` for the profile (in the vault).
    pub store_password: bool,
    /// Started from the dialog (errors are shown there).
    pub interactive: bool,
}

/// Non-secret part of a request, carried back with the result.
pub struct ConnectMeta {
    pub target: RemoteTarget,
    pub name: Option<String>,
    pub production: bool,
    pub color: Option<Rgb>,
    pub interactive: bool,
    pub store: Option<(String, Zeroizing<String>)>,
}

pub type ConnectResult = Result<(SshConnection, SftpFs, String), SshError>;

/// State of the site manager / connect dialog.
#[derive(Default)]
pub struct ConnectForm {
    pub target: Option<PaneRef>,
    /// Profile being edited (None = quick connect / new profile).
    pub selected: Option<String>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: Zeroizing<String>,
    pub save_password: bool,
    /// The selected profile has a saved password.
    pub password_saved: bool,
    /// ... and it is still in the system keyring (saved by 0.3/0.4).
    pub saved_in_keyring: bool,
    pub key_path: String,
    pub passphrase: Zeroizing<String>,
    pub start_path: String,
    pub color: Option<Rgb>,
    pub production: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub info: Option<String>,
    pub unknown_host: Option<(String, String)>,
    pub accept_fingerprint: Option<String>,
    pub focus: bool,
    pub confirm_delete: bool,
}

impl ConnectForm {
    pub fn wipe_secrets(&mut self) {
        crate::secret::wipe(&mut self.password);
        crate::secret::wipe(&mut self.passphrase);
    }

    /// Password for a connection attempt: exactly what is typed in the form right
    /// now (never trimmed or otherwise changed, saved profile or not); if the field
    /// is empty, the saved password is used when the profile has one.
    /// Returns (password, use the saved password).
    pub fn password_source(&self, profile_saves_password: bool) -> (Option<Zeroizing<String>>, bool) {
        if self.password.is_empty() {
            (None, profile_saves_password)
        } else {
            (Some(self.password.clone()), false)
        }
    }

    fn load_profile(&mut self, p: &Profile) {
        self.wipe_secrets();
        self.selected = Some(p.id.clone());
        self.name = p.name.clone();
        self.host = p.host.clone();
        self.port = p.port.to_string();
        self.user = p.user.clone();
        self.save_password = p.save_password;
        self.password_saved = p.save_password;
        self.saved_in_keyring = p.password_in_keyring();
        self.key_path = p.key_path.clone().unwrap_or_default();
        self.start_path = p.start_path.clone().unwrap_or_default();
        self.color = p.color.as_deref().and_then(Rgb::from_hex);
        self.production = p.production;
        self.error = None;
        self.info = None;
        self.unknown_host = None;
        self.accept_fingerprint = None;
        self.confirm_delete = false;
    }

    fn clear_for_new(&mut self) {
        let target = self.target;
        let user = default_user();
        *self = ConnectForm {
            target,
            port: "22".into(),
            user,
            focus: true,
            ..Default::default()
        };
    }

    /// Accept `user@host:port` in the host field and validate.
    fn normalize(&mut self) -> Result<(String, u16, String), String> {
        let mut host = self.host.trim().to_string();
        if let Some((u, h)) = host.split_once('@') {
            self.user = u.to_string();
            host = h.to_string();
        }
        if let Some((h, p)) = host.rsplit_once(':') {
            if p.parse::<u16>().is_ok() && !h.contains(':') {
                self.port = p.to_string();
                host = h.to_string();
            }
        }
        self.host = host.clone();
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| tr!("Invalid port"))?;
        let user = self.user.trim().to_string();
        if host.is_empty() || user.is_empty() {
            return Err(tr!("Fill in the server and the user"));
        }
        Ok((host, port, user))
    }

    fn key_path_opt(&self) -> Option<String> {
        let k = self.key_path.trim();
        (!k.is_empty()).then(|| k.to_string())
    }
}

pub fn default_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

impl App {
    // ------------------------------------------------------------ connecting

    /// Start connecting panel `at`. The pane shows "connecting…" until done.
    pub(super) fn connect(&mut self, at: PaneRef, mut req: ConnectRequest) {
        if let Some(p) = self.pane_mut(at) {
            p.connecting = true;
        }
        let key = req.target.key();
        let cached = self.live_connection(&key);
        let prof = req.target.profile.as_ref().and_then(|id| self.profiles.get(id)).cloned();
        let mut keyring_lookup = false;
        if req.password.is_none() && req.use_saved {
            match &prof {
                Some(p) if p.password_in_vault() && self.vault.open.is_some() => {
                    req.password = self.vault_password(&p.id);
                }
                // An open connection to this server is reused: no password needed.
                Some(p) if p.password_in_vault() && self.vault.exists() && cached.is_none() => {
                    // Locked: ask for the master password first, then connect.
                    let name = p.name.clone();
                    self.with_vault(
                        tr!("The connection to “{name}” uses a saved password.", name = name),
                        Box::new(move |a: &mut App| a.connect(at, req)),
                        Some(Box::new(move |a: &mut App| {
                            if let Some(p) = a.pane_mut(at) {
                                p.connecting = false;
                                p.error = Some(tr!(
                                    "Not connected: the saved passwords are locked (Enter or Ctrl+R = try again)"
                                ));
                            }
                        })),
                    );
                    return;
                }
                Some(p) if p.password_in_keyring() => keyring_lookup = true,
                _ => {}
            }
        }
        let ConnectRequest {
            target,
            name,
            production,
            color,
            path,
            password,
            passphrase,
            accept_fingerprint,
            use_saved: _,
            store_password,
            interactive,
        } = req;
        let store = match (&target.profile, &password, store_password) {
            (Some(id), Some(pw), true) => Some((id.clone(), pw.clone())),
            _ => None,
        };
        let meta = ConnectMeta {
            target: target.clone(),
            name,
            production,
            color,
            interactive,
            store,
        };
        let prompter: Arc<dyn KbdPrompter> = Arc::new(UiPrompter {
            tx: self.tx.clone(),
            ctx: self.ctx.clone(),
            at,
        });
        self.spawn(async move {
            let result: ConnectResult = async {
                // Reuse an existing connection to the same server (only a new SFTP channel).
                let conn = match cached {
                    Some(c) if c.sftp().await.is_ok() => c,
                    _ => {
                        let mut password = password;
                        let mut keyring_note = None;
                        if password.is_none() && keyring_lookup {
                            if let Some(id) = target.profile.clone() {
                                match tokio::task::spawn_blocking(move || secrets::get(&id)).await {
                                    Ok(Ok(Some(p))) => password = Some(Zeroizing::new(p)),
                                    Ok(Ok(None)) => {}
                                    Ok(Err(e)) => keyring_note = Some(e),
                                    Err(e) => keyring_note = Some(e.to_string()),
                                }
                            }
                        }
                        let params = ConnectParams {
                            host: target.host.clone(),
                            port: target.port,
                            user: target.user.clone(),
                            password: password.as_ref().map(|p| p.to_string()),
                            key_path: target.key_path.clone().map(Into::into),
                            key_passphrase: passphrase.as_ref().map(|p| p.to_string()),
                            accept_fingerprint,
                            known_hosts: None,
                            prompter: Some(prompter),
                            ..Default::default()
                        };
                        let r = SshConnection::connect(&params).await;
                        // ConnectParams holds plain copies of the secrets: wipe them.
                        let ConnectParams {
                            password: mut pw,
                            key_passphrase: mut kp,
                            ..
                        } = params;
                        {
                            use zeroize::Zeroize;
                            pw.zeroize();
                            kp.zeroize();
                        }
                        match r {
                            Err(SshError::AuthFailed(m)) if keyring_note.is_some() => {
                                return Err(SshError::AuthFailed(txt!(
                                    "{report}; system keyring unavailable: {error}",
                                    report = m,
                                    error = keyring_note.unwrap_or_default()
                                )));
                            }
                            r => r?,
                        }
                    }
                };
                let sftp = conn.sftp().await?;
                let home = sftp.home().await.unwrap_or_else(|_| "/".into());
                let start = match path {
                    Some(p) if !p.trim().is_empty() => {
                        let p = if let Some(rest) = p.strip_prefix("~/") {
                            rolid_ssh::posix_join(&home, rest)
                        } else {
                            p
                        };
                        // Fall back to home if the remembered directory is gone.
                        match sftp.stat(&p).await {
                            Ok(e) if e.is_dir_like() => sftp.canonicalize(&p).await.unwrap_or(p),
                            _ => home,
                        }
                    }
                    _ => home,
                };
                Ok((conn, sftp, start))
            }
            .await;
            Msg::Connected { at, meta, result: Box::new(result) }
        });
    }

    /// Reconnect a remote panel from its remembered target (no dialog).
    pub(super) fn connect_pane(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        let Some(r) = p.remote.clone() else { return };
        if p.connecting {
            return;
        }
        let path = Some(p.path.clone());
        let prof = r
            .target
            .profile
            .as_ref()
            .and_then(|id| self.profiles.get(id))
            .cloned();
        self.connect(
            at,
            ConnectRequest {
                target: r.target,
                name: prof.as_ref().map(|p| p.name.clone()).or(r.name),
                production: prof.as_ref().map(|p| p.production).unwrap_or(r.production),
                color: None, // keep the tab's current color on reconnect
                path,
                password: None,
                passphrase: None,
                accept_fingerprint: None,
                use_saved: prof.as_ref().is_some_and(|p| p.save_password),
                store_password: false,
                interactive: false,
            },
        );
    }

    pub(super) fn on_connected(&mut self, at: PaneRef, meta: ConnectMeta, result: ConnectResult) {
        if let Some(p) = self.pane_mut(at) {
            p.connecting = false;
        }
        match result {
            Ok((conn, sftp, path)) => {
                let link = self.register_connection(&meta.target, conn);
                let sftp: Arc<dyn Vfs> = Arc::new(rolid_ssh::RemoteFs::with_session(link, sftp));
                if let Dialog::Connect(f) = &mut self.dialog {
                    if f.target == Some(at) {
                        f.wipe_secrets();
                        self.dialog = Dialog::None;
                    }
                }
                let Some(tab) = self.tab_mut(at) else {
                    return;
                };
                if let Some(c) = meta.color {
                    tab.color = c;
                }
                let mut p = Pane::new(sftp, path.clone());
                p.remote = Some(RemoteInfo {
                    target: meta.target,
                    name: meta.name,
                    production: meta.production,
                });
                tab.pane = p;
                self.load(at, path, None);
                self.adopt_pending(at);
                if let Some((id, pw)) = meta.store {
                    self.store_password(id, pw);
                }
            }
            Err(e) => {
                if let Some(p) = self.pane_mut(at) {
                    if p.offline {
                        p.error = Some(tr!(
                            "Not connected: {error}  (Enter or Ctrl+R = try again)",
                            error = e.text()
                        ));
                    }
                }
                let form_open = matches!(&self.dialog, Dialog::Connect(f) if f.target == Some(at));
                if !form_open && !meta.interactive && matches!(self.dialog, Dialog::None) {
                    // Automatic reconnect failed: let the user fix it in the dialog,
                    // except for a changed host key, which must never be "accepted" here.
                    // Network errors (server down, timeout) only show in the panel;
                    // Enter / Ctrl+R retries.
                    if matches!(e, SshError::AuthFailed(_) | SshError::UnknownHostKey { .. }) {
                        self.open_connect_for(at, Some(&meta.target));
                    }
                }
                match &mut self.dialog {
                    Dialog::Connect(f) if f.target == Some(at) => {
                        f.busy = false;
                        match e {
                            SshError::UnknownHostKey {
                                algorithm,
                                fingerprint,
                                ..
                            } => {
                                f.unknown_host = Some((algorithm, fingerprint));
                            }
                            other => f.error = Some(rolid_i18n::t(&other.text())),
                        }
                    }
                    _ => self.last_error = Some(rolid_i18n::t(&e.text())),
                }
            }
        }
    }

    /// Open the site manager for panel `at`, preselecting a profile/target if given.
    pub(super) fn open_connect_for(&mut self, at: PaneRef, target: Option<&RemoteTarget>) {
        let mut f = ConnectForm {
            target: Some(at),
            ..Default::default()
        };
        f.clear_for_new();
        f.target = Some(at);
        if let Some(t) = target {
            if let Some(p) = t.profile.as_ref().and_then(|id| self.profiles.get(id)) {
                f.load_profile(p);
            } else {
                f.host = t.host.clone();
                f.port = t.port.to_string();
                f.user = t.user.clone();
                f.key_path = t.key_path.clone().unwrap_or_default();
            }
            f.focus = false;
        }
        self.dialog = Dialog::Connect(Box::new(f));
    }

    pub(super) fn open_connect(&mut self) {
        let at = self.active_ref();
        let target = self
            .pane(at)
            .and_then(|p| p.remote.as_ref().map(|r| r.target.clone()));
        self.open_connect_for(at, target.as_ref());
    }

    /// Connect using the dialog's values.
    fn submit_connect(&mut self) {
        let Dialog::Connect(f) = &mut self.dialog else {
            return;
        };
        let (host, port, user) = match f.normalize() {
            Ok(v) => v,
            Err(e) => {
                f.error = Some(e);
                return;
            }
        };
        f.busy = true;
        f.error = None;
        let profile = f
            .selected
            .as_ref()
            .and_then(|id| self.profiles.get(id))
            .cloned();
        let (password, use_saved) =
            f.password_source(profile.as_ref().is_some_and(|p| p.save_password));
        let req = ConnectRequest {
            target: RemoteTarget {
                profile: profile.as_ref().map(|p| p.id.clone()),
                host,
                port,
                user,
                key_path: f.key_path_opt(),
            },
            name: profile
                .as_ref()
                .map(|_| f.name.trim().to_string())
                .filter(|n| !n.is_empty()),
            production: f.production,
            color: f.color,
            path: (!f.start_path.trim().is_empty()).then(|| f.start_path.trim().to_string()),
            use_saved,
            store_password: f.save_password && profile.is_some() && password.is_some(),
            password,
            passphrase: (!f.passphrase.is_empty()).then(|| f.passphrase.clone()),
            accept_fingerprint: f.accept_fingerprint.clone(),
            interactive: true,
        };
        let at = f.target.unwrap_or_else(|| self.active_ref());
        self.connect(at, req);
    }

    /// Save the dialog's values as a profile (and the password into the vault).
    fn save_profile_from_form(&mut self) {
        let Dialog::Connect(f) = &mut self.dialog else {
            return;
        };
        let (host, port, user) = match f.normalize() {
            Ok(v) => v,
            Err(e) => {
                f.error = Some(e);
                return;
            }
        };
        let id = f.selected.clone().unwrap_or_else(rolid_config::new_id);
        let old = self.profiles.get(&id).cloned();
        let name = match f.name.trim() {
            "" => format!("{user}@{host}"),
            n => n.to_string(),
        };
        f.name = name.clone();
        let p = Profile {
            id: id.clone(),
            name,
            host,
            port,
            user,
            key_path: f.key_path_opt(),
            color: f.color.map(|c| c.to_hex()),
            start_path: (!f.start_path.trim().is_empty()).then(|| f.start_path.trim().to_string()),
            production: f.production,
            // Becomes true once the password is really in the vault (see store_password);
            // an already saved password is kept.
            save_password: f.save_password && old.as_ref().is_some_and(|o| o.save_password),
            password_store: old.as_ref().map(|o| o.password_store).unwrap_or_default(),
        };
        f.selected = Some(id.clone());
        f.error = None;
        let password = (!f.password.is_empty()).then(|| f.password.clone());
        let save_pw = f.save_password;
        if !save_pw {
            f.password_saved = false;
            f.saved_in_keyring = false;
        }
        self.profiles.upsert(p);
        match rolid_config::save_profiles(&self.profiles) {
            Ok(()) => {
                if let Dialog::Connect(f) = &mut self.dialog {
                    f.info = Some(tr!("Profile saved."));
                }
            }
            Err(e) => self.last_error = Some(tr!("Cannot save the profiles: {error}", error = e)),
        }
        match (save_pw, password) {
            (true, Some(pw)) => self.store_password(id, pw),
            (false, _) => {
                self.forget_password(&id);
                if old.as_ref().is_some_and(|o| o.password_in_keyring()) {
                    self.delete_from_keyring(id);
                }
            }
            (true, None) => {} // keep the already saved password
        }
    }

    /// The site manager shows that the profile's password is now saved.
    pub(super) fn site_manager_password_saved(&mut self, id: &str) {
        if let Dialog::Connect(f) = &mut self.dialog {
            if f.selected.as_deref() == Some(id) {
                f.password_saved = true;
                f.saved_in_keyring = false;
                f.info = Some(tr!("Password saved (protected by the master password)."));
            }
        }
    }

    /// The master password dialog was canceled: the password was not saved.
    /// One consistent message (in the site manager if it shows this profile).
    pub(super) fn password_not_saved(&mut self, id: &str) {
        let still_saved = self.profiles.get(id).is_some_and(|p| p.save_password);
        let msg = if still_saved {
            tr!("Profile saved, the saved password did not change (the master password was not entered).")
        } else {
            tr!("Profile saved without a password (the master password was not entered).")
        };
        match &mut self.dialog {
            Dialog::Connect(f) if f.selected.as_deref() == Some(id) => {
                f.password_saved = still_saved;
                if !still_saved {
                    f.saved_in_keyring = false;
                    f.save_password = false;
                }
                f.error = None;
                f.info = Some(msg);
            }
            _ => {
                self.last_error = Some(if still_saved {
                    tr!("The password did not change (the master password was not entered).")
                } else {
                    tr!("The password was not saved (the master password was not entered).")
                })
            }
        }
    }

    fn delete_profile(&mut self, id: String) {
        let in_keyring = self.profiles.get(&id).is_some_and(|p| p.password_in_keyring());
        self.profiles.remove(&id);
        if let Err(e) = rolid_config::save_profiles(&self.profiles) {
            self.last_error = Some(tr!("Cannot save the profiles: {error}", error = e));
        }
        self.forget_password(&id);
        if in_keyring {
            self.delete_from_keyring(id);
        }
        if let Dialog::Connect(f) = &mut self.dialog {
            f.clear_for_new();
            f.info = Some(tr!("Profile deleted."));
        }
    }

    /// Open a saved profile: switch to a tab (in either panel) already showing it,
    /// or open it as a new tab in the active panel.
    pub(super) fn open_profile(&mut self, id: &str) {
        self.open_profile_in(id, self.active_side, true);
    }

    /// Open a saved profile as a new tab in panel `side`; with `reuse`, a tab
    /// already showing it is switched to instead.
    pub(super) fn open_profile_in(&mut self, id: &str, side: usize, reuse: bool) {
        let existing = self.sides.iter().enumerate().find_map(|(side, s)| {
            s.tabs
                .iter()
                .position(|t| {
                    t.pane.remote.as_ref().and_then(|r| r.target.profile.as_deref()) == Some(id)
                })
                .map(|i| (side, i))
        });
        if let Some((side, i)) = existing.filter(|_| reuse) {
            self.activate_tab(side, i);
            return;
        }
        let Some(p) = self.profiles.get(id).cloned() else {
            return;
        };
        let target = RemoteTarget {
            profile: Some(p.id.clone()),
            host: p.host.clone(),
            port: p.port,
            user: p.user.clone(),
            key_path: p.key_path.clone(),
        };
        let pane = offline_pane(
            target,
            Some(p.name.clone()),
            p.production,
            p.start_path.clone().unwrap_or_default(),
        );
        let color = p.color.as_deref().and_then(Rgb::from_hex);
        let at = self.push_tab(side, pane, color, None);
        self.active_side = side;
        self.connect(
            at,
            ConnectRequest {
                target: RemoteTarget {
                    profile: Some(p.id.clone()),
                    host: p.host.clone(),
                    port: p.port,
                    user: p.user.clone(),
                    key_path: p.key_path.clone(),
                },
                name: Some(p.name.clone()),
                production: p.production,
                color: p.color.as_deref().and_then(Rgb::from_hex),
                path: p.start_path.clone(),
                password: None,
                passphrase: None,
                accept_fingerprint: None,
                use_saved: p.save_password,
                store_password: false,
                interactive: false,
            },
        );
    }

    // ------------------------------------------------- keyboard-interactive

    /// Dialog for the oldest pending server question. Returns nothing; answers are
    /// sent back to the waiting connection task.
    pub(super) fn auth_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(f) = self.auth_prompts.first_mut() else { return };
        let mut done: Option<bool> = None; // Some(true) = send, Some(false) = cancel
        let title = tr!("Login – {target}", target = f.req.label.as_str());
        dialog::window("auth-prompt", title).icon(Role::Unlock).min_width(420.0).show(ctx, |ui| {
            if f.req.retry {
                dialog::warn(ui, tr!("The password was not accepted. Enter it again."), true);
            }
            if !f.req.name.trim().is_empty() {
                dialog::lead(ui, egui::RichText::new(f.req.name.trim()).strong());
            }
            if !f.req.instructions.trim().is_empty() {
                dialog::lead(ui, f.req.instructions.trim());
            }
            dialog::note(ui, tr!("The server asks (keyboard-interactive):"));
            dialog::form(ui, "auth-prompt-form", |ui| {
                for (i, (text, echo)) in f.req.prompts.iter().enumerate() {
                    ui.label(text.trim());
                    let r = crate::secret::edit(ui, &mut f.answers[i], !*echo, "", 300.0);
                    if i == 0 && f.focus {
                        r.request_focus();
                        f.focus = false;
                    }
                    ui.end_row();
                }
            });
            let b = dialog::buttons(ui, [dialog::btn(Role::Send, tr!("Send")).default(true), dialog::cancel()]);
            if b[0] {
                done = Some(true);
            } else if b[1] {
                done = Some(false);
            }
        });
        if let Some(send) = done {
            let mut f = self.auth_prompts.remove(0);
            let answers = send.then(|| {
                f.answers.iter_mut().map(std::mem::take).collect()
            });
            if let Some(reply) = f.reply.take() {
                let _ = reply.send(answers);
            }
        }
    }

    // ---------------------------------------------------------- site manager

    pub(super) fn site_manager_ui(&mut self, ctx: &egui::Context) {
        let mut act: Option<Action> = None;
        let mut close = false;
        let profiles = &self.profiles;
        let Dialog::Connect(f) = &mut self.dialog else {
            return;
        };

        let win = dialog::window("connect", tr!("Server manager")).icon(Role::Connect).min_width(720.0).max_width(760.0);
        win.danger(f.production).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                // ---- left: saved profiles
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    if ui.button(tr!("＋ New / quick connect")).clicked() {
                        f.clear_for_new();
                    }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .show(ui, |ui| {
                            if profiles.list.is_empty() {
                                dialog::note(ui, tr!("No saved servers yet."));
                            }
                            for p in &profiles.list {
                                let c = p
                                    .color
                                    .as_deref()
                                    .and_then(Rgb::from_hex)
                                    .map(rgb32)
                                    .unwrap_or(ui.visuals().widgets.inactive.bg_fill);
                                let sel = f.selected.as_deref() == Some(p.id.as_str());
                                let label =
                                    format!("{}{}", if p.production { "⚠ " } else { "" }, p.name);
                                let b =
                                    egui::Button::new(egui::RichText::new(label).color(text_on(c)))
                                        .fill(c)
                                        .stroke(if sel {
                                            Stroke::new(2.0, Color32::BLACK)
                                        } else {
                                            Stroke::NONE
                                        })
                                        .min_size(Vec2::new(210.0, 26.0));
                                let r = ui
                                    .add(b)
                                    .on_hover_text(format!("{}@{}:{}", p.user, p.host, p.port));
                                if r.clicked() {
                                    f.load_profile(p);
                                }
                                if r.double_clicked() {
                                    f.load_profile(p);
                                    act = Some(Box::new(|a: &mut App| a.submit_connect()));
                                }
                            }
                        });
                    // Deleting the selected profile (asks once more).
                    if let Some(id) = f.selected.clone() {
                        ui.add_space(4.0);
                        if f.confirm_delete {
                            if crate::look::btn(Role::Delete, egui::RichText::new(tr!("Really delete?")).color(crate::look::RED).strong()).ui(ui).clicked() {
                                act = Some(Box::new(move |a: &mut App| a.delete_profile(id)));
                            }
                        } else if crate::look::btn(Role::Delete, tr!("Delete")).ui(ui).clicked() {
                            f.confirm_delete = true;
                        }
                    }
                });
                ui.add_sized([9.0, 380.0], egui::Separator::default().vertical());
                // ---- right: form
                ui.vertical(|ui| {
                    ui.add_enabled_ui(!f.busy, |ui| {
                        dialog::form(ui, "cf", |ui| {
                            ui.label(tr!("Name:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut f.name)
                                    .hint_text(tr!("e.g. Web production"))
                                    .desired_width(300.0),
                            );
                            ui.end_row();
                            ui.label(tr!("Server:"));
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut f.host)
                                    .hint_text(tr!("host or user@host:port"))
                                    .desired_width(300.0),
                            );
                            if f.focus {
                                r.request_focus();
                                f.focus = false;
                            }
                            ui.end_row();
                            ui.label(tr!("Port:"));
                            ui.add(egui::TextEdit::singleline(&mut f.port).desired_width(60.0));
                            ui.end_row();
                            ui.label(tr!("User:"));
                            ui.add(egui::TextEdit::singleline(&mut f.user).desired_width(300.0));
                            ui.end_row();
                            ui.label(tr!("Password:"));
                            let hint = match (f.password_saved, f.saved_in_keyring) {
                                (true, false) => tr!("saved (protected by the master password)"),
                                (true, true) => tr!("saved in the system keyring (older version)"),
                                _ => tr!("optional"),
                            };
                            crate::secret::edit(ui, &mut f.password, true, &hint, 300.0);
                            ui.end_row();
                            ui.label(tr!("Key:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut f.key_path)
                                    .hint_text(tr!("default: agent, ~/.ssh/id_*"))
                                    .desired_width(300.0),
                            );
                            ui.end_row();
                            ui.label(tr!("Key passphrase:"));
                            crate::secret::edit(ui, &mut f.passphrase, true, &tr!("optional, not saved"), 300.0);
                            ui.end_row();
                            ui.label(tr!("Start folder:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut f.start_path)
                                    .hint_text(tr!("home"))
                                    .desired_width(300.0),
                            );
                            ui.end_row();
                            ui.label(tr!("Tab color:"));
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                                let swatch =
                                    |ui: &mut egui::Ui,
                                     color: &mut Option<Rgb>,
                                     c: Rgb,
                                     name: &'static str| {
                                        let b = egui::Button::new("")
                                            .fill(rgb32(c))
                                            .min_size(Vec2::splat(20.0))
                                            .stroke(if *color == Some(c) {
                                                Stroke::new(2.0, Color32::BLACK)
                                            } else {
                                                Stroke::new(1.0, Color32::GRAY)
                                            });
                                        if ui.add(b).on_hover_text(rolid_i18n::t_str(name)).clicked() {
                                            *color = Some(c);
                                        }
                                    };
                                ui.horizontal(|ui| {
                                    let none = egui::Button::new("–")
                                        .min_size(Vec2::splat(20.0))
                                        .stroke(if f.color.is_none() {
                                            Stroke::new(2.0, Color32::BLACK)
                                        } else {
                                            Stroke::new(1.0, Color32::GRAY)
                                        });
                                    if ui
                                        .add(none)
                                        .on_hover_text(tr!("Do not change the tab color"))
                                        .clicked()
                                    {
                                        f.color = None;
                                    }
                                    for (name, c) in &PASTEL_PRESETS[..8] {
                                        swatch(ui, &mut f.color, *c, name);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    let cur = f.color.unwrap_or(Rgb(0xFF, 0xFF, 0xFF));
                                    let mut rgb = [cur.0, cur.1, cur.2];
                                    if ui
                                        .color_edit_button_srgb(&mut rgb)
                                        .on_hover_text(tr!("Any color (16M shades)"))
                                        .changed()
                                    {
                                        f.color = Some(Rgb(rgb[0], rgb[1], rgb[2]));
                                    }
                                    for (name, c) in &PASTEL_PRESETS[8..] {
                                        swatch(ui, &mut f.color, *c, name);
                                    }
                                });
                            });
                            ui.end_row();
                        });
                    });
                });
            });
            ui.add_enabled_ui(!f.busy, |ui| {
                let heading = tr!("Options");
                dialog::group(ui, &heading, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut f.save_password, tr!("Save password (protected by the master password)"));
                        dialog::help(
                            ui,
                            tr!(
                                "The password is saved in an encrypted file. The first time you create \
                                 a master password; after the application starts, you enter it once when \
                                 a saved password is first used."
                            ),
                        );
                    });
                    ui.checkbox(&mut f.production, tr!("⚠ Production server (red stripe, stricter confirmations)"));
                });
            });

            if let Some((alg, fp)) = &f.unknown_host {
                dialog::warn(ui, tr!("Unknown server (not in ~/.ssh/known_hosts)."), false);
                ui.label(tr!("Key type: {algorithm}", algorithm = alg));
                ui.monospace(fp.as_str());
                let trust = ui
                    .horizontal(|ui| {
                        let r = crate::look::btn(Role::Trust, tr!("Trust and connect")).ui(ui).clicked();
                        dialog::help(
                            ui,
                            tr!("Check the fingerprint with the server administrator. Once confirmed, it is saved to known_hosts."),
                        );
                        r
                    })
                    .inner;
                if trust {
                    f.accept_fingerprint = Some(fp.clone());
                    f.unknown_host = None;
                    act = Some(Box::new(|a: &mut App| a.submit_connect()));
                }
            }
            if let Some(e) = &f.error {
                dialog::warn(ui, e.as_str(), true);
            }
            if let Some(i) = &f.info {
                ui.colored_label(Color32::from_rgb(20, 120, 40), i.as_str());
            }
            if f.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr!("Connecting…"));
                });
            }
            let b = dialog::buttons(ui, [
                dialog::btn(Role::Connect, tr!("Connect")).default(true).enabled(!f.busy && f.unknown_host.is_none()),
                dialog::btn(Role::Save, tr!("Save profile")),
                dialog::close(),
            ]);
            if b[0] && act.is_none() {
                act = Some(Box::new(|a: &mut App| a.submit_connect()));
            }
            if b[1] {
                act = Some(Box::new(|a: &mut App| a.save_profile_from_form()));
            }
            close |= b[2];
        });
        if close {
            f.wipe_secrets();
            self.dialog = Dialog::None;
        }
        if let Some(a) = act {
            a(self);
        }
    }

    // ------------------------------------------------------------- workspace

    pub(super) fn workspace_state(&self) -> Workspace {
        let side_state = |s: &super::Side| SideState {
            active: s.active,
            tabs: s
                .tabs
                .iter()
                .map(|t| PanelTab {
                    name: t.name.clone(),
                    color: t.color.to_hex(),
                    // Inside an archive: the folder containing it is remembered.
                    location: t.pane.location(),
                    view: (t.pane.style != rolid_config::ViewStyle::Full).then_some(t.pane.style),
                })
                .collect(),
        };
        Workspace {
            active_side: self.active_side,
            left: side_state(&self.sides[0]),
            right: side_state(&self.sides[1]),
            favorites: self.places.favorites.clone(),
            recent: self.places.recent.clone(),
            targets: self.places.targets.clone(),
            legacy_tabs: Vec::new(),
        }
    }

    pub(super) fn workspace_dirty(&self) -> bool {
        rolid_config::workspace_fingerprint(&self.workspace_state()) != self.saved_workspace
    }

    pub(super) fn save_workspace_if_changed(&mut self) {
        let ws = self.workspace_state();
        let fp = rolid_config::workspace_fingerprint(&ws);
        if fp != self.saved_workspace {
            match rolid_config::save_workspace(&ws) {
                Ok(()) => self.saved_workspace = fp,
                Err(e) => self.last_error = Some(tr!("Cannot save the workspace: {error}", error = e)),
            }
        }
    }

    /// Recreate tabs from the saved workspace. Remote panels start disconnected and
    /// connect when their tab is shown.
    pub(super) fn restore_workspace(&mut self, ws: Workspace) {
        self.places = rolid_config::Places { favorites: ws.favorites, recent: ws.recent, targets: ws.targets };
        for (side, st) in [ws.left, ws.right].into_iter().enumerate() {
            for t in st.tabs {
                let pane = match t.location {
                    Location::Local { path } => {
                        let path = if std::path::Path::new(&path).is_dir() {
                            path
                        } else {
                            App::home_dir()
                        };
                        Pane::new(Arc::new(LocalFs), path)
                    }
                    Location::Remote { target, path } => {
                        let prof = target.profile.as_ref().and_then(|i| self.profiles.get(i));
                        let name = prof.map(|p| p.name.clone());
                        let production = prof.is_some_and(|p| p.production);
                        offline_pane(target, name, production, path)
                    }
                };
                let mut pane = pane;
                pane.style = t.view.unwrap_or_default();
                let color = Rgb::from_hex(&t.color).unwrap_or(PASTEL_PRESETS[0].1);
                self.push_tab(side, pane, Some(color), t.name);
            }
            let n = self.sides[side].tabs.len();
            if n > 0 {
                self.sides[side].active = st.active.min(n - 1);
            }
        }
        self.active_side = ws.active_side.min(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_password_is_used_unchanged() {
        let f = ConnectForm {
            password: Zeroizing::new("  Žluťoučký kůň пароль \t".into()),
            ..Default::default()
        };
        let (pw, keyring) = f.password_source(true);
        assert_eq!(pw.as_deref().map(String::as_str), Some("  Žluťoučký kůň пароль \t"));
        assert!(!keyring, "a typed password wins over the keyring");
    }

    #[test]
    fn empty_field_uses_the_keyring_only_when_saved() {
        let f = ConnectForm::default();
        assert!(f.password_source(true).1);
        assert!(f.password_source(true).0.is_none());
        assert_eq!(f.password_source(false), (None, false));
    }
}

