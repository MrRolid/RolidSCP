//! "Count folder sizes": in the background, one folder after another.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rolid_core::copy::{Progress, Totals};
use rolid_core::EntryKind;

use super::{App, Job, Msg, PaneRef};
use crate::dirsize::{self, DirSize};

impl App {
    /// File → Count the size of all folders (Alt+Shift+Enter).
    pub(super) fn size_all_dirs(&mut self) {
        let at = self.active_ref();
        self.size_dirs(at, false);
    }

    /// Context menu → Count the size of the selected folders (marked, else the one under the cursor).
    pub(super) fn size_selected_dirs(&mut self) {
        let at = self.active_ref();
        self.size_dirs(at, true);
    }

    fn size_dirs(&mut self, at: PaneRef, selected: bool) {
        let conn = self.pane(at).and_then(|p| p.remote.as_ref()).and_then(|r| self.live_connection(&r.target.key()));
        let Some(p) = self.pane(at) else { return };
        if p.search.is_some() {
            self.last_error = Some(tr!("Folder sizes are not counted in search results."));
            return;
        }
        let real_dir = |e: &&rolid_core::Entry| e.kind == EntryKind::Dir;
        let names: Vec<String> = if selected {
            let marked: Vec<String> =
                p.entries.iter().filter(real_dir).filter(|e| p.marked.contains(&e.name)).map(|e| e.name.clone()).collect();
            if marked.is_empty() {
                p.current_entry().filter(|e| e.kind == EntryKind::Dir).map(|e| vec![e.name.clone()]).unwrap_or_default()
            } else {
                marked
            }
        } else {
            p.entries
                .iter()
                .filter(real_dir)
                .filter(|e| !p.dir_sizes.contains_key(&e.name) && !p.dir_pending.contains(&e.name))
                .map(|e| e.name.clone())
                .collect()
        };
        if names.is_empty() {
            if selected {
                self.last_error = Some(tr!("No folder is selected (links to folders are not counted)."));
            }
            return;
        }
        let fs = p.fs.clone();
        let path = p.path.clone();
        let local = fs.is_local_disk();
        let title = tr!("Folder sizes ({count}): {place}", count = names.len(), place = p.display_label());
        let id = self.id();
        let Some(p) = self.pane_mut(at) else { return };
        p.dir_pending.extend(names.iter().cloned());
        // One flag: Cancel, Esc, Ctrl+R and leaving the folder all stop it.
        let cancel = p.size_cancel.clone();
        let progress = Arc::new(Mutex::new(Progress { started: Some(Instant::now()), ..Default::default() }));
        self.jobs.push(Job {
            id,
            title,
            progress: progress.clone(),
            total_bytes: 0,
            cancel: cancel.clone(),
            note: Arc::default(),
            count: Some(names.len() as u64),
            totals: Arc::new(Mutex::new(Totals::default())),
        });
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let send = move |m: Msg| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };
        self.spawn(async move {
            // Whether `du` on the server works (tried on the first folder).
            let mut du_ok = conn.is_some();
            let mut method = if local { "locally" } else if conn.is_some() { "du on the server" } else { "by walking" };
            for name in &names {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                progress.lock().unwrap().current = name.clone();
                let full = fs.join(&path, name);
                let r: Option<DirSize> = if local {
                    let (c, pr) = (cancel.clone(), progress.clone());
                    tokio::task::spawn_blocking(move || {
                        let base = pr.lock().unwrap().bytes_done;
                        dirsize::local(&PathBuf::from(full), &c, &mut |b| pr.lock().unwrap().bytes_done = base + b)
                    })
                    .await
                    .ok()
                    .flatten()
                } else {
                    let mut r = None;
                    let mut done = false;
                    if du_ok {
                        if let Some(c) = &conn {
                            match dirsize::remote_du(c, &full, &cancel).await {
                                Ok(x) => {
                                    r = x;
                                    done = true;
                                }
                                Err(e) => {
                                    tracing::info!("folder sizes: {e}, walking over SFTP");
                                    du_ok = false;
                                    method = "by walking (du does not work)";
                                }
                            }
                        }
                    }
                    if !done {
                        let pr = progress.clone();
                        r = dirsize::walk(fs.as_ref(), &full, &cancel, &mut |b| pr.lock().unwrap().current = format!("{name} ({})", crate::pane::human_size(b))).await;
                    }
                    r
                };
                progress.lock().unwrap().files_done += 1;
                let (n, p2) = (name.clone(), path.clone());
                send(Msg::Then {
                    job: None,
                    f: Box::new(move |a: &mut App| {
                        if let Some(p) = a.pane_mut(at).filter(|p| p.path == p2) {
                            p.set_dir_size(&n, r);
                        }
                    }),
                });
            }
            let canceled = cancel.load(Ordering::Relaxed);
            tracing::info!("folder sizes {path}: {} folders, {method}{}", names.len(), if canceled { ", canceled" } else { "" });
            Msg::Then {
                job: Some(id),
                f: Box::new(move |a: &mut App| {
                    if let Some(p) = a.pane_mut(at).filter(|p| p.path == path) {
                        for n in &names {
                            p.dir_pending.remove(n);
                        }
                    }
                }),
            }
        });
    }
}
