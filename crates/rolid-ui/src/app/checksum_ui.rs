//! "Calculate checksums…": dialog, computation in the background, copy and save.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use rolid_core::copy::Conflict;
use rolid_core::{EntryKind, Vfs};
use rolid_i18n::rolid_text::txt;
use rolid_i18n::{t, Text};
use rolid_local::LocalFs;

use super::archive_ui::new_temp;
use super::{App, CopySpec, Dialog, PaneRef};
use crate::checksum::{self, Algo};
use crate::dialog;
use crate::look::Role;
use crate::pane::{human_size, Target};

/// First step: what and how.
pub struct SumAsk {
    at: PaneRef,
    items: Vec<Target>,
    /// Some of the items are folders.
    dirs: bool,
    algo: Algo,
    recursive: bool,
}

pub struct SumRow {
    /// Path relative to the panel's folder (as written to the list).
    pub rel: String,
    pub abs: String,
    pub size: u64,
    pub sum: Option<Result<String, String>>,
}

#[derive(Default)]
pub struct SumState {
    pub rows: Vec<SumRow>,
    pub done_bytes: u64,
    pub total: u64,
    /// How the sums are computed (shown in the dialog).
    pub method: Option<Text>,
    /// Still looking for files in the folders.
    pub listing: bool,
    pub finished: bool,
    /// Folders / links left out, with the reason.
    pub skipped: Vec<Text>,
}

/// Second step: running / finished.
pub struct SumRun {
    at: PaneRef,
    algo: Algo,
    state: Arc<Mutex<SumState>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    eta: rolid_core::eta::Window,
    name: String,
    to_other: bool,
    /// The source panel cannot be written (archive).
    here_read_only: bool,
    /// Note after copy / save (`true` = error).
    msg: Option<(bool, String)>,
    /// The file exists in the target: asked to overwrite.
    ask_overwrite: bool,
}

pub enum SumDialog {
    Ask(SumAsk),
    Run(Box<SumRun>),
}

impl App {
    /// File / context menu → Calculate checksums…
    pub(super) fn ask_checksums(&mut self) {
        let at = self.active_ref();
        let algo = self.settings.checksum_algo.as_deref().and_then(Algo::from_key).unwrap_or_default();
        let Some(p) = self.pane(at) else { return };
        let items = p.target_items();
        if items.is_empty() {
            self.last_error = Some(tr!("No file is selected."));
            return;
        }
        let dirs = items.iter().any(|t| p.entries.iter().any(|e| e.name == t.name && e.is_dir_like()) || p.search.is_some());
        self.dialog = Dialog::Checksum(Box::new(SumDialog::Ask(SumAsk { at, items, dirs, algo, recursive: true })));
    }

    fn start_checksums(&mut self, a: SumAsk) {
        self.settings.checksum_algo = Some(a.algo.key().into());
        self.save_settings();
        let conn = self.pane(a.at).and_then(|p| p.remote.as_ref()).and_then(|r| self.live_connection(&r.target.key()));
        let Some(p) = self.pane(a.at) else { return };
        let fs = p.fs.clone();
        let base = p.path.clone();
        let here_read_only = fs.read_only();
        let kinds: HashMap<String, bool> = p.entries.iter().map(|e| (e.name.clone(), e.kind == EntryKind::Dir)).collect();
        let state = Arc::new(Mutex::new(SumState { listing: true, ..Default::default() }));
        let cancel = Arc::new(AtomicBool::new(false));
        let run = SumRun {
            at: a.at,
            algo: a.algo,
            state: state.clone(),
            cancel: cancel.clone(),
            started: Instant::now(),
            eta: Default::default(),
            name: a.algo.file_name().to_string(),
            to_other: here_read_only,
            here_read_only,
            msg: None,
            ask_overwrite: false,
        };
        self.dialog = Dialog::Checksum(Box::new(SumDialog::Run(Box::new(run))));
        let (algo, recursive, items) = (a.algo, a.recursive, a.items);
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            // Repaint while it runs.
            let tick = {
                let (ctx, state) = (ctx.clone(), state.clone());
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        ctx.request_repaint();
                        if state.lock().unwrap().finished {
                            break;
                        }
                    }
                })
            };
            run_checksums(fs, conn, base, items, kinds, algo, recursive, state.clone(), cancel).await;
            state.lock().unwrap().finished = true;
            ctx.request_repaint();
            let _ = tick.await;
        });
    }

    pub(super) fn checksum_dialog_ui(&mut self, ctx: &egui::Context) {
        let Dialog::Checksum(d) = &mut self.dialog else { return };
        let mut close = false;
        let mut start: Option<SumAsk> = None;
        let mut save = false;
        let mut copy: Option<String> = None;
        match d.as_mut() {
            SumDialog::Ask(f) => {
                dialog::window("sum-ask", tr!("Calculate checksums")).icon(Role::Info).min_width(460.0).show(ctx, |ui| {
                    dialog::lead(ui, crate::app::archive_ui::items_label(&f.items));
                    let heading = tr!("Options");
                    dialog::group(ui, &heading, |ui| {
                        dialog::form(ui, "sum-ask-form", |ui| {
                            ui.label(tr!("Algorithm:"));
                            ui.horizontal_wrapped(|ui| {
                                for a in Algo::ALL {
                                    ui.radio_value(&mut f.algo, a, a.label());
                                }
                            });
                            ui.end_row();
                        });
                        if f.dirs {
                            ui.checkbox(&mut f.recursive, tr!("Including folder contents (recursively)"));
                        }
                        dialog::note(ui, tr!("Symbolic links inside folders are not followed."));
                    });
                    let b = dialog::buttons(ui, [dialog::btn(Role::Ok, tr!("Calculate")).default(true), dialog::cancel()]);
                    if b[0] {
                        start = Some(SumAsk { at: f.at, items: std::mem::take(&mut f.items), dirs: f.dirs, algo: f.algo, recursive: f.recursive });
                    }
                    if b[1] {
                        close = true;
                    }
                });
            }
            SumDialog::Run(r) => {
                let st = r.state.lock().unwrap();
                let finished = st.finished;
                r.eta.sample(r.started.elapsed(), st.done_bytes);
                let other = find_other(&self.sides, r.at);
                let title = tr!("Checksums – {algorithm}", algorithm = r.algo.label());
                dialog::window("sum-run", title).icon(Role::Info).min_width(860.0).max_width(860.0).show(ctx, |ui| {
                    dialog::lead(ui, tr!("Method: {method}", method = st.method.as_ref().map(t).unwrap_or_else(|| "…".to_string())));
                    let ok = st.rows.iter().filter(|x| matches!(x.sum, Some(Ok(_)))).count();
                    let bad = st.rows.iter().filter(|x| matches!(x.sum, Some(Err(_)))).count();
                    let frac = if st.total > 0 { st.done_bytes as f32 / st.total as f32 } else if finished { 1.0 } else { 0.0 };
                    let files = if st.listing {
                        trn!("{done} of {n} file (looking for files…)", "{done} of {n} files (looking for files…)", st.rows.len(), done = ok + bad)
                    } else {
                        trn!("{done} of {n} file", "{done} of {n} files", st.rows.len(), done = ok + bad)
                    };
                    let mut parts = vec![files, tr!("{done} of {total}", done = human_size(st.done_bytes), total = human_size(st.total))];
                    if !finished {
                        parts.push(match r.eta.speed() {
                            Some(s) => format!("{}/s", human_size(s as u64)),
                            None => tr!("speed: calculating"),
                        });
                        if !st.listing {
                            parts.push(t(&rolid_core::eta::format_eta(r.eta.eta(st.total.saturating_sub(st.done_bytes)))));
                        }
                    } else if r.cancel.load(Ordering::Relaxed) {
                        parts.push(tr!("canceled"));
                    } else {
                        parts.push(tr!("done"));
                    }
                    if bad > 0 {
                        parts.push(trn!("{n} error", "{n} errors", bad));
                    }
                    let text = parts.join(" · ");
                    ui.add(egui::ProgressBar::new(frac.min(1.0)).text(text));
                    let row_h = 18.0;
                    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show_rows(ui, row_h, st.rows.len(), |ui, range| {
                        for x in &st.rows[range] {
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(egui::vec2(300.0, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.set_width(300.0);
                                    ui.add(egui::Label::new(&x.rel).truncate()).on_hover_text(&x.rel);
                                });
                                match &x.sum {
                                    None => {
                                        ui.weak("…");
                                    }
                                    Some(Ok(s)) => {
                                        ui.add(egui::Label::new(egui::RichText::new(s).monospace().size(crate::fonts::fs(11.5))).truncate());
                                    }
                                    Some(Err(e)) => {
                                        ui.colored_label(super::WARN_ORANGE, e);
                                    }
                                }
                            });
                        }
                    });
                    for s in st.skipped.iter().take(3) {
                        ui.weak(tr!("Skipped: {item}", item = t(s)));
                    }
                    if st.skipped.len() > 3 {
                        ui.weak(trn!("… and {n} more skipped", "… and {n} more skipped", st.skipped.len() - 3));
                    }
                    if finished && ok > 0 {
                        if crate::look::btn(Role::Copy, tr!("Copy to clipboard")).ui(ui).clicked() {
                            copy = Some(listing_of(r.algo, &st.rows));
                        }
                        let heading = tr!("Save to file");
                        dialog::group(ui, &heading, |ui| {
                            dialog::form(ui, "sum-save-form", |ui| {
                                ui.label(tr!("Name:"));
                                ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(220.0));
                                ui.end_row();
                            });
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(!r.here_read_only, |ui| {
                                    if ui.radio(!r.to_other, tr!("to this panel (paths in the file are relative to it)")).clicked() {
                                        r.to_other = false;
                                        r.ask_overwrite = false;
                                    }
                                });
                                let other_ok = other.is_some_and(|(_, ro)| !ro);
                                ui.add_enabled_ui(other_ok, |ui| {
                                    if ui.radio(r.to_other, tr!("to the other panel")).clicked() {
                                        r.to_other = true;
                                        r.ask_overwrite = false;
                                    }
                                });
                            });
                            if r.here_read_only {
                                dialog::note(ui, tr!("An archive cannot be written to – the file will be saved to the other panel."));
                            }
                        });
                    }
                    if let Some((err, m)) = &r.msg {
                        if *err {
                            dialog::warn(ui, m.as_str(), false);
                        } else {
                            ui.label(m);
                        }
                    }
                    if r.ask_overwrite {
                        dialog::warn(ui, tr!("“{name}” already exists in the target.", name = r.name.trim()), false);
                    }
                    if !finished {
                        // Cancel stops the calculation; the dialog stays with what is done.
                        let b = dialog::buttons(ui, [dialog::cancel()]);
                        if b[0] {
                            r.cancel.store(true, Ordering::Relaxed);
                        }
                    } else if ok > 0 {
                        let label = if r.ask_overwrite { tr!("Overwrite") } else { tr!("Save to file") };
                        let role = if r.ask_overwrite { Role::Overwrite } else { Role::Save };
                        let can = !r.name.trim().is_empty() && (!r.to_other || other.is_some_and(|(_, ro)| !ro)) && (r.to_other || !r.here_read_only);
                        let b = dialog::buttons(ui, [dialog::btn(role, label).enabled(can), dialog::close()]);
                        if b[0] {
                            save = true;
                        }
                        if b[1] {
                            close = true;
                        }
                    } else if dialog::buttons(ui, [dialog::close()])[0] {
                        close = true;
                    }
                });
            }
        }
        if let Some(text) = copy {
            self.ctx.copy_text(text);
            if let Dialog::Checksum(d) = &mut self.dialog {
                if let SumDialog::Run(r) = d.as_mut() {
                    r.msg = Some((false, tr!("Copied to the clipboard.")));
                }
            }
        }
        if save {
            self.save_checksums();
        }
        if let Some(a) = start {
            self.start_checksums(a);
        } else if close {
            if let Dialog::Checksum(d) = &self.dialog {
                if let SumDialog::Run(r) = d.as_ref() {
                    r.cancel.store(true, Ordering::Relaxed);
                }
            }
            self.dialog = Dialog::None;
        }
    }

    fn save_checksums(&mut self) {
        let Dialog::Checksum(d) = &mut self.dialog else { return };
        let SumDialog::Run(r) = d.as_mut() else { return };
        let name = r.name.trim().to_string();
        if let Err(e) = rolid_core::vfs::validate_name(&name) {
            r.msg = Some((true, t(&e.text())));
            return;
        }
        let to = if r.to_other {
            let side = 1 - r.at.side;
            PaneRef { tab: self.sides[side].current().id, side }
        } else {
            r.at
        };
        let text = listing_of(r.algo, &r.state.lock().unwrap().rows);
        let Some(dst) = super::find_pane(&self.sides, to) else { return };
        let (dfs, dpath, label) = (dst.fs.clone(), dst.path.clone(), dst.display_label());
        let exists = dst.entries.iter().any(|e| e.name == name);
        if exists && !r.ask_overwrite {
            r.ask_overwrite = true;
            return;
        }
        r.ask_overwrite = false;
        if dfs.is_local_disk() {
            let path = PathBuf::from(dfs.join(&dpath, &name));
            match std::fs::write(&path, text) {
                Ok(()) => {
                    r.msg = Some((false, tr!("Saved: {path}", path = &path)));
                    self.refresh_all(&[to]);
                }
                Err(e) => r.msg = Some((true, format!("{}: {e}", path.display()))),
            }
            return;
        }
        // Server: written to a private temp folder, uploaded by the queue.
        let temp = match new_temp() {
            Ok(t) => t,
            Err(e) => {
                r.msg = Some((true, tr!("Cannot create a temporary folder: {error}", error = e)));
                return;
            }
        };
        let local = temp.path.join(&name);
        if let Err(e) = std::fs::write(&local, &text) {
            r.msg = Some((true, format!("{}: {e}", local.display())));
            return;
        }
        r.msg = Some((false, tr!("Uploading through the queue to {location}", location = format!("{label}:{dpath}"))));
        let production = dst.remote.as_ref().is_some_and(|x| x.production);
        let links = self.pane_link(to).into_iter().collect();
        self.spawn_copy(CopySpec {
            sfs: Arc::new(LocalFs),
            dfs,
            dpath,
            items: vec![Target { dir: temp.path.to_string_lossy().into_owned(), name: name.clone(), label: name.clone(), size: text.len() as u64 }],
            // An existing file was confirmed above.
            conflict: Conflict::Overwrite,
            title: format!("{name} → {label}"),
            refresh: vec![to],
            links,
            auto_retry: !production,
            state: None,
            keep_going: false,
            pre_failed: Vec::new(),
            then: None,
            temp: Some(temp),
            moving: false,
            verify: false,
            target_name: None,
        });
    }
}

/// The other panel and whether it is read-only.
fn find_other(sides: &[super::Side; 2], at: PaneRef) -> Option<(PaneRef, bool)> {
    let side = 1 - at.side;
    let r = PaneRef { tab: sides[side].current().id, side };
    super::find_pane(sides, r).map(|p| (r, p.fs.read_only() || p.search.is_some()))
}

fn listing_of(algo: Algo, rows: &[SumRow]) -> String {
    let ok: Vec<(String, String)> =
        rows.iter().filter_map(|x| match &x.sum { Some(Ok(s)) => Some((x.rel.clone(), s.clone())), _ => None }).collect();
    checksum::listing(algo, &ok)
}

/// Find the files and compute their sums, filling `state`.
#[allow(clippy::too_many_arguments)]
async fn run_checksums(
    fs: Arc<dyn Vfs>,
    conn: Option<rolid_ssh::SshConnection>,
    base: String,
    items: Vec<Target>,
    kinds: HashMap<String, bool>,
    algo: Algo,
    recursive: bool,
    state: Arc<Mutex<SumState>>,
    cancel: Arc<AtomicBool>,
) {
    let local = fs.is_local_disk();
    // Files: the selection, folders walked (links inside are not followed).
    for t in &items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let abs = fs.join(&t.dir, &t.name);
        let rel = if t.dir == base { t.name.clone() } else { t.label.clone() };
        let is_dir = match kinds.get(&t.name).filter(|_| t.dir == base) {
            Some(d) => *d,
            None => fs.stat(&abs).await.map(|e| e.kind == EntryKind::Dir).unwrap_or(false),
        };
        if !is_dir {
            let size = fs.stat(&abs).await.map(|e| e.size).unwrap_or(t.size);
            let mut s = state.lock().unwrap();
            s.total += size;
            s.rows.push(SumRow { rel, abs, size, sum: None });
            continue;
        }
        if !recursive {
            state.lock().unwrap().skipped.push(txt!("{path}/ (folder)", path = rel));
            continue;
        }
        let mut stack = vec![(abs, rel)];
        while let Some((dir, rel)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut list = match fs.list(&dir).await {
                Ok(l) => l,
                Err(e) => {
                    state.lock().unwrap().skipped.push(txt!("{path}/: {error}", path = rel, error = e.text()));
                    continue;
                }
            };
            list.sort_by(|a, b| a.name.cmp(&b.name));
            let mut s = state.lock().unwrap();
            for e in list.iter().rev().filter(|e| e.kind == EntryKind::Dir) {
                stack.push((fs.join(&dir, &e.name), format!("{rel}/{}", e.name)));
            }
            for e in list.iter().filter(|e| e.kind == EntryKind::File) {
                s.total += e.size;
                s.rows.push(SumRow { rel: format!("{rel}/{}", e.name), abs: fs.join(&dir, &e.name), size: e.size, sum: None });
            }
            for e in list.iter().filter(|e| e.kind == EntryKind::Symlink) {
                s.skipped.push(txt!("{path} (link)", path = format!("{rel}/{}", e.name)));
            }
        }
    }
    // How.
    let mut tool = false;
    if !local {
        if let (Some(c), Some(probe)) = (&conn, checksum::probe_command(algo)) {
            let mut out = Vec::new();
            // Without an answer in time: read the files over SFTP instead.
            if c.exec_probe(&probe, &cancel, &mut |d| {
                out.extend_from_slice(d);
                out.len() < 256
            })
            .await
            .is_ok()
            {
                let o = String::from_utf8_lossy(&out);
                tool = o.contains("GNU") || o.contains("B3");
            }
        }
    }
    {
        let mut s = state.lock().unwrap();
        s.listing = false;
        s.method = Some(if local {
            txt!("locally")
        } else if tool {
            txt!("{tool} on the server (SSH)", tool = algo.tool().unwrap_or(""))
        } else if fs.read_only() && !fs.is_remote() {
            txt!("in the archive (by extracting)")
        } else if let (true, Some(tool)) = (conn.is_some(), algo.tool()) {
            txt!("by downloading over SFTP ({tool} is not on the server)", tool = tool)
        } else if conn.is_some() {
            txt!("by downloading over SFTP (the tool is not on the server)")
        } else {
            txt!("by downloading over SFTP")
        });
    }
    let n = state.lock().unwrap().rows.len();
    if tool {
        if let Some(c) = &conn {
            remote_tool(c, algo, &state, &cancel).await;
        }
    } else {
        for i in 0..n {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let abs = state.lock().unwrap().rows[i].abs.clone();
            let r = if local {
                let (c, st) = (cancel.clone(), state.clone());
                tokio::task::spawn_blocking(move || {
                    checksum::file_sum(std::path::Path::new(&abs), algo, &c, &mut |b| st.lock().unwrap().done_bytes += b)
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            } else {
                stream_sum(fs.as_ref(), &abs, algo, &state, &cancel).await
            };
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            state.lock().unwrap().rows[i].sum = Some(r);
        }
    }
    tracing::info!("checksums {}: {n} files, {}", algo.label(), state.lock().unwrap().method.as_ref().map(|m| m.to_string()).unwrap_or_default());
}

/// Sum over the file system (SFTP, archive).
async fn stream_sum(fs: &dyn Vfs, path: &str, algo: Algo, state: &Mutex<SumState>, cancel: &AtomicBool) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let mut r = fs.open_read(path).await.map_err(|e| t(&e.text()))?;
    let mut h = algo.hasher();
    let mut buf = vec![0u8; 256 << 10];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(tr!("canceled"));
        }
        let n = r.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        state.lock().unwrap().done_bytes += n as u64;
    }
    Ok(h.finish())
}

/// Sums by the server's tool: GNU `*sum -z` for batches of files, `b3sum` one by one.
async fn remote_tool(conn: &rolid_ssh::SshConnection, algo: Algo, state: &Mutex<SumState>, cancel: &AtomicBool) {
    let all: Vec<(usize, String, u64)> =
        state.lock().unwrap().rows.iter().enumerate().map(|(i, r)| (i, r.abs.clone(), r.size)).collect();
    let mut batches: Vec<Vec<(usize, String, u64)>> = Vec::new();
    let mut cur = Vec::new();
    let mut len = 0;
    let per = if algo == Algo::Blake3 { 1 } else { 200 };
    for x in all {
        len += x.1.len() + 3;
        cur.push(x);
        if cur.len() >= per || len > 60_000 {
            batches.push(std::mem::take(&mut cur));
            len = 0;
        }
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    for b in batches {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let paths: Vec<String> = b.iter().map(|x| x.1.clone()).collect();
        let index: HashMap<&str, (usize, u64)> = b.iter().map(|x| (x.1.as_str(), (x.0, x.2))).collect();
        let cmd = rolid_search::remote::guarded(&checksum::remote_command(algo, &paths));
        let mut buf = Vec::new();
        let res = conn
            .exec_stream(&cmd, cancel, &mut |d| {
                buf.extend_from_slice(d);
                if algo != Algo::Blake3 {
                    while let Some(pos) = buf.iter().position(|c| *c == 0) {
                        let rec: Vec<u8> = buf.drain(..=pos).collect();
                        if let Some((sum, path)) = checksum::parse_z_record(&rec[..rec.len() - 1]) {
                            if let Some(&(i, size)) = index.get(path.as_str()) {
                                let mut s = state.lock().unwrap();
                                s.rows[i].sum = Some(Ok(sum));
                                s.done_bytes += size;
                            }
                        }
                    }
                }
                true
            })
            .await;
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let mut s = state.lock().unwrap();
        match res {
            Ok((_, err)) => {
                if algo == Algo::Blake3 {
                    let out = String::from_utf8_lossy(&buf);
                    let sum = out.trim();
                    let (i, size) = (b[0].0, b[0].2);
                    if sum.len() == 64 && sum.bytes().all(|c| c.is_ascii_hexdigit()) {
                        s.rows[i].sum = Some(Ok(sum.to_string()));
                        s.done_bytes += size;
                    } else {
                        s.rows[i].sum = Some(Err(first_line(&err)));
                    }
                } else {
                    for x in &b {
                        if s.rows[x.0].sum.is_none() {
                            let why = err.lines().find(|l| l.contains(x.1.as_str())).map(str::to_string).unwrap_or_else(|| first_line(&err));
                            s.rows[x.0].sum = Some(Err(why));
                        }
                    }
                }
            }
            Err(e) => {
                for x in &b {
                    s.rows[x.0].sum = Some(Err(t(&e.text())));
                }
            }
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().filter(|l| !l.is_empty()).map(str::to_string).unwrap_or_else(|| tr!("cannot be read"))
}
