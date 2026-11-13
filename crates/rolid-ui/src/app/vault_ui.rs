//! Master password: the encrypted password vault in the UI – create, unlock, lock
//! (manually or after inactivity), change, reset and migration from the keyring.
//!
//! Anything that needs a saved password calls [`App::with_vault`]; the action runs at
//! once when the vault is unlocked, otherwise after the user unlocks (or creates) it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use rolid_config::strength::{self, Strength};
use rolid_config::vault::{self, KdfParams, MigrationReport, Vault, VaultError};
use rolid_config::PasswordStore;
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use super::{Action, App, Msg};
use crate::dialog;
use crate::look::Role;

/// Target time for deriving the key (Argon2id) on this machine.
const KDF_TARGET: Duration = Duration::from_millis(500);
/// Choices for automatic locking (seconds after the last use); `None` =
/// only when the app closes.
pub const LOCK_CHOICES: [Option<u32>; 6] = [Some(30), Some(60), Some(300), Some(900), Some(3600), None];

const RED: Color32 = Color32::from_rgb(190, 20, 20);
const GREEN: Color32 = Color32::from_rgb(20, 120, 40);
const ORANGE: Color32 = Color32::from_rgb(170, 90, 0);

pub fn lock_label(secs: Option<u32>) -> String {
    match secs {
        None => tr!("Never (only when the application closes)"),
        Some(s) if s < 60 || s % 60 != 0 => trn!("{n} second after the last use", "{n} seconds after the last use", s),
        Some(s) => trn!("{n} minute after the last use", "{n} minutes after the last use", s / 60),
    }
}

/// How long until the vault locks: `after` seconds from its last use
/// (`None` = it does not lock by itself).
pub fn lock_left(after: Option<u32>, last_use: Instant, now: Instant) -> Option<Duration> {
    let limit = Duration::from_secs(after? as u64);
    Some(limit.saturating_sub(now.saturating_duration_since(last_use)))
}

/// "1:05" (minutes and seconds, rounded up).
pub fn countdown(left: Duration) -> String {
    let s = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    format!("{}:{:02}", s / 60, s % 60)
}

/// An action waiting for the vault, with what to do if the user cancels.
struct Pending {
    ok: Action,
    cancel: Option<Action>,
}

pub enum VaultDialog {
    Create {
        pw1: Zeroizing<String>,
        pw2: Zeroizing<String>,
        strength: Strength,
        ack: bool,
        reason: String,
        error: Option<String>,
        focus: bool,
    },
    Unlock {
        pw: Zeroizing<String>,
        reason: String,
        error: Option<String>,
        focus: bool,
    },
    Change {
        old: Zeroizing<String>,
        new1: Zeroizing<String>,
        new2: Zeroizing<String>,
        strength: Strength,
        error: Option<String>,
        focus: bool,
    },
    /// Confirm deleting the vault ("forgot the master password").
    Reset { back_to_unlock: bool },
    /// Offer moving passwords from the keyring (0.3/0.4).
    Migrate { count: usize },
    Info { title: String, text: String, error: bool },
}

pub enum VaultMsg {
    Created(Result<Vault, VaultError>),
    Unlocked(Result<Vault, VaultError>),
    Changed(Result<Vault, VaultError>),
    Migrated { vault: Vault, result: Result<MigrationReport, VaultError> },
}

pub struct VaultState {
    pub path: PathBuf,
    pub open: Option<Vault>,
    pub dialog: Option<VaultDialog>,
    pending: Vec<Pending>,
    /// Key derivation / migration running in the background.
    pub busy: bool,
    failures: u32,
    retry_at: Option<Instant>,
    /// When the vault was last used (unlocked, a password read or saved).
    last_use: std::cell::Cell<Instant>,
    /// Migration was offered in this session already.
    offered: bool,
    /// Enter / Esc pressed this frame (taken before other dialogs see them).
    keys: (bool, bool),
}

impl VaultState {
    pub fn new() -> Self {
        Self {
            path: Vault::default_path(),
            open: None,
            dialog: None,
            pending: Vec::new(),
            busy: false,
            failures: 0,
            retry_at: None,
            last_use: std::cell::Cell::new(Instant::now()),
            offered: false,
            keys: (false, false),
        }
    }

    pub fn exists(&self) -> bool {
        Vault::exists(&self.path)
    }

    /// The vault was used: the time to locking starts again.
    pub fn touch(&self) {
        self.last_use.set(Instant::now());
    }

    pub fn blocks_input(&self) -> bool {
        self.dialog.is_some()
    }

    fn wait_left(&self) -> Option<Duration> {
        self.retry_at
            .and_then(|t| t.checked_duration_since(Instant::now()))
            .filter(|d| !d.is_zero())
    }
}

fn user_inputs() -> Vec<String> {
    vec![super::session::default_user()]
}

fn check(pw: &str) -> Strength {
    let inputs = user_inputs();
    let refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
    strength::check(pw, &refs)
}

fn secret(s: &Zeroizing<String>) -> SecretString {
    SecretString::from(s.as_str())
}

/// Strength meter and the reasons for rejection.
fn strength_ui(ui: &mut egui::Ui, pw: &str, s: &Strength) {
    if pw.is_empty() {
        ui.weak(tr!(
            "At least {chars} characters, {upper} capital letters and a digit or a character other than a letter.",
            chars = strength::MIN_CHARS,
            upper = strength::MIN_UPPER
        ));
        return;
    }
    let color = match (s.ok(), s.score) {
        (true, 4) => GREEN,
        (true, _) => Color32::from_rgb(70, 140, 40),
        (false, 0 | 1) => RED,
        (false, _) => ORANGE,
    };
    let frac = if s.ok() { (s.score as f32 + 1.0) / 5.0 } else { (s.score.min(2) as f32 + 1.0) / 5.0 };
    ui.add(
        egui::ProgressBar::new(frac)
            .desired_width(300.0)
            .fill(color)
            .text(RichText::new(tr!("Strength: {label}", label = rolid_i18n::tc_str("password", s.label()))).color(Color32::WHITE)),
    );
    for r in &s.reasons {
        ui.colored_label(RED, format!("• {}", rolid_i18n::t(r)));
    }
}

fn pw_field(ui: &mut egui::Ui, s: &mut Zeroizing<String>, hint: &str, focus: &mut bool) -> egui::Response {
    let r = crate::secret::edit(ui, s, true, hint, 300.0);
    if *focus {
        r.request_focus();
        *focus = false;
    }
    r
}

impl App {
    // ------------------------------------------------------------ public API

    /// Run `ok` with the vault unlocked: now, or after the user unlocks / creates it.
    /// `cancel` runs if the user closes the dialog instead.
    pub(super) fn with_vault(&mut self, reason: String, ok: Action, cancel: Option<Action>) {
        if self.vault.open.is_some() && !self.vault.busy {
            self.vault.touch();
            ok(self);
            return;
        }
        self.vault.pending.push(Pending { ok, cancel });
        let asking = matches!(
            self.vault.dialog,
            Some(VaultDialog::Unlock { .. } | VaultDialog::Create { .. } | VaultDialog::Change { .. })
        );
        if self.vault.busy || asking {
            return; // runs when the current unlock / migration finishes
        }
        self.vault.dialog = Some(if self.vault.exists() {
            VaultDialog::Unlock { pw: Default::default(), reason, error: None, focus: true }
        } else {
            new_create(reason)
        });
    }

    /// The saved password of a profile, if the vault is unlocked.
    pub(super) fn vault_password(&self, profile_id: &str) -> Option<Zeroizing<String>> {
        let v = self.vault.open.as_ref()?;
        self.vault.touch();
        v.get(profile_id).map(|s| Zeroizing::new(s.expose_secret().to_owned()))
    }

    /// Save a server password into the vault (asks for the master password if needed).
    pub(super) fn store_password(&mut self, id: String, pw: Zeroizing<String>) {
        let cancel_id = id.clone();
        let name = self.profiles.get(&id).map(|p| p.name.clone()).unwrap_or_default();
        self.with_vault(
            tr!("The password for “{name}” will be saved protected by the master password.", name = name),
            Box::new(move |a: &mut App| {
                let Some(v) = a.vault.open.as_mut() else { return };
                v.set(&id, SecretString::from(pw.as_str()));
                if let Err(e) = v.save() {
                    a.last_error = Some(tr!("Cannot save the password: {error}", error = e.text()));
                    return;
                }
                tracing::info!("profile password saved to the vault");
                if let Some(p) = a.profiles.list.iter_mut().find(|p| p.id == id) {
                    let was_keyring = p.password_in_keyring();
                    p.save_password = true;
                    p.password_store = PasswordStore::Vault;
                    if was_keyring {
                        a.delete_from_keyring(id.clone());
                    }
                }
                a.save_profiles_or_report();
                a.site_manager_password_saved(&id);
            }),
            Some(Box::new(move |a: &mut App| a.password_not_saved(&cancel_id))),
        );
    }

    /// Forget the saved password of a profile.
    pub(super) fn forget_password(&mut self, id: &str) {
        if let Some(v) = self.vault.open.as_mut() {
            if v.remove(id) {
                if let Err(e) = v.save() {
                    self.last_error = Some(rolid_i18n::t(&e.text()));
                }
            }
        } // locked: the stale entry is removed on the next unlock
    }

    /// Lock the vault; the copies of saved passwords kept for reconnecting
    /// are wiped too (password connections then wait for "Unlock").
    pub(super) fn lock_vault(&mut self, why: &str) {
        if self.vault.open.take().is_some() {
            let n = self.links.wipe_passwords();
            tracing::info!("password vault locked ({why}); {n} reconnect password copies wiped");
        }
    }

    pub(super) fn set_vault_lock(&mut self, secs: Option<u32>) {
        self.settings.set_vault_lock_after(secs);
        self.save_settings();
        self.vault.touch();
    }

    /// Time left until the vault locks itself (`None` = locked, or no limit).
    pub(super) fn vault_lock_left(&self) -> Option<Duration> {
        self.vault.open.as_ref()?;
        lock_left(self.settings.vault_lock_after(), self.vault.last_use.get(), Instant::now())
    }

    pub(super) fn open_vault_dialog(&mut self, d: VaultDialog) {
        if self.vault.dialog.is_none() && !self.vault.busy {
            self.vault.dialog = Some(d);
        }
    }

    pub(super) fn ask_unlock(&mut self) {
        let reason = tr!("Unlocks the saved server passwords.");
        self.with_vault(reason, Box::new(|_| {}), None);
    }

    pub(super) fn ask_create(&mut self) {
        self.open_vault_dialog(new_create(tr!("The master password protects the saved server passwords.")));
    }

    pub(super) fn ask_change(&mut self) {
        self.open_vault_dialog(VaultDialog::Change {
            old: Default::default(),
            new1: Default::default(),
            new2: Default::default(),
            strength: Strength::default(),
            error: None,
            focus: true,
        });
    }

    pub(super) fn ask_reset(&mut self) {
        self.open_vault_dialog(VaultDialog::Reset { back_to_unlock: false });
    }

    pub(super) fn legacy_profile_ids(&self) -> Vec<String> {
        self.profiles.list.iter().filter(|p| p.password_in_keyring()).map(|p| p.id.clone()).collect()
    }

    pub(super) fn ask_migrate(&mut self) {
        let count = self.legacy_profile_ids().len();
        if count > 0 {
            self.open_vault_dialog(VaultDialog::Migrate { count });
        }
    }

    // ------------------------------------------------------- per-frame hooks

    /// Before the other dialogs: lock the vault the set time after its last
    /// use, take Enter/Esc for the vault dialog, offer the keyring migration once.
    pub(super) fn vault_frame_start(&mut self, ctx: &egui::Context) {
        if let Some(left) = self.vault_lock_left() {
            if left.is_zero() && self.vault.dialog.is_none() && !self.vault.busy {
                let secs = self.settings.vault_lock_after().unwrap_or(0);
                self.lock_vault(&format!("{secs} s after the last use"));
            } else {
                // The countdown on the status bar button changes every second.
                ctx.request_repaint_after(left.min(Duration::from_secs(1)));
            }
        }
        self.vault.keys = if self.vault.dialog.is_some() {
            ctx.input_mut(|i| {
                (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape))
            })
        } else {
            (false, false)
        };
        // Passwords from 0.3/0.4 in the keyring: offer to move them, at most once a day.
        if !self.vault.offered && self.vault.dialog.is_none() && !self.vault.busy {
            self.vault.offered = true;
            let declined_today = self.settings.migration_declined_day == Some(rolid_config::today());
            if !declined_today {
                self.ask_migrate();
            }
        }
        if let Some(left) = self.vault.wait_left() {
            ctx.request_repaint_after(left.min(Duration::from_millis(250)));
        }
    }

    pub(super) fn on_vault_msg(&mut self, m: VaultMsg) {
        self.vault.busy = false;
        match m {
            VaultMsg::Created(r) | VaultMsg::Unlocked(r) | VaultMsg::Changed(r) => match r {
                Ok(v) => {
                    let changed = matches!(self.vault.dialog, Some(VaultDialog::Change { .. }));
                    tracing::info!("password vault unlocked ({} saved passwords)", v.len());
                    self.vault.open = Some(v);
                    self.vault.failures = 0;
                    self.vault.retry_at = None;
                    self.vault.touch();
                    self.close_vault_dialog();
                    self.prune_vault();
                    if changed {
                        self.vault.dialog = Some(info(
                            tr!("Master password changed"),
                            tr!("The saved passwords are now protected by the new master password."),
                            false,
                        ));
                    }
                    self.run_pending();
                }
                Err(VaultError::WrongPassword) => {
                    self.vault.failures += 1;
                    let wait = vault::backoff(self.vault.failures);
                    self.vault.retry_at = Some(Instant::now() + wait);
                    tracing::info!("wrong master password (failed attempt no. {})", self.vault.failures);
                    let msg = tr!(
                        "Wrong master password. Next attempt in {secs} s. The passwords are safe – you can try as many times as you like.",
                        secs = wait.as_secs()
                    );
                    self.set_vault_error(msg);
                }
                Err(e) => {
                    tracing::warn!("password vault: {e}");
                    self.set_vault_error(rolid_i18n::t(&e.text()));
                }
            },
            VaultMsg::Migrated { vault, result } => {
                self.vault.open = Some(vault);
                match result {
                    Ok(r) => self.apply_migration(r),
                    Err(e) => {
                        self.vault.dialog = Some(info(
                            tr!("Moving the passwords failed"),
                            tr!("{error}\n\nThe passwords stayed in the system keyring and work as before.", error = e.text()),
                            true,
                        ));
                    }
                }
                self.run_pending();
            }
        }
    }

    // --------------------------------------------------------------- helpers

    fn set_vault_error(&mut self, msg: String) {
        match &mut self.vault.dialog {
            Some(
                VaultDialog::Unlock { error, focus, .. }
                | VaultDialog::Create { error, focus, .. }
                | VaultDialog::Change { error, focus, .. },
            ) => {
                *error = Some(msg);
                *focus = true;
            }
            _ => self.last_error = Some(msg),
        }
    }

    fn close_vault_dialog(&mut self) {
        self.vault.dialog = None;
        // Drop egui's copies of what was typed (undo history of the text fields).
        self.ctx.data_mut(|d| d.remove_by_type::<egui::text_edit::TextEditState>());
    }

    fn run_pending(&mut self) {
        if self.vault.open.is_none() {
            return;
        }
        self.vault.touch();
        for p in std::mem::take(&mut self.vault.pending) {
            (p.ok)(self);
        }
    }

    fn cancel_pending(&mut self) {
        for p in std::mem::take(&mut self.vault.pending) {
            if let Some(c) = p.cancel {
                c(self);
            }
        }
    }

    /// Remove vault entries of deleted profiles or profiles that no longer save a password.
    fn prune_vault(&mut self) {
        let keep: Vec<String> =
            self.profiles.list.iter().filter(|p| p.password_in_vault()).map(|p| p.id.clone()).collect();
        if let Some(v) = self.vault.open.as_mut() {
            if v.retain(|id| keep.iter().any(|k| k == id)) {
                if let Err(e) = v.save() {
                    self.last_error = Some(rolid_i18n::t(&e.text()));
                }
            }
        }
    }

    pub(super) fn save_profiles_or_report(&mut self) {
        if let Err(e) = rolid_config::save_profiles(&self.profiles) {
            self.last_error = Some(tr!("Cannot save the profiles: {error}", error = e.to_string()));
        }
    }

    pub(super) fn delete_from_keyring(&self, id: String) {
        self.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || rolid_config::secrets::delete(&id)).await;
            Msg::Note(None)
        });
    }

    fn spawn_vault<F>(&mut self, f: F)
    where
        F: FnOnce() -> VaultMsg + Send + 'static,
    {
        self.vault.busy = true;
        self.spawn(async move {
            let m = tokio::task::spawn_blocking(f)
                .await
                .unwrap_or_else(|e| VaultMsg::Unlocked(Err(VaultError::Io(e.to_string()))));
            Msg::Vault(Box::new(m))
        });
    }

    fn start_migration(&mut self) {
        let Some(mut v) = self.vault.open.take() else { return };
        let ids = self.legacy_profile_ids();
        self.spawn_vault(move || {
            let result = vault::migrate_from_keyring(&mut v, &ids, &vault::SystemKeyring);
            VaultMsg::Migrated { vault: v, result }
        });
    }

    fn apply_migration(&mut self, r: MigrationReport) {
        for p in self.profiles.list.iter_mut() {
            if r.moved.contains(&p.id) {
                p.password_store = PasswordStore::Vault;
            } else if r.missing.contains(&p.id) {
                p.password_store = PasswordStore::Vault;
                p.save_password = false;
            }
        }
        self.save_profiles_or_report();
        tracing::info!(
            "move from the keyring: moved {}, not found {}, failed {}",
            r.moved.len(),
            r.missing.len(),
            r.failed.len()
        );
        let mut text = tr!("Passwords moved: {count}.", count = r.moved.len());
        if !r.missing.is_empty() {
            text.push('\n');
            text += &trn!(
                "{n} profile had no password in the keyring (you will enter it when connecting).",
                "{n} profiles had no password in the keyring (you will enter it when connecting).",
                r.missing.len()
            );
        }
        if !r.failed.is_empty() {
            text.push('\n');
            text += &trn!(
                "{n} password could not be read – it stayed in the keyring: {errors}",
                "{n} passwords could not be read – they stayed in the keyring: {errors}",
                r.failed.len(),
                errors = r.failed.iter().map(|(_, e)| e.as_str()).collect::<Vec<_>>().join("; ")
            );
        }
        if !r.not_deleted.is_empty() {
            text.push('\n');
            text += &trn!(
                "{n} password is in the new storage already, but could not be deleted from the keyring – delete it by hand (service “rolidscp”).",
                "{n} passwords are in the new storage already, but could not be deleted from the keyring – delete them by hand (service “rolidscp”).",
                r.not_deleted.len()
            );
        }
        self.vault.dialog = Some(info(tr!("Passwords moved from the keyring"), text, !r.failed.is_empty()));
    }

    fn reset_vault(&mut self) {
        if let Err(e) = Vault::delete_file(&self.vault.path) {
            self.vault.dialog =
                Some(info(tr!("Deleting failed"), rolid_i18n::t(&e.text()), true));
            return;
        }
        self.vault.open = None;
        self.vault.failures = 0;
        self.vault.retry_at = None;
        let mut n = 0;
        for p in self.profiles.list.iter_mut().filter(|p| p.password_in_vault()) {
            p.save_password = false;
            n += 1;
        }
        self.save_profiles_or_report();
        tracing::info!("password vault deleted by the user ({n} profiles without a password)");
        self.cancel_pending();
        self.vault.dialog = Some(info(
            tr!("Saved passwords deleted"),
            trn!(
                "The password file is deleted, {n} profile is left without a saved password. \
                 The next time you save a password, you will create a new master password.",
                "The password file is deleted, {n} profiles are left without a saved password. \
                 The next time you save a password, you will create a new master password.",
                n
            ),
            false,
        ));
    }

    // -------------------------------------------------------------------- UI

    pub(super) fn vault_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vault.dialog.take() else { return };
        let (enter, esc) = self.vault.keys;
        let busy = self.vault.busy;
        let wait = self.vault.wait_left();
        let mut keep = true;
        let mut act: Option<Action> = None;

        // Enter and Esc were taken for this dialog in `vault_frame_start`
        // (it is on top of the others), so they come in as `enter` / `esc`;
        // the buttons still handle clicks and the ✕.
        let (title, role, danger) = match &d {
            VaultDialog::Create { .. } => (tr!("🔑 Create master password"), Role::Lock, false),
            VaultDialog::Unlock { .. } => (tr!("🔒 Unlock saved passwords"), Role::Unlock, false),
            VaultDialog::Change { .. } => (tr!("🔑 Change master password"), Role::Lock, false),
            VaultDialog::Reset { .. } => (tr!("Delete the saved passwords and start again?"), Role::Delete, true),
            VaultDialog::Migrate { .. } => (tr!("Move the passwords from the system keyring?"), Role::Lock, false),
            VaultDialog::Info { title, error, .. } => (title.clone(), Role::Info, *error),
        };
        let busy_line = |ui: &mut egui::Ui, text: String| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(text);
            });
        };
        dialog::window("vault-dialog", title).icon(role).danger(danger).min_width(460.0).max_width(520.0).show(ctx, |ui| {
            match &mut d {
                VaultDialog::Create { pw1, pw2, strength: s, ack, reason, error, focus } => {
                    dialog::lead(ui, reason.as_str());
                    let heading = tr!("Master password");
                    dialog::group(ui, &heading, |ui| {
                        dialog::form(ui, "vault-create", |ui| {
                            ui.label(tr!("Master password:"));
                            if pw_field(ui, pw1, "", focus).changed() {
                                *s = check(pw1);
                            }
                            ui.end_row();
                            ui.label("");
                            ui.vertical(|ui| strength_ui(ui, pw1, s));
                            ui.end_row();
                            ui.label(tr!("Master password again:"));
                            let mut f = false;
                            pw_field(ui, pw2, "", &mut f);
                            ui.end_row();
                        });
                        if !pw2.is_empty() && pw1.as_str() != pw2.as_str() {
                            dialog::warn(ui, tr!("The passwords do not match."), true);
                        }
                    });
                    dialog::warn(
                        ui,
                        tr!(
                            "A forgotten master password cannot be recovered. It is not stored anywhere and nobody \
                             can find it out or reset it. If you forget it, the saved server passwords \
                             will be lost (the server profiles stay, you will enter the passwords again)."
                        ),
                        false,
                    );
                    ui.checkbox(ack, tr!("I understand that the master password cannot be recovered."));
                    if let Some(e) = error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    let ready = s.ok() && pw1.as_str() == pw2.as_str() && *ack && !busy;
                    if busy {
                        busy_line(ui, tr!("Creating…"));
                    }
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Ok, tr!("Create")).default(true).enabled(ready),
                        dialog::cancel().enabled(!busy),
                    ]);
                    if b[0] || (enter && ready) {
                        let m = secret(pw1);
                        let path = self.vault.path.clone();
                        act = Some(Box::new(move |a: &mut App| {
                            a.spawn_vault(move || {
                                let params = KdfParams::calibrate(KDF_TARGET);
                                VaultMsg::Created(Vault::create(&path, &m, params))
                            })
                        }));
                    }
                    if (b[1] || esc) && !busy {
                        keep = false;
                    }
                }
                VaultDialog::Unlock { pw, reason, error, focus } => {
                    dialog::lead(ui, reason.as_str());
                    dialog::form(ui, "vault-unlock", |ui| {
                        ui.label(tr!("Master password:"));
                        pw_field(ui, pw, "", focus);
                        ui.end_row();
                    });
                    if let Some(e) = error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    if ui.add_enabled(!busy, egui::Link::new(tr!("I forgot the master password…"))).clicked() {
                        act = Some(Box::new(|a: &mut App| {
                            a.vault.dialog = Some(VaultDialog::Reset { back_to_unlock: true })
                        }));
                    }
                    let ready = !busy && wait.is_none() && !pw.is_empty();
                    if busy {
                        busy_line(ui, tr!("Verifying…"));
                    } else if let Some(w) = wait {
                        dialog::note(ui, tr!("Wait {secs} s…", secs = w.as_secs() + 1));
                    }
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Unlock, tr!("Unlock")).default(true).enabled(ready),
                        dialog::cancel().enabled(!busy),
                    ]);
                    if b[0] || (enter && ready) {
                        let m = secret(pw);
                        crate::secret::wipe(pw);
                        let path = self.vault.path.clone();
                        act = Some(Box::new(move |a: &mut App| {
                            a.spawn_vault(move || VaultMsg::Unlocked(Vault::open(&path, &m)))
                        }));
                    }
                    if (b[1] || esc) && !busy {
                        keep = false;
                    }
                }
                VaultDialog::Change { old, new1, new2, strength: s, error, focus } => {
                    dialog::form(ui, "vault-change", |ui| {
                        ui.label(tr!("Current master password:"));
                        pw_field(ui, old, "", focus);
                        ui.end_row();
                        ui.label(tr!("New master password:"));
                        let mut f = false;
                        if pw_field(ui, new1, "", &mut f).changed() {
                            *s = check(new1);
                        }
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| strength_ui(ui, new1, s));
                        ui.end_row();
                        ui.label(tr!("New master password again:"));
                        pw_field(ui, new2, "", &mut f);
                        ui.end_row();
                    });
                    if !new2.is_empty() && new1.as_str() != new2.as_str() {
                        dialog::warn(ui, tr!("The new passwords do not match."), true);
                    }
                    dialog::note(ui, tr!("A forgotten master password cannot be recovered."));
                    if let Some(e) = error {
                        dialog::warn(ui, e.as_str(), true);
                    }
                    let ready = !busy
                        && wait.is_none()
                        && !old.is_empty()
                        && s.ok()
                        && new1.as_str() == new2.as_str();
                    if busy {
                        busy_line(ui, tr!("Re-encrypting…"));
                    } else if let Some(w) = wait {
                        dialog::note(ui, tr!("Wait {secs} s…", secs = w.as_secs() + 1));
                    }
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Ok, tr!("Change")).default(true).enabled(ready),
                        dialog::cancel().enabled(!busy),
                    ]);
                    if b[0] || (enter && ready) {
                        let (o, n) = (secret(old), secret(new1));
                        crate::secret::wipe(old);
                        let path = self.vault.path.clone();
                        act = Some(Box::new(move |a: &mut App| {
                            a.spawn_vault(move || {
                                VaultMsg::Changed(Vault::open(&path, &o).and_then(|mut v| {
                                    v.change_password(&n, KdfParams::calibrate(KDF_TARGET))?;
                                    Ok(v)
                                }))
                            })
                        }));
                    }
                    if (b[1] || esc) && !busy {
                        keep = false;
                    }
                }
                VaultDialog::Reset { back_to_unlock } => {
                    dialog::lead(
                        ui,
                        tr!(
                            "A forgotten master password cannot be recovered. But you can delete the file with the saved \
                             passwords and start again: the server profiles stay, only without saved passwords – \
                             you will enter them again when connecting. Then you will create a new master password."
                        ),
                    );
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Delete, tr!("Delete saved passwords")),
                        dialog::btn(Role::Close, tr!("Back")),
                    ]);
                    if b[0] {
                        act = Some(Box::new(|a: &mut App| a.reset_vault()));
                    } else if b[1] || esc {
                        if *back_to_unlock {
                            act = Some(Box::new(|a: &mut App| {
                                let reason = tr!("Unlocks the saved server passwords.");
                                a.vault.dialog = Some(VaultDialog::Unlock {
                                    pw: Default::default(),
                                    reason,
                                    error: None,
                                    focus: true,
                                })
                            }));
                        } else {
                            keep = false;
                        }
                    }
                }
                VaultDialog::Migrate { count } => {
                    dialog::lead(
                        ui,
                        trn!(
                            "Older versions of RolidSCP saved {n} password in the system keyring. Passwords are now \
                             saved in an encrypted file protected by the master password.",
                            "Older versions of RolidSCP saved {n} passwords in the system keyring. Passwords are now \
                             saved in an encrypted file protected by the master password.",
                            *count
                        ),
                    );
                    ui.horizontal(|ui| {
                        dialog::note(
                            ui,
                            tr!("The passwords are deleted from the keyring once they are safely written and verified in the new file."),
                        );
                        dialog::help(
                            ui,
                            tr!("If you decline, the passwords stay in the keyring and everything works as before. You will be reminded at most once a day."),
                        );
                    });
                    let b = dialog::buttons(ui, [
                        dialog::btn(Role::Ok, tr!("Move")).default(true),
                        dialog::btn(Role::Close, tr!("Not now")),
                    ]);
                    if b[0] || enter {
                        act = Some(Box::new(|a: &mut App| {
                            a.with_vault(
                                tr!("The passwords from the keyring will be moved under the master password."),
                                Box::new(|a: &mut App| a.start_migration()),
                                None,
                            )
                        }));
                    } else if b[1] || esc {
                        act = Some(Box::new(|a: &mut App| {
                            a.settings.migration_declined_day = Some(rolid_config::today());
                            a.save_settings();
                        }));
                    }
                }
                VaultDialog::Info { text, error, .. } => {
                    if *error {
                        dialog::warn(ui, text.as_str(), true);
                    } else {
                        dialog::lead(ui, text.as_str());
                    }
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, tr!("OK")).default(true)]);
                    if b[0] || enter || esc || dialog::cancel_pressed(ui) {
                        keep = false;
                    }
                }
            }
        });

        let canceled = !keep && !matches!(d, VaultDialog::Info { .. });
        if keep && act.is_none() {
            self.vault.dialog = Some(d);
        } else if keep {
            // The action decides what comes next; keep the dialog unless it replaced it.
            let replaces = matches!(d, VaultDialog::Reset { .. } | VaultDialog::Migrate { .. });
            if !replaces {
                self.vault.dialog = Some(d);
            } else {
                self.close_vault_dialog();
            }
        } else {
            self.close_vault_dialog();
        }
        if let Some(a) = act {
            a(self);
        }
        if canceled {
            self.cancel_pending();
        }
    }
}

/// An information dialog (a function, so that the extraction of the texts
/// sees the `tr!` calls: `field: tr!(…)` is taken for a path by rolid-po-tool).
fn info(title: String, text: String, error: bool) -> VaultDialog {
    VaultDialog::Info { title, text, error }
}

fn new_create(reason: String) -> VaultDialog {
    VaultDialog::Create {
        pw1: Default::default(),
        pw2: Default::default(),
        strength: Strength::default(),
        ack: false,
        reason,
        error: None,
        focus: true,
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn locks_the_set_time_after_the_last_use() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(lock_left(Some(30), t0, at(10)), Some(Duration::from_secs(20)));
        assert_eq!(lock_left(Some(30), t0, at(30)), Some(Duration::ZERO), "due");
        assert_eq!(lock_left(Some(30), t0, at(95)), Some(Duration::ZERO));
        // Used again at 25 s: another 30 s from then.
        assert_eq!(lock_left(Some(30), at(25), at(30)), Some(Duration::from_secs(25)));
        assert_eq!(lock_left(None, t0, at(100_000)), None, "never");
        assert_eq!(LOCK_CHOICES, [Some(30), Some(60), Some(300), Some(900), Some(3600), None]);
    }

    #[test]
    fn countdown_text() {
        assert_eq!(countdown(Duration::from_secs(65)), "1:05");
        assert_eq!(countdown(Duration::from_millis(29_300)), "0:30", "rounded up");
        assert_eq!(countdown(Duration::ZERO), "0:00");
    }
}
