//! Lost connections: noticing them, reconnecting in the background, showing the
//! state in panels and tabs, and continuing afterwards (reload panels, resume
//! queued transfers, upload pending F4 edits).
//!
//! The UI thread never waits for the network: it only reads the [`Link`] state
//! each frame; connecting runs on the tokio runtime.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use eframe::egui::{self, Color32, RichText};
use rolid_config::{secrets, RemoteTarget};
use rolid_ssh::link::drive_reconnect;
use rolid_ssh::{Attempt, ConnectParams, KbdPrompter, KbdRequest, Link, LinkState, NeedUser, SshConnection, SshError};
use zeroize::Zeroizing;

use super::{Action, App, PaneRef};

/// One server connection shared by all its panels.
pub struct LinkEntry {
    pub link: Arc<Link>,
    pub target: RemoteTarget,
    /// Generation the UI has reacted to (a newer one = reconnected).
    seen_gen: u64,
    /// Saved password for reconnecting (only while the vault is unlocked).
    password: Arc<Mutex<Option<Zeroizing<String>>>>,
    /// The same for the jump hosts, the nearest to the server first.
    hops: Arc<Mutex<Vec<Option<Zeroizing<String>>>>>,
    /// The server's clock against this computer's (read after connecting).
    pub clock: Option<rolid_ssh::clock::ClockOffset>,
    /// The line at the bottom already said that the clock is off.
    clock_warned: bool,
}

#[derive(Default)]
pub struct LinksState {
    pub map: HashMap<String, LinkEntry>,
    /// The error line as it was while all connections were up.
    error_before_loss: Option<String>,
    /// An error line that appeared while a connection was lost (a failed
    /// operation because of the loss): cleared when the connection is back.
    error_during_loss: Option<String>,
}

impl LinksState {
    /// Wipe the saved passwords kept for reconnecting (the vault was locked).
    /// Returns how many there were.
    pub fn wipe_passwords(&mut self) -> usize {
        let mut n = 0;
        for e in self.map.values() {
            if let Some(mut p) = e.password.lock().unwrap().take() {
                crate::secret::wipe(&mut p);
                n += 1;
            }
            for mut p in e.hops.lock().unwrap().drain(..).flatten() {
                crate::secret::wipe(&mut p);
                n += 1;
            }
        }
        n
    }
}

/// During automatic reconnection no dialog may pop up: a question from the
/// server means "the user is needed".
struct NoPrompt {
    asked: Arc<AtomicBool>,
}

#[async_trait]
impl KbdPrompter for NoPrompt {
    async fn prompt(&self, _req: KbdRequest) -> Option<Vec<Zeroizing<String>>> {
        self.asked.store(true, Ordering::SeqCst);
        None
    }
}

const AMBER: Color32 = Color32::from_rgb(150, 90, 0);
const RED: Color32 = Color32::from_rgb(190, 20, 20);

impl App {
    pub(super) fn link(&self, key: &str) -> Option<Arc<Link>> {
        self.links.map.get(key).map(|e| e.link.clone())
    }

    /// The link of a remote panel.
    pub(super) fn pane_link(&self, at: PaneRef) -> Option<Arc<Link>> {
        let r = self.pane(at)?.remote.as_ref()?;
        self.link(&r.target.key())
    }

    /// A live connection to reuse for `key`, if there is one.
    pub(super) fn live_connection(&self, key: &str) -> Option<SshConnection> {
        self.links.map.get(key).and_then(|e| e.link.current()).map(|(_, c)| c)
    }

    /// A new connection to `target` was made (first connect, or by the user while
    /// the old one was lost): install it in the shared link.
    pub(super) fn register_connection(&mut self, target: &RemoteTarget, conn: SshConnection) -> Arc<Link> {
        let key = target.key();
        match self.links.map.get_mut(&key) {
            Some(e) => {
                let same = e.link.current().is_some_and(|(_, c)| c.same(&conn));
                if !same {
                    e.link.restore(conn);
                }
                e.target = target.clone();
                e.link.clone()
            }
            None => {
                let link = Link::new(conn);
                let seen_gen = link.generation();
                self.links.map.insert(
                    key,
                    LinkEntry { link: link.clone(), target: target.clone(), seen_gen, password: Arc::default(), hops: Arc::default(), clock: None, clock_warned: false },
                );
                link
            }
        }
    }

    /// Per frame: notice lost connections, start reconnecting, react to restored ones.
    pub(super) fn links_frame(&mut self, ctx: &egui::Context) {
        let mut start = Vec::new();
        let mut restored = Vec::new();
        let mut waiting = false;
        let vault_open = self.vault.open.is_some();
        for (key, e) in &mut self.links.map {
            e.link.check();
            let st = e.link.state();
            if !st.is_up() {
                waiting = true;
            }
            if e.link.claim_driver() {
                start.push(key.clone());
            }
            let gen = e.link.generation();
            if gen != e.seen_gen {
                e.seen_gen = gen;
                if st.is_up() {
                    restored.push(key.clone());
                }
            }
        }
        if !vault_open {
            // Locked: no copy kept for reconnecting.
            self.links.wipe_passwords();
        }
        // Lost connections nobody uses any more (tabs closed): stop trying.
        let unused: Vec<String> = self
            .links
            .map
            .iter()
            .filter(|(k, e)| !e.link.state().is_up() && !self.link_in_use(k))
            .map(|(k, _)| k.clone())
            .collect();
        for k in unused {
            if let Some(e) = self.links.map.remove(&k) {
                e.link.stop();
                tracing::info!("connection {k} is no longer used: reconnecting stopped");
            }
            start.retain(|s| *s != k);
        }
        for key in start {
            self.start_reconnect(&key);
        }
        for key in restored {
            self.on_link_restored(&key);
        }
        // Remember which error line came while a connection was down.
        if waiting {
            if self.last_error.is_some() && self.last_error != self.links.error_before_loss {
                self.links.error_during_loss = self.last_error.clone();
            }
        } else {
            self.links.error_before_loss = self.last_error.clone();
            self.links.error_during_loss = None;
        }
        if waiting {
            // Countdown in the panels.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Run the reconnection loop for `key` on the runtime.
    fn start_reconnect(&mut self, key: &str) {
        let Some(e) = self.links.map.get(key) else { return };
        let (link, target, slot, hops) = (e.link.clone(), e.target.clone(), e.password.clone(), e.hops.clone());
        let prof = target.profile.as_ref().and_then(|id| self.profiles.get(id)).cloned();
        let needs_vault = prof.as_ref().is_some_and(|p| p.save_password && p.password_in_vault());
        let keyring = prof.as_ref().is_some_and(|p| p.save_password && p.password_in_keyring());
        if needs_vault {
            if let Some(pw) = prof.as_ref().and_then(|p| self.vault_password(&p.id)) {
                *slot.lock().unwrap() = Some(pw);
            }
        }
        let (hop_pw, hop_needs) = self.hop_passwords(&target);
        *hops.lock().unwrap() = hop_pw;
        tracing::info!("reconnecting {key}");
        self.rt.spawn(drive_reconnect(link, move |_attempt| {
            let (target, slot, hops, hop_needs) = (target.clone(), slot.clone(), hops.clone(), hop_needs.clone());
            async move { reconnect_once(target, slot, needs_vault, keyring, hops, hop_needs).await }
        }));
    }

    /// The saved passwords of the jump hosts of `target` (nearest first) and
    /// which of them need the vault. A locked vault gives `None`.
    pub(super) fn hop_passwords(&self, target: &RemoteTarget) -> (Vec<Option<Zeroizing<String>>>, Vec<bool>) {
        target
            .jumps()
            .iter()
            .map(|j| {
                let p = j.profile.as_deref().and_then(|id| self.profiles.get(id));
                let needs = p.is_some_and(|p| p.password_in_vault());
                (p.filter(|_| needs).and_then(|p| self.vault_password(&p.id)), needs)
            })
            .unzip()
    }

    /// The connection is back: reload panels (keeping cursor, marks and scroll
    /// position), continue F4 uploads that were cut off.
    fn on_link_restored(&mut self, key: &str) {
        let panes: Vec<PaneRef> = self
            .all_panes()
            .filter(|(_, p)| !p.offline && p.search.is_none() && p.remote.as_ref().is_some_and(|r| r.target.key() == key))
            .map(|(r, _)| r)
            .collect();
        for at in panes {
            self.refresh(at);
        }
        // First put back what the loss cut off, then upload the edits again.
        self.repair_replacements(key.to_string(), true);
        self.check_server_clock(key);
        // An error caused by the loss is no longer true.
        if self.last_error.is_some() && self.last_error == self.links.error_during_loss {
            self.last_error = None;
            self.links.error_during_loss = None;
        }
    }

    /// Ask server `key` for its time (in the background); a clock off by
    /// more than a minute is said once in the line at the bottom.
    pub(super) fn check_server_clock(&mut self, key: &str) {
        if self.settings.clock_warning_off {
            return;
        }
        let Some(link) = self.link(key) else { return };
        let key = key.to_string();
        self.spawn(async move {
            let r = match link.current() {
                Some((_, conn)) => rolid_ssh::clock::measure(&conn).await,
                None => None,
            };
            super::Msg::Then { job: None, f: Box::new(move |a: &mut App| a.on_server_clock(&key, r)) }
        });
    }

    fn on_server_clock(&mut self, key: &str, r: Option<rolid_ssh::clock::ClockOffset>) {
        let Some(e) = self.links.map.get_mut(key) else { return };
        match r {
            Some(o) => tracing::info!("server clock of {key}: {:+} s (± {} s)", o.secs, o.uncertainty),
            None => tracing::info!("server clock of {key}: not known (no shell or no answer)"),
        }
        // A reading replaces the last one; no reading keeps it.
        if r.is_some() {
            e.clock = r;
        }
        let Some(o) = e.clock.filter(|o| o.is_off()) else {
            e.clock_warned = false;
            return;
        };
        if std::mem::replace(&mut e.clock_warned, true) || self.settings.clock_warning_off {
            return;
        }
        let server = self.server_name(key);
        tracing::warn!("the clock of {key} is off by {} s", o.secs);
        self.last_error = Some(clock_warning(&server, o));
    }

    /// The name of a server for messages: its saved name, or user@host.
    fn server_name(&self, key: &str) -> String {
        let e = self.links.map.get(key);
        e.and_then(|e| e.target.profile.as_deref())
            .and_then(|id| self.profiles.get(id))
            .map(|p| p.name.clone())
            .or_else(|| e.map(|e| e.link.label.clone()))
            .unwrap_or_else(|| key.to_string())
    }

    /// A note for the transfer dialogs when a server shown in a panel has a
    /// clock that is off (the panels are where a transfer goes from and to).
    pub(super) fn clock_note(&self) -> Option<String> {
        self.sides.iter().find_map(|side| {
            let key = side.current().pane.remote.as_ref()?.target.key();
            let o = self.clock_off(&key)?;
            let diff = clock_diff(o.secs.unsigned_abs());
            let server = self.server_name(&key);
            Some(if o.secs > 0 {
                tr!("⚠ The clock of {server} is {diff} ahead: files changed on it look newer than they are.", server = server, diff = diff)
            } else {
                tr!("⚠ The clock of {server} is {diff} behind: files changed on it look older than they are.", server = server, diff = diff)
            })
        })
    }

    /// The clock of server `key`, when it is off by more than a minute.
    /// Nothing when the warning is turned off in the settings.
    pub(super) fn clock_off(&self, key: &str) -> Option<rolid_ssh::clock::ClockOffset> {
        if self.settings.clock_warning_off {
            return None;
        }
        self.links.map.get(key).and_then(|e| e.clock).filter(|o| o.is_off())
    }

    /// Finish or undo replacements of files on server `key` that a lost
    /// connection or a crash cut off ([`rolid_core::replace`]); then, after a
    /// restored connection (`restored`), upload the edits that were waiting.
    pub(super) fn repair_replacements(&mut self, key: String, restored: bool) {
        let Some(link) = self.link(&key) else { return };
        let fs = rolid_ssh::RemoteFs::new(link);
        self.spawn(async move {
            let r = rolid_core::replace::repair(&fs).await;
            super::Msg::Then { job: None, f: Box::new(move |a: &mut App| a.on_repaired(&key, r, restored)) }
        });
    }

    fn on_repaired(&mut self, key: &str, r: rolid_core::replace::Repair, restored: bool) {
        if !r.is_empty() {
            let server = self.links.map.get(key).map(|e| e.link.label.clone()).unwrap_or_else(|| key.to_string());
            let mut parts = Vec::new();
            if !r.restored.is_empty() {
                tracing::warn!("cut-off replacements on {key}: originals put back: {}", r.restored.join(", "));
                parts.push(tr!(
                    "Replacing a file on {server} was cut off (lost connection or crash). The original file was put back under its name; the new version did not arrive, copy or save it again:\n{files}",
                    server = server.as_str(),
                    files = r.restored.join("\n")
                ));
            }
            if !r.completed.is_empty() {
                tracing::info!("cut-off replacements on {key}: finished: {}", r.completed.join(", "));
                parts.push(tr!(
                    "Replacing a file on {server} was cut off (lost connection or crash). The new version had arrived complete and is now in place:\n{files}",
                    server = server.as_str(),
                    files = r.completed.join("\n")
                ));
            }
            self.last_error = Some(parts.join("\n\n"));
            let panes: Vec<PaneRef> = self
                .all_panes()
                .filter(|(_, p)| !p.offline && p.search.is_none() && p.remote.as_ref().is_some_and(|x| x.target.key() == key))
                .map(|(r, _)| r)
                .collect();
            for at in panes {
                self.refresh(at);
            }
        }
        if r.pending > 0 {
            tracing::info!("{} cut-off replacements on {key} wait for the next connection", r.pending);
        }
        if restored {
            self.edits_link_restored(key);
        }
    }

    /// The strip at the top of a disconnected panel. Returns an action to run.
    pub(super) fn link_bar(&self, ui: &mut egui::Ui, at: PaneRef) -> Option<Action> {
        let p = self.pane(at)?;
        if p.offline {
            return None;
        }
        let target = p.remote.as_ref()?.target.clone();
        let key = target.key();
        let link = self.link(&key)?;
        let st = link.state();
        if st.is_up() {
            return None;
        }
        let mut act: Option<Action> = None;
        let color = match st {
            LinkState::Reconnecting { .. } => AMBER,
            _ => RED,
        };
        egui::Frame::new()
            .fill(Color32::from_rgb(255, 243, 205))
            .stroke(egui::Stroke::new(1.0, color))
            .corner_radius(3.0)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("⚡ {}", rolid_i18n::t(&st.text()))).color(color).strong());
                    match &st {
                        LinkState::Reconnecting { .. } | LinkState::Stopped { .. } => {
                            if ui.small_button(tr!("Connect now")).clicked() {
                                let l = link.clone();
                                act = Some(Box::new(move |_: &mut App| {
                                    l.connect_now();
                                }));
                            }
                        }
                        LinkState::NeedsUser { reason: NeedUser::Unlock, .. } => {
                            if ui.small_button(tr!("Unlock…")).clicked() {
                                let l = link.clone();
                                act = Some(Box::new(move |a: &mut App| {
                                    let l2 = l.clone();
                                    a.with_vault(
                                        tr!("The saved password is needed to restore the connection."),
                                        Box::new(move |_| {
                                            l2.connect_now();
                                        }),
                                        None,
                                    );
                                }));
                            }
                        }
                        LinkState::NeedsUser { .. } => {
                            if ui.small_button(tr!("Enter credentials…")).clicked() {
                                act = Some(Box::new(move |a: &mut App| a.open_connect_for(at, Some(&target))));
                            }
                        }
                        LinkState::Up => {}
                    }
                    if !matches!(st, LinkState::Stopped { .. }) && ui.small_button(tr!("Cancel")).clicked() {
                        let l = link.clone();
                        act = Some(Box::new(move |_: &mut App| l.stop()));
                    }
                });
                if let Some(e) = st.error() {
                    ui.add(egui::Label::new(RichText::new(rolid_i18n::t(e)).weak().size(crate::fonts::fs(11.0))).truncate());
                }
            });
        act
    }

    /// Does a panel, F4 edit or transfer still use the connection `key`?
    fn link_in_use(&self, key: &str) -> bool {
        let link = self.links.map.get(key).map(|e| e.link.clone());
        self.all_panes().any(|(_, p)| p.remote.as_ref().is_some_and(|r| r.target.key() == key))
            || self.edits_use_link(key)
            || self.viewers_use_link(key)
            || link.is_some_and(|l| {
                self.failed_jobs.iter().any(|f| f.spec.links.iter().any(|x| Arc::ptr_eq(x, &l)))
                    || !self.jobs.is_empty() // running jobs may wait for it
            })
    }

    /// Is the panel's connection lost (for graying it out)?
    pub(super) fn pane_disconnected(&self, at: PaneRef) -> bool {
        self.pane(at).is_some_and(|p| !p.offline) && self.pane_link(at).is_some_and(|l| !l.state().is_up())
    }
}

/// One reconnection attempt with what is available without asking: the saved
/// password (if unlocked), ssh-agent, keys.
async fn reconnect_once(
    target: RemoteTarget,
    slot: Arc<Mutex<Option<Zeroizing<String>>>>,
    needs_vault: bool,
    keyring: bool,
    hops: Arc<Mutex<Vec<Option<Zeroizing<String>>>>>,
    hop_needs: Vec<bool>,
) -> Attempt {
    let mut password = slot.lock().unwrap().clone();
    if password.is_none() && keyring {
        if let Some(id) = target.profile.clone() {
            if let Ok(Ok(Some(p))) = tokio::task::spawn_blocking(move || secrets::get(&id)).await {
                password = Some(Zeroizing::new(p));
            }
        }
    }
    let locked = needs_vault && password.is_none();
    let hop_pw = hops.lock().unwrap().clone();
    let hop_locked = hop_needs.iter().enumerate().any(|(i, n)| *n && hop_pw.get(i).is_none_or(|p| p.is_none()));
    let asked = Arc::new(AtomicBool::new(false));
    let mut params = params_for(&target, password.as_deref().map(|p| p.as_str()), None, None, &hop_pw, Some(Arc::new(NoPrompt { asked: asked.clone() })));
    drop(hop_pw);
    let r = SshConnection::connect(&params).await;
    params.wipe();
    match r {
        Ok(c) => Attempt::Connected(c),
        Err(SshError::AuthFailed(m)) if locked => Attempt::NeedsUser(NeedUser::Unlock, m),
        Err(e @ SshError::JumpAuthFailed { .. }) if hop_locked => Attempt::NeedsUser(NeedUser::Unlock, e.text()),
        Err(e @ SshError::JumpAuthFailed { .. }) => Attempt::NeedsUser(NeedUser::Credentials, e.text()),
        Err(SshError::AuthFailed(m)) => {
            let m = if asked.load(Ordering::SeqCst) {
                rolid_i18n::rolid_text::txt!("{error} (the server asks for more information)", error = m)
            } else {
                m
            };
            Attempt::NeedsUser(NeedUser::Credentials, m)
        }
        Err(
            e @ (SshError::UnknownHostKey { .. }
            | SshError::HostKeyChanged { .. }
            | SshError::HostKeyRevoked { .. }
            | SshError::HostKeysDisagree { .. }),
        ) => {
            Attempt::NeedsUser(NeedUser::HostKey, e.text())
        }
        Err(SshError::Other(m)) => Attempt::Retry(m),
    }
}

/// Connection parameters for `target` and its jump hosts. `hops[i]` is the
/// password of the i-th jump host, the nearest to the server first.
/// `accept`: a server key the user trusted, for the server (of the route)
/// with that address and port.
pub(super) fn params_for(
    target: &RemoteTarget,
    password: Option<&str>,
    passphrase: Option<&str>,
    accept: Option<&(String, u16, String)>,
    hops: &[Option<Zeroizing<String>>],
    prompter: Option<Arc<dyn rolid_ssh::KbdPrompter>>,
) -> ConnectParams {
    let one = |t: &RemoteTarget, pw: Option<&str>| ConnectParams {
        host: t.host.clone(),
        port: t.port,
        user: t.user.clone(),
        password: pw.map(Into::into),
        key_path: t.key_path.clone().map(Into::into),
        accept_fingerprint: accept.filter(|(h, p, _)| *h == t.host && *p == t.port).map(|a| a.2.clone()),
        known_hosts: Some(crate::terminal::own_known_hosts()),
        known_hosts_read: crate::terminal::ssh_known_hosts().into_iter().collect(),
        prompter: prompter.clone(),
        ..Default::default()
    };
    let mut p = one(target, password);
    p.key_passphrase = passphrase.map(Into::into);
    let mut via: Option<Box<ConnectParams>> = None;
    for (i, j) in target.jumps().iter().enumerate().rev() {
        let mut jp = one(j, hops.get(i).and_then(|x| x.as_deref()).map(|s| s.as_str()));
        jp.via = via.take();
        via = Some(Box::new(jp));
    }
    p.via = via;
    p
}

/// How far a clock is off, in words: "5 min 12 s", "2 h 3 min", "3 days".
pub fn clock_diff(secs: u64) -> String {
    if secs < 3600 {
        tr!("{m} min {s} s", m = secs / 60, s = secs % 60)
    } else if secs < 2 * 86_400 {
        tr!("{h} h {m} min", h = secs / 3600, m = secs / 60 % 60)
    } else {
        trn!("{n} day", "{n} days", secs / 86_400)
    }
}

/// The line at the bottom about a server's clock that is off.
pub fn clock_warning(server: &str, o: rolid_ssh::clock::ClockOffset) -> String {
    let diff = clock_diff(o.secs.unsigned_abs());
    if o.secs > 0 {
        tr!(
            "The clock of server {server} is {diff} ahead of this computer. Times of files changed on it are off by as much, and \"Overwrite only older\" may decide wrongly.",
            server = server,
            diff = diff
        )
    } else {
        tr!(
            "The clock of server {server} is {diff} behind this computer. Times of files changed on it are off by as much, and \"Overwrite only older\" may decide wrongly.",
            server = server,
            diff = diff
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_differences_in_words() {
        assert_eq!(clock_diff(65), "1 min 5 s");
        assert_eq!(clock_diff(3600), "1 h 0 min");
        assert_eq!(clock_diff(3 * 3600 + 125), "3 h 2 min");
        assert_eq!(clock_diff(3 * 86_400), "3 days");
        let o = rolid_ssh::clock::ClockOffset { secs: -3600, uncertainty: 1 };
        assert!(clock_warning("Web", o).contains("behind"));
    }

    #[test]
    fn locking_wipes_the_reconnect_passwords() {
        let mut st = LinksState::default();
        let target = RemoteTarget { profile: Some("p".into()), host: "h".into(), port: 22, user: "u".into(), key_path: None, jump: None };
        for (key, pw) in [("a", Some("secret")), ("b", None)] {
            let slot = Arc::new(Mutex::new(pw.map(|p| Zeroizing::new(p.to_string()))));
            st.map.insert(key.into(), LinkEntry { link: Link::new_lost(key, "lost"), target: target.clone(), seen_gen: 0, password: slot, hops: Arc::new(Mutex::new(vec![Some(Zeroizing::new("hop".into())), None])), clock: None, clock_warned: false });
        }
        assert_eq!(st.wipe_passwords(), 3);
        assert!(st.map.values().all(|e| e.password.lock().unwrap().is_none() && e.hops.lock().unwrap().is_empty()));
        assert_eq!(st.wipe_passwords(), 0);
    }
}
