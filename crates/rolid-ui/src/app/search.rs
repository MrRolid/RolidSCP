//! Search dialog (Ctrl+F7 / Ctrl+F / Alt+F7), running searches and their history.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use rolid_search::{parse_size, remote, search_local, search_vfs, Hit, KindFilter, NameMode, Outcome, Query, Sink};
use serde::{Deserialize, Serialize};

use super::{Action, App, Dialog, Msg, PaneRef};
use crate::dialog;
use crate::look::Role;

const HISTORY_FILE: &str = "search_history.toml";
const HISTORY_MAX: usize = 20;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    #[serde(default, rename = "query")]
    pub list: Vec<Query>,
}

pub struct SearchForm {
    pub at: PaneRef,
    pub root: String,
    pub name: String,
    pub name_mode: NameMode,
    pub case_sensitive: bool,
    pub content: String,
    pub content_regex: bool,
    pub min_size: String,
    pub max_size: String,
    pub days: String,
    pub kind: KindFilter,
    pub hidden: bool,
    pub gitignore: bool,
    pub remote: bool,
    pub error: Option<String>,
    pub focus: bool,
}

impl SearchForm {
    fn fill_from(&mut self, q: &Query) {
        self.name = q.name.clone();
        self.name_mode = q.name_mode;
        self.case_sensitive = q.case_sensitive;
        self.content = q.content.clone();
        self.content_regex = q.content_regex;
        self.min_size = q.min_size.map(|n| n.to_string()).unwrap_or_default();
        self.max_size = q.max_size.map(|n| n.to_string()).unwrap_or_default();
        self.days = q.modified_days.map(|n| n.to_string()).unwrap_or_default();
        self.kind = q.kind;
        self.hidden = q.hidden;
        self.gitignore = q.gitignore;
    }

    fn to_query(&self) -> Result<Query, String> {
        let days = match self.days.trim() {
            "" => None,
            d => Some(d.parse::<u32>().map_err(|_| tr!("Invalid number of days “{days}”", days = d))?),
        };
        let q = Query {
            root: self.root.trim().to_string(),
            name: self.name.trim().to_string(),
            name_mode: self.name_mode,
            case_sensitive: self.case_sensitive,
            content: self.content.clone(),
            content_regex: self.content_regex,
            min_size: parse_size(&self.min_size).map_err(|e| rolid_i18n::t(&e))?,
            max_size: parse_size(&self.max_size).map_err(|e| rolid_i18n::t(&e))?,
            modified_days: days,
            kind: self.kind,
            hidden: self.hidden,
            gitignore: self.gitignore,
            max_results: 10_000,
        };
        if q.root.is_empty() {
            return Err(tr!("Enter where to search."));
        }
        // Validate patterns before starting.
        rolid_search::Matcher::new(&q).map_err(|e| rolid_i18n::t(&e))?;
        Ok(q)
    }
}

fn describe(q: &Query) -> String {
    let mut s = Vec::new();
    if !q.name.is_empty() {
        s.push(q.name.clone());
    }
    if !q.content.is_empty() {
        s.push(tr!("“{text}”", text = &q.content));
    }
    if s.is_empty() {
        s.push(tr!("(everything)"));
    }
    s.join(" + ")
}

impl App {
    pub(super) fn load_search_history(&mut self) {
        match rolid_config::load_file::<History>(HISTORY_FILE) {
            Ok(h) => self.search_history = h.list,
            Err(e) => self.last_error = Some(tr!("Search history: {error}", error = e)),
        }
    }

    fn remember_query(&mut self, q: &Query) {
        let same = |a: &Query| Query { root: String::new(), ..a.clone() } == Query { root: String::new(), ..q.clone() };
        self.search_history.retain(|h| !same(h));
        self.search_history.insert(0, q.clone());
        self.search_history.truncate(HISTORY_MAX);
        let h = History { list: self.search_history.clone() };
        if let Err(e) = rolid_config::save_file(HISTORY_FILE, &h) {
            self.last_error = Some(tr!("Search history: {error}", error = e));
        }
    }

    pub(super) fn clear_search_history(&mut self) {
        self.search_history.clear();
        if let Err(e) = rolid_config::save_file(HISTORY_FILE, &History::default()) {
            self.last_error = Some(tr!("Search history: {error}", error = e));
        }
    }

    pub(super) fn open_search_dialog(&mut self) {
        let at = self.active_ref();
        let Some(p) = self.pane(at) else { return };
        let root = p.search.as_ref().map(|s| s.query.root.clone()).unwrap_or_else(|| p.path.clone());
        let mut f = SearchForm {
            at,
            root,
            name: String::new(),
            name_mode: NameMode::Glob,
            case_sensitive: false,
            content: String::new(),
            content_regex: false,
            min_size: String::new(),
            max_size: String::new(),
            days: String::new(),
            kind: KindFilter::Any,
            hidden: false,
            gitignore: false,
            remote: p.remote.is_some(),
            error: None,
            focus: true,
        };
        // Like Total Commander: start from the last search.
        let last = p.search.as_ref().map(|s| s.query.clone()).or_else(|| self.search_history.first().cloned());
        if let Some(q) = last {
            f.fill_from(&q);
        }
        self.dialog = Dialog::Search(Box::new(f));
    }

    pub(super) fn start_search(&mut self, at: PaneRef, q: Query) {
        let Some(p) = self.pane(at) else { return };
        if p.offline {
            self.last_error = Some(tr!("The panel is not connected – connecting, try the search again."));
            self.connect_pane(at);
            return;
        }
        self.remember_query(&q);
        let id = self.id();
        let cancel = Arc::new(AtomicBool::new(false));
        let conn = self
            .pane(at)
            .and_then(|p| p.remote.as_ref())
            .and_then(|r| self.live_connection(&r.target.key()));
        let Some(p) = self.pane_mut(at) else { return };
        // Inside an archive the search walks the virtual folder like a remote one.
        let remote = p.remote.is_some() || p.fs.read_only();
        let fs = p.fs.clone();
        let pending = p.begin_search(id, q.clone(), cancel.clone());
        let ctx = self.ctx.clone();
        let emit = move |h: Hit| {
            pending.lock().unwrap().push(h);
            ctx.request_repaint_after(Duration::from_millis(100));
        };
        let max = q.max_results;
        self.spawn(async move {
            let result = if remote {
                let sink = Sink::new(&emit, max, &cancel);
                match conn {
                    Some(c) => remote::search_remote(&c, fs.as_ref(), &q, &sink).await,
                    None => search_vfs(fs.as_ref(), &q, &sink).await,
                }
            } else {
                tokio::task::spawn_blocking(move || {
                    let sink = Sink::new(&emit, max, &cancel);
                    search_local(&q, &sink)
                })
                .await
                .unwrap_or_else(|e| Err(rolid_core::vfs::raw_text(e)))
            };
            Msg::SearchDone { at, id, result: result.map_err(|e| rolid_i18n::t(&e)) }
        });
    }

    pub(super) fn on_search_done(&mut self, at: PaneRef, id: u64, result: Result<Outcome, String>) {
        let summary = match &result {
            Ok(o) => {
                let secs = rolid_i18n::decimal(o.elapsed.as_secs_f64(), 1);
                let method = rolid_i18n::t_str(o.method.label());
                let s = if o.canceled {
                    trn!(
                        "Stopped: {n} result in {secs} s ({method})",
                        "Stopped: {n} results in {secs} s ({method})",
                        o.hits,
                        secs = secs,
                        method = method
                    )
                } else {
                    trn!(
                        "Done: {n} result in {secs} s ({method})",
                        "Done: {n} results in {secs} s ({method})",
                        o.hits,
                        secs = secs,
                        method = method
                    )
                };
                if o.truncated {
                    tr!("{summary} – only up to the limit is shown, refine the search", summary = s)
                } else {
                    s
                }
            }
            Err(e) => tr!("Search error: {error}", error = e),
        };
        if let Err(e) = &result {
            self.last_error = Some(tr!("Search: {error}", error = e));
        }
        if let Some(p) = self.pane_mut(at) {
            p.finish_search(id, summary);
        }
    }

    /// Pull streamed hits into panels; keep repainting while searches run.
    pub(super) fn drain_searches(&mut self, ctx: &egui::Context) {
        let mut running = false;
        for side in &mut self.sides {
            for p in side.tabs.iter_mut().map(|t| &mut t.pane) {
                if p.search.is_some() {
                    p.drain_search();
                    running |= p.search.as_ref().is_some_and(|s| s.running);
                }
            }
        }
        if running {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    /// Re-run the search shown in panel `at` (Ctrl+R).
    pub(super) fn rerun_search(&mut self, at: PaneRef) {
        if let Some(q) = self.pane(at).and_then(|p| p.search.as_ref()).map(|s| s.query.clone()) {
            self.start_search(at, q);
        }
    }

    pub(super) fn search_dialog_ui(&mut self, ctx: &egui::Context) {
        let mut act: Option<Action> = None;
        let mut close = false;
        let history = &self.search_history;
        let Dialog::Search(f) = &mut self.dialog else { return };

        dialog::window("search", tr!("Find files")).icon(Role::Search).min_width(560.0).show(ctx, |ui| {
            dialog::form(ui, "sf", |ui| {
                ui.label(tr!("Search in:"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.root).desired_width(420.0));
                    if f.remote {
                        dialog::help(ui, tr!("On the server the search uses find/grep (fast); without a shell it walks the folders over SFTP."));
                    }
                });
                ui.end_row();

                ui.label(tr!("File name:"));
                ui.vertical(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut f.name)
                            .hint_text(tr!("*.rs;*.toml  ·  word = contains  ·  empty = all"))
                            .desired_width(420.0),
                    );
                    if f.focus {
                        r.request_focus();
                        f.focus = false;
                    }
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.name_mode, NameMode::Glob, tr!("Wildcards"));
                        ui.radio_value(&mut f.name_mode, NameMode::Contains, tr!("Contains"));
                        ui.radio_value(&mut f.name_mode, NameMode::Regex, tr!("Regular expression"));
                    });
                });
                ui.end_row();

                ui.label(tr!("Text in the file:"));
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.content).hint_text(tr!("optional")).desired_width(420.0));
                    ui.checkbox(&mut f.content_regex, tr!("Regular expression"));
                });
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut f.case_sensitive, tr!("Case sensitive"));
                ui.end_row();

                if !history.is_empty() {
                    ui.label(tr!("History:"));
                    egui::ComboBox::from_id_salt("search-history")
                        .selected_text(tr!("choose an earlier search…"))
                        .width(420.0)
                        .show_ui(ui, |ui| {
                            for q in history {
                                if ui.selectable_label(false, describe(q)).clicked() {
                                    f.fill_from(q);
                                }
                            }
                        });
                    ui.end_row();
                }
            });
            let heading = tr!("Filters");
            dialog::group(ui, &heading, |ui| {
                dialog::form(ui, "sf-filters", |ui| {
                    ui.label(tr!("Size:"));
                    ui.horizontal(|ui| {
                        ui.label(trc!("size range", "from"));
                        ui.add(egui::TextEdit::singleline(&mut f.min_size).hint_text(tr!("e.g. 10k")).desired_width(80.0));
                        ui.label(trc!("size range", "to"));
                        ui.add(egui::TextEdit::singleline(&mut f.max_size).hint_text(tr!("e.g. 5M")).desired_width(80.0));
                    });
                    ui.end_row();

                    ui.label(tr!("Modified:"));
                    ui.horizontal(|ui| {
                        ui.label(trc!("modified", "in the last"));
                        ui.add(egui::TextEdit::singleline(&mut f.days).desired_width(50.0));
                        ui.label(trc!("modified", "days"));
                    });
                    ui.end_row();

                    ui.label(tr!("Type:"));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.kind, KindFilter::Any, trc!("kind", "All"));
                        ui.radio_value(&mut f.kind, KindFilter::Files, trc!("kind", "Files"));
                        ui.radio_value(&mut f.kind, KindFilter::Dirs, trc!("kind", "Folders"));
                    });
                    ui.end_row();

                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut f.hidden, tr!("Include hidden"));
                        ui.add_enabled_ui(!f.remote, |ui| {
                            ui.checkbox(&mut f.gitignore, tr!("Respect .gitignore"))
                                .on_disabled_hover_text(tr!("Only when searching on a local disk"));
                        });
                    });
                    ui.end_row();
                });
            });
            if let Some(e) = &f.error {
                dialog::warn(ui, e.as_str(), true);
            }
            let b = dialog::buttons(ui, [dialog::btn(Role::Search, tr!("Search")).default(true), dialog::cancel()]);
            if b[0] {
                match f.to_query() {
                    Ok(q) => {
                        let at = f.at;
                        act = Some(Box::new(move |a: &mut App| a.start_search(at, q)));
                        close = true;
                    }
                    Err(e) => f.error = Some(e),
                }
            }
            if b[1] {
                close = true;
            }
        });
        if close {
            self.dialog = Dialog::None;
        }
        if let Some(a) = act {
            a(self);
        }
    }
}
