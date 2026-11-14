//! Listings that keep up by themselves.
//!
//! * A local folder shown in a panel is watched (inotify); a change made by
//!   another program refreshes it within a moment. Where watching does not
//!   work (network disks, a watch that cannot be set up) the server rules apply.
//! * A server folder (and an unwatched local one) refreshes when the user comes
//!   back to it – another tab, the other panel, back to the window – if the
//!   listing is older than [`STALE`], and while the window is active also every
//!   few seconds as set in View → Automatic refresh (30 s by default; folders of
//!   more than [`BIG_FOLDER`] items at most every 5 minutes).
//! * A refresh never disturbs: it runs in the background, only what changed
//!   changes (see `Pane::update_listing`), and it waits while the user renames
//!   in the list, drags, or a dialog or menu is open. A lost connection is not
//!   tried and shows no error; the same folder is never listed twice at once.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui;
use rolid_core::VfsResult;

use crate::pane::Prepared;
use rolid_edit::watch::DirWatcher;

use super::{App, Dialog, Msg, PaneRef};

/// A listing older than this refreshes when the user comes back to it.
pub const STALE: Duration = Duration::from_secs(5);
/// Folders with more items refresh by the interval at most every [`BIG_INTERVAL`].
pub const BIG_FOLDER: usize = 20_000;
pub const BIG_INTERVAL: Duration = Duration::from_secs(300);
/// Wait for a quiet moment after a change on the disk (a copy writes many times).
pub const QUIET: Duration = Duration::from_millis(300);
/// The interval choices in seconds (0 = off) and the default.
pub const CHOICES: [u32; 5] = [0, 10, 30, 60, 300];
pub const DEFAULT_SECS: u32 = 30;

/// File systems where a watch does not see changes made on other computers.
const NETWORK_FS: [&str; 14] = [
    "nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "sshfs", "9p", "davfs", "fuse.rclone", "ceph", "glusterfs", "afs", "fuse.gvfsd-fuse",
];

/// Can a watch be trusted on this file system (`None` = unknown: yes)?
pub fn watchable(fstype: Option<&str>) -> bool {
    !fstype.is_some_and(|t| NETWORK_FS.contains(&t))
}

/// The interval for a folder of `items` (`None` = off).
pub fn interval(secs: u32, items: usize) -> Option<Duration> {
    if secs == 0 {
        return None;
    }
    let d = Duration::from_secs(secs.into());
    Some(if items > BIG_FOLDER { d.max(BIG_INTERVAL) } else { d })
}

/// Should a shown panel refresh by the interval now? Only while the window is active.
pub fn due(secs: u32, items: usize, age: Duration, focused: bool) -> bool {
    focused && interval(secs, items).is_some_and(|i| age >= i)
}

/// Coming back to a panel: is its listing old enough to refresh?
pub fn stale(age: Option<Duration>) -> bool {
    age.is_some_and(|a| a > STALE)
}

/// Where a panel's listing comes from: remote or not, connection label, folder.
pub type LocKey = (bool, String, String);

#[derive(Default)]
pub struct AutoRefresh {
    /// The window was active in the last frame.
    focused: bool,
    /// Watched local folders (`None` = cannot be watched: refreshed by the interval).
    watches: HashMap<String, Option<DirWatcher>>,
    /// Watched folders that changed, with the time of the last change.
    changed: HashMap<String, Instant>,
    /// Folders being listed in the background.
    in_flight: HashSet<LocKey>,
}

impl AutoRefresh {
    /// Is a local folder watched (so the interval is not needed)?
    fn watched(&self, path: &str) -> bool {
        self.watches.get(path).is_some_and(Option::is_some)
    }
}

impl App {
    /// The seconds between refreshes (View → Automatic refresh), 0 = off.
    pub(super) fn refresh_secs(&self) -> u32 {
        self.settings.auto_refresh_secs.unwrap_or(DEFAULT_SECS)
    }

    pub(super) fn set_refresh_secs(&mut self, secs: u32) {
        self.settings.auto_refresh_secs = (secs != DEFAULT_SECS).then_some(secs);
        self.save_settings();
    }

    fn loc_key(&self, at: PaneRef) -> Option<LocKey> {
        let p = self.pane(at)?;
        Some((p.fs.is_remote(), p.fs.label(), p.path.clone()))
    }

    /// The panels on screen (the active tab of each side).
    fn shown_panes(&self) -> [PaneRef; 2] {
        [0, 1].map(|side| PaneRef { tab: self.sides[side].current().id, side })
    }

    /// A local folder in a plain listing (not a search, not inside an archive).
    fn local_folder(&self, at: PaneRef) -> Option<String> {
        let p = self.pane(at)?;
        (p.fs.is_local_disk() && p.search.is_none() && p.archive.is_none() && !p.offline).then(|| p.path.clone())
    }

    /// The user is in the middle of something a refresh could disturb.
    fn refresh_blocked(&self, ctx: &egui::Context) -> bool {
        !matches!(self.dialog, Dialog::None)
            || self.inner_drag.is_some()
            || self.drag_out.is_some()
            || self.switching.is_some()
            || self.renaming.is_some()
            || self.folder_pick.is_some()
            || self.all_panes().any(|(_, p)| p.rename.is_some() || p.path_edit.is_some())
            || ctx.any_popup_open()
            || egui::Popup::is_any_open(ctx)
            || ctx.dragged_id().is_some()
    }

    /// Once per frame: notice coming back to the window, keep the watches,
    /// refresh what changed or is due, and plan the next wake-up.
    pub(super) fn auto_refresh_frame(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let back = focused && !self.auto.focused;
        self.auto.focused = focused;
        self.keep_watches();
        if self.refresh_blocked(ctx) {
            // Try again soon; nothing is lost (the ages keep growing).
            if focused || !self.auto.changed.is_empty() {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            return;
        }
        if back {
            for at in self.shown_panes() {
                self.refresh_if_stale(at);
            }
        }
        // Local folders changed by other programs (after a quiet moment).
        let now = Instant::now();
        let ready: Vec<String> = self.auto.changed.iter().filter(|(_, t)| now.duration_since(**t) >= QUIET).map(|(p, _)| p.clone()).collect();
        for path in ready {
            self.auto.changed.remove(&path);
            let panes: Vec<PaneRef> = self.all_panes().map(|(r, _)| r).filter(|r| self.local_folder(*r).as_deref() == Some(path.as_str())).collect();
            if let Some(at) = panes.first() {
                self.quiet_refresh(*at);
            }
        }
        // The interval, for what is not watched.
        let secs = self.refresh_secs();
        let mut next: Option<Duration> = None;
        for at in self.shown_panes() {
            let Some(p) = self.pane(at) else { continue };
            if self.local_folder(at).is_some_and(|d| self.auto.watched(&d)) {
                continue;
            }
            let Some(iv) = interval(secs, p.entries.len()) else { continue };
            let age = p.loaded_at.map(|t| t.elapsed()).unwrap_or_default();
            if due(secs, p.entries.len(), age, focused) {
                self.quiet_refresh(at);
            } else if focused && p.loaded_at.is_some() {
                let left = iv.saturating_sub(age);
                next = Some(next.map_or(left, |n| n.min(left)));
            }
        }
        if let Some(t) = self.auto.changed.values().map(|t| (*t + QUIET).saturating_duration_since(now)).min() {
            next = Some(next.map_or(t, |n| n.min(t)));
        }
        if let Some(n) = next {
            ctx.request_repaint_after(n + Duration::from_millis(20));
        }
    }

    /// Watch the local folders on screen; forget the others.
    fn keep_watches(&mut self) {
        let shown: Vec<String> = self.shown_panes().into_iter().filter_map(|at| self.local_folder(at)).collect();
        self.auto.watches.retain(|p, _| shown.contains(p));
        self.auto.changed.retain(|p, _| shown.contains(p));
        for path in shown {
            if self.auto.watches.contains_key(&path) {
                continue;
            }
            let fstype = rolid_disk::mounts::mount_of(&path).map(|m| m.fstype);
            let watch = if watchable(fstype.as_deref()) {
                let (tx, ctx, p) = (self.tx.clone(), self.ctx.clone(), path.clone());
                match DirWatcher::new(std::path::Path::new(&path), move || {
                    let _ = tx.send(Msg::DirChanged(p.clone()));
                    ctx.request_repaint();
                }) {
                    Ok(w) => Some(w),
                    Err(e) => {
                        tracing::info!("folder not watched, refreshed by the interval: {e}");
                        None
                    }
                }
            } else {
                tracing::info!("network file system {fstype:?}: refreshed by the interval");
                None
            };
            self.auto.watches.insert(path, watch);
        }
    }

    /// A watched folder changed (from the watcher thread).
    pub(super) fn on_dir_changed(&mut self, path: String) {
        if self.auto.watches.contains_key(&path) {
            self.auto.changed.insert(path, Instant::now());
        }
    }

    /// Coming back to a panel (tab, the other panel, the window): refresh an old listing.
    pub(super) fn refresh_if_stale(&mut self, at: PaneRef) {
        if self.pane(at).is_some_and(|p| stale(p.loaded_at.map(|t| t.elapsed()))) {
            self.quiet_refresh(at);
        }
    }

    /// List the panel's folder in the background and update every panel that
    /// shows it, without disturbing. Not while it is loading or already being
    /// listed, not without a connection.
    pub(super) fn quiet_refresh(&mut self, at: PaneRef) {
        let Some(p) = self.pane(at) else { return };
        if p.offline || p.loading || p.search.is_some() || p.archive.is_some() || p.path.is_empty() || self.pane_disconnected(at) {
            return;
        }
        let (fs, path) = (p.fs.clone(), p.path.clone());
        let sort = Some((p.sort, p.sort_asc));
        let Some(key) = self.loc_key(at) else { return };
        if !self.auto.in_flight.insert(key.clone()) {
            return;
        }
        tracing::debug!("background refresh");
        self.spawn(async move {
            let result = super::prepare(fs.list(&path).await, sort).await;
            Msg::Refreshed { key, result }
        });
    }

    /// A background listing arrived.
    pub(super) fn on_refreshed(&mut self, key: LocKey, result: VfsResult<Prepared>) {
        self.auto.in_flight.remove(&key);
        let panes: Vec<PaneRef> = self.all_panes().filter(|(r, p)| !p.loading && self.loc_key(*r).as_ref() == Some(&key)).map(|(r, _)| r).collect();
        let listing = match result {
            Ok(l) => l,
            Err(e) => {
                // Not an error for the user: the next refresh tries again after
                // the interval (a lost connection is shown in the panel's strip).
                tracing::debug!("background refresh failed: {}", e.text().id);
                for at in panes {
                    if let Some(p) = self.pane_mut(at) {
                        p.loaded_at = Some(Instant::now());
                    }
                }
                return;
            }
        };
        if self.refresh_blocked(&self.ctx.clone()) {
            return; // started meanwhile: the listing stays old and is refreshed later
        }
        // Usually one panel: it takes the listing itself (no copy of a big one).
        let mut listing = Some(listing);
        let last = panes.len().saturating_sub(1);
        for (i, at) in panes.into_iter().enumerate() {
            let l = if i == last {
                listing.take()
            } else {
                listing.as_ref().map(|l| Prepared { entries: l.entries.clone(), sorted: l.sorted, print: l.print })
            };
            if let (Some(p), Some(l)) = (self.pane_mut(at), l) {
                if p.update_prepared(l) {
                    tracing::debug!("listing changed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coming_back_refreshes_listings_older_than_five_seconds() {
        assert!(!stale(None), "never listed: loaded the normal way");
        assert!(!stale(Some(Duration::from_secs(2))));
        assert!(!stale(Some(STALE)));
        assert!(stale(Some(Duration::from_millis(5_001))));
        assert!(stale(Some(Duration::from_secs(600))));
    }

    #[test]
    fn the_interval_runs_only_while_the_window_is_active() {
        let s = Duration::from_secs;
        assert!(due(30, 10, s(31), true));
        assert!(!due(30, 10, s(29), true));
        assert!(!due(30, 10, s(3600), false), "inactive window: no refresh");
        assert!(!due(0, 10, s(3600), true), "off");
        assert!(due(10, 10, s(10), true));
        // Big folders at most every 5 minutes.
        assert!(!due(30, BIG_FOLDER + 1, s(299), true));
        assert!(due(30, BIG_FOLDER + 1, s(300), true));
        assert!(due(30, BIG_FOLDER, s(30), true), "20 000 items is not big yet");
        assert_eq!(interval(600, BIG_FOLDER + 1), Some(s(600)), "a longer interval stays");
        assert_eq!(interval(0, 5), None);
        assert!(CHOICES.contains(&DEFAULT_SECS));
    }

    #[test]
    fn network_disks_are_not_watched() {
        assert!(watchable(Some("ext4")));
        assert!(watchable(Some("btrfs")));
        assert!(watchable(Some("tmpfs")));
        assert!(watchable(None));
        for t in ["nfs", "nfs4", "cifs", "smb3", "fuse.sshfs"] {
            assert!(!watchable(Some(t)), "{t}");
        }
    }
}
