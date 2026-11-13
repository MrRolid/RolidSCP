//! "Open with": which applications can open a file, the way the desktop sees
//! it (freedesktop.org): MIME type from the shared-mime-info glob list,
//! applications from `.desktop` files, defaults and associations from
//! `mimeapps.list`. Everything is read from disk once and kept.
//!
//! Also: files from a server that must never be run (executables, scripts,
//! `.desktop` launchers) – those are opened only in a text editor, or after an
//! explicit warning.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rolid_text::{n, txt, Text};

/// An application that can open files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    /// Desktop file ID (`org.gnome.TextEditor.desktop`).
    pub id: String,
    pub name: String,
    pub exec: String,
    pub mime: Vec<String>,
    /// Not shown in menus (still usable when it is someone's default).
    pub no_display: bool,
}

/// The applications and MIME data of this system.
#[derive(Debug, Default)]
pub struct AppDb {
    apps: Vec<DesktopApp>,
    /// (weight, MIME type, glob, case sensitive).
    globs: Vec<(u32, String, String, bool)>,
    parents: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, HashSet<String>>,
}

/// Where to look (XDG base directories, most important first).
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    /// `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`.
    pub data: Vec<PathBuf>,
    /// `$XDG_CONFIG_HOME`, then `$XDG_CONFIG_DIRS`.
    pub config: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP`, lowercase (for `<desktop>-mimeapps.list`).
    pub desktops: Vec<String>,
    /// Preferred languages for `Name[xx]` (e.g. `cs_CZ`, `cs`).
    pub langs: Vec<String>,
}

impl Dirs {
    /// From the environment, with the defaults of the XDG specification.
    pub fn from_env() -> Dirs {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = std::env::home_dir().unwrap_or_default();
        let split = |v: String| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect::<Vec<_>>();
        let mut data = vec![env("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"))];
        data.extend(split(env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into())));
        let mut config = vec![env("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"))];
        config.extend(split(env("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into())));
        let desktops = env("XDG_CURRENT_DESKTOP")
            .map(|d| d.split(':').map(|s| s.to_lowercase()).collect())
            .unwrap_or_default();
        let mut langs = Vec::new();
        if let Some(l) = env("LC_ALL").or_else(|| env("LC_MESSAGES")).or_else(|| env("LANG")) {
            let l = l.split(['.', '@']).next().unwrap_or("").to_string();
            if let Some((short, _)) = l.split_once('_') {
                langs.push(l.clone());
                langs.push(short.to_string());
            } else if !l.is_empty() {
                langs.push(l);
            }
        }
        Dirs { data, config, desktops, langs }
    }
}

impl AppDb {
    /// This system's database (loaded on first use, then kept).
    pub fn system() -> &'static AppDb {
        static DB: std::sync::OnceLock<AppDb> = std::sync::OnceLock::new();
        DB.get_or_init(|| AppDb::load(&Dirs::from_env()))
    }

    /// Start loading the system database in the background (so the menu is instant).
    pub fn preload() {
        std::thread::spawn(|| {
            let _ = AppDb::system();
        });
    }

    pub fn load(dirs: &Dirs) -> AppDb {
        let mut db = AppDb::default();
        // MIME data: the first directory with the file wins per entry.
        for d in &dirs.data {
            let m = d.join("mime");
            if let Ok(t) = std::fs::read_to_string(m.join("globs2")) {
                for line in t.lines().filter(|l| !l.starts_with('#')) {
                    let mut p = line.splitn(4, ':');
                    let (Some(w), Some(mime), Some(glob)) = (p.next(), p.next(), p.next()) else { continue };
                    let cs = p.next().is_some_and(|f| f.split(',').any(|x| x == "cs"));
                    db.globs.push((w.parse().unwrap_or(50), mime.into(), glob.into(), cs));
                }
            }
            if let Ok(t) = std::fs::read_to_string(m.join("subclasses")) {
                for (c, p) in t.lines().filter_map(|l| l.split_once(' ')) {
                    db.parents.entry(c.into()).or_default().push(p.into());
                }
            }
            if let Ok(t) = std::fs::read_to_string(m.join("aliases")) {
                for (a, c) in t.lines().filter_map(|l| l.split_once(' ')) {
                    db.aliases.entry(a.into()).or_insert_with(|| c.into());
                }
            }
        }
        // Applications: an ID found in an earlier directory hides later ones.
        let mut seen = HashSet::new();
        for d in &dirs.data {
            let base = d.join("applications");
            let mut files = Vec::new();
            collect_desktop_files(&base, &base, &mut files);
            files.sort();
            for (id, path) in files {
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(app) = std::fs::read_to_string(&path).ok().and_then(|t| parse_desktop(&id, &t, &dirs.langs)) {
                    db.apps.push(app);
                }
            }
        }
        // mimeapps.list in the order of the specification; defaults.list last.
        let mut lists = Vec::new();
        for d in &dirs.config {
            for de in &dirs.desktops {
                lists.push(d.join(format!("{de}-mimeapps.list")));
            }
            lists.push(d.join("mimeapps.list"));
        }
        for d in &dirs.data {
            let a = d.join("applications");
            for de in &dirs.desktops {
                lists.push(a.join(format!("{de}-mimeapps.list")));
            }
            lists.push(a.join("mimeapps.list"));
            lists.push(a.join("defaults.list"));
        }
        for l in lists {
            if let Ok(t) = std::fs::read_to_string(&l) {
                db.read_mimeapps(&t);
            }
        }
        db
    }

    fn read_mimeapps(&mut self, text: &str) {
        let mut group = "";
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                group = line;
                continue;
            }
            let Some((mime, ids)) = line.split_once('=') else { continue };
            let ids = ids.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from);
            let mime = mime.trim().to_string();
            match group {
                "[Default Applications]" => self.defaults.entry(mime).or_default().extend(ids),
                "[Added Associations]" => self.added.entry(mime).or_default().extend(ids),
                "[Removed Associations]" => self.removed.entry(mime).or_default().extend(ids),
                _ => {}
            }
        }
    }

    pub fn app(&self, id: &str) -> Option<&DesktopApp> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// MIME type by file name (`application/octet-stream` when unknown).
    pub fn mime_for(&self, file_name: &str) -> String {
        let lower = file_name.to_lowercase();
        let mut best: Option<(u32, usize, &str)> = None;
        for (w, mime, glob, cs) in &self.globs {
            let name = if *cs { file_name } else { lower.as_str() };
            let glob_cmp = if *cs { glob.clone() } else { glob.to_lowercase() };
            if glob_match(&glob_cmp, name) {
                let key = (*w, glob.len());
                if best.is_none_or(|(bw, bl, _)| key > (bw, bl)) {
                    best = Some((key.0, key.1, mime));
                }
            }
        }
        best.map_or_else(|| "application/octet-stream".into(), |(_, _, m)| self.canonical(m))
    }

    fn canonical(&self, mime: &str) -> String {
        self.aliases.get(mime).cloned().unwrap_or_else(|| mime.to_string())
    }

    /// The MIME type and its parents (subclasses), nearest first; text types
    /// end with `text/plain`, everything with `application/octet-stream`.
    fn lineage(&self, mime: &str) -> Vec<String> {
        let mut out = vec![self.canonical(mime)];
        let mut i = 0;
        while i < out.len() {
            if let Some(ps) = self.parents.get(&out[i]) {
                for p in ps {
                    let p = self.canonical(p);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
            i += 1;
        }
        if out[0].starts_with("text/") && !out.iter().any(|m| m == "text/plain") {
            out.push("text/plain".into());
        }
        out
    }

    /// Applications for a MIME type: the default first, then the others.
    /// Terminal applications and hidden ones are left out (unless set as default).
    pub fn apps_for(&self, mime: &str) -> Vec<&DesktopApp> {
        let mut out: Vec<&DesktopApp> = Vec::new();
        for m in self.lineage(mime) {
            let removed = self.removed.get(&m);
            let ok = |id: &str| removed.is_none_or(|r| !r.contains(id));
            let listed = self
                .defaults
                .get(&m)
                .into_iter()
                .flatten()
                .chain(self.added.get(&m).into_iter().flatten())
                .filter(|id| ok(id))
                .filter_map(|id| self.app(id));
            let claiming = self.apps.iter().filter(|a| !a.no_display && a.mime.contains(&m) && ok(&a.id));
            for a in listed.chain(claiming) {
                if !out.iter().any(|x| x.id == a.id) {
                    out.push(a);
                }
            }
        }
        out
    }

    /// The default application (first of [`AppDb::apps_for`]).
    pub fn default_for(&self, mime: &str) -> Option<&DesktopApp> {
        self.apps_for(mime).into_iter().next()
    }
}

fn collect_desktop_files(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for de in rd.flatten() {
        let p = de.path();
        if p.is_dir() {
            collect_desktop_files(base, &p, out);
        } else if p.extension().is_some_and(|e| e == "desktop") {
            if let Ok(rel) = p.strip_prefix(base) {
                // Desktop file ID: path below applications/ with '/' → '-'.
                out.push((rel.to_string_lossy().replace('/', "-"), p));
            }
        }
    }
}

/// `*.txt`, `README`, `*.tar.gz`, `Makefile.*`-style globs (`*` and `?`).
fn glob_match(glob: &str, name: &str) -> bool {
    fn m(g: &[char], n: &[char]) -> bool {
        match (g.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => m(&g[1..], n) || (!n.is_empty() && m(g, &n[1..])),
            (Some('?'), Some(_)) => m(&g[1..], &n[1..]),
            (Some('['), _) => false, // character classes: rare in globs2, not supported
            (Some(a), Some(b)) if a == b => m(&g[1..], &n[1..]),
            _ => false,
        }
    }
    let g: Vec<char> = glob.chars().collect();
    let n: Vec<char> = name.chars().collect();
    m(&g, &n)
}

/// The `[Desktop Entry]` of an application, `None` for anything else (links,
/// hidden entries, terminal programs).
pub fn parse_desktop(id: &str, text: &str, langs: &[String]) -> Option<DesktopApp> {
    let mut in_entry = false;
    let mut kv: HashMap<String, String> = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim().to_string()).or_insert_with(|| unescape_value(v.trim()));
        }
    }
    let yes = |k: &str| kv.get(k).is_some_and(|v| v == "true");
    if kv.get("Type").map(String::as_str) != Some("Application") || yes("Hidden") || yes("Terminal") {
        return None;
    }
    let exec = kv.get("Exec")?.clone();
    let name = langs
        .iter()
        .find_map(|l| kv.get(&format!("Name[{l}]")))
        .or_else(|| kv.get("Name"))
        .cloned()
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
    let mime = kv
        .get("MimeType")
        .map(|m| m.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    Some(DesktopApp { id: id.to_string(), name, exec, mime, no_display: yes("NoDisplay") })
}

/// Escapes of desktop-entry string values: `\s \n \t \r \\`.
fn unescape_value(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Program and arguments from an `Exec` line for opening `file`: quoting as in
/// the Desktop Entry specification, `%f %F %u %U` = the file (appended when
/// missing), `%%` = `%`, other field codes dropped. No shell is involved.
pub fn exec_argv(exec: &str, file: &Path) -> Result<Vec<String>, Text> {
    let path = file.to_string_lossy();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => match it.next() {
                Some(n) => cur.push(n),
                None => return Err(txt!("Exec ends with the character \\")),
            },
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quoted {
        return Err(txt!("unclosed quotes in Exec"));
    }
    if in_word {
        words.push(cur);
    }
    let mut out = Vec::new();
    let mut used = false;
    for w in words {
        let mut s = String::new();
        let mut chars = w.chars();
        let mut dropped = false;
        while let Some(c) = chars.next() {
            if c != '%' {
                s.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => s.push('%'),
                Some('f' | 'F' | 'u' | 'U') => {
                    s.push_str(&path);
                    used = true;
                }
                Some(_) | None => dropped = true, // %i %c %k and deprecated codes
            }
        }
        if !(dropped && s.is_empty()) {
            out.push(s);
        }
    }
    if out.is_empty() {
        return Err(txt!("empty Exec command"));
    }
    if !used {
        out.push(path.into_owned());
    }
    Ok(out)
}

/// Start a program without waiting for it (a thread reaps the process).
pub fn spawn(argv: &[String]) -> Result<(), Text> {
    let (prog, args) = argv.split_first().ok_or_else(|| txt!("empty command"))?;
    let mut child = std::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| txt!("“{prog}” cannot be started: {error}", prog = prog, error = e.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The system's own "open this file" command.
pub fn system_open_argv(file: &Path) -> Vec<String> {
    let f = file.to_string_lossy().into_owned();
    if cfg!(windows) {
        // `start` is a cmd built-in; the empty "" is the window title.
        vec!["cmd".into(), "/C".into(), "start".into(), String::new(), f]
    } else if cfg!(target_os = "macos") {
        vec!["open".into(), f]
    } else {
        vec!["xdg-open".into(), f]
    }
}

/// Windows' own "Open with" chooser (registered applications).
pub fn windows_chooser_argv(file: &Path) -> Vec<String> {
    vec!["rundll32.exe".into(), "shell32.dll,OpenAs_RunDLL".into(), file.to_string_lossy().into_owned()]
}

/// Extensions of files that run something when opened.
const RISKY_EXT: &[&str] = &[
    "sh", "bash", "zsh", "csh", "ksh", "fish", "command", "run", "bin", "appimage", "desktop", "exe", "msi", "com",
    "bat", "cmd", "ps1", "vbs", "js", "jar", "py", "pl", "rb", "lnk", "scr",
];

/// Why a file from a server must not be opened by just any application:
/// executable bit, script (`#!`), launcher (`.desktop`), program extension.
/// `head` = the first bytes of the file. `None` = harmless. The reason is
/// English, marked with `n!` (the UI translates it).
pub fn risky(name: &str, mode: Option<u32>, head: &[u8]) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    if ext == "desktop" {
        Some(n!("is an application launcher (.desktop)"))
    } else if head.starts_with(b"#!") {
        Some(n!("is a script (starts with #!)"))
    } else if mode.is_some_and(|m| m & 0o111 != 0) {
        Some(n!("has the execute permission on the server"))
    } else if RISKY_EXT.contains(&ext.as_str()) {
        Some(n!("is a program or a script (by its extension)"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rolid-apps-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: PathBuf, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn app(name: &str, exec: &str, mime: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nName[cs]={name} CZ\nExec={exec}\nMimeType={mime}\n{extra}\n[Desktop Action new]\nName=Ignored\nExec=nothing\n")
    }

    /// A small system: user dir (local) overrides the system dir (usr).
    fn system(tag: &str) -> (PathBuf, Dirs) {
        let d = tmp(tag);
        let usr = d.join("usr");
        let local = d.join("local");
        let cfg = d.join("config");
        write(
            usr.join("mime/globs2"),
            "# comment\n50:text/plain:*.txt\n50:text/x-log:*.log\n50:application/x-shellscript:*.sh\n\
             50:image/png:*.png\n55:application/x-compressed-tar:*.tar.gz\n50:application/gzip:*.gz\n\
             50:text/x-makefile:makefile:cs\n",
        );
        write(usr.join("mime/subclasses"), "text/x-log text/plain\napplication/x-shellscript text/plain\n");
        write(usr.join("mime/aliases"), "text/x-logfile text/x-log\n");
        write(usr.join("applications/gedit.desktop"), &app("Gedit", "gedit %U", "text/plain;", ""));
        write(usr.join("applications/kate.desktop"), &app("Kate", "kate -b %U", "text/plain;text/x-log;", ""));
        write(usr.join("applications/vim.desktop"), &app("Vim", "vim %F", "text/plain;", "Terminal=true"));
        write(usr.join("applications/hidden.desktop"), &app("Hidden", "h %f", "text/plain;", "Hidden=true"));
        write(usr.join("applications/viewer.desktop"), &app("Viewer", "viewer %f", "image/png;", ""));
        write(usr.join("applications/kde/okular.desktop"), &app("Okular", "okular %U", "image/png;", ""));
        write(usr.join("applications/old.desktop"), &app("Old", "old %f", "image/png;", ""));
        // The user's own copy hides the system one.
        write(local.join("applications/viewer.desktop"), &app("My Viewer", "myviewer %f", "image/png;", ""));
        write(
            cfg.join("mimeapps.list"),
            "[Default Applications]\nimage/png=missing.desktop;kde-okular.desktop;\n\
             [Added Associations]\ntext/x-log=gedit.desktop;\n\
             [Removed Associations]\nimage/png=old.desktop;\n",
        );
        write(cfg.join("x-cinnamon-mimeapps.list"), "[Default Applications]\ntext/plain=kate.desktop\n");
        let dirs = Dirs {
            data: vec![local, usr],
            config: vec![cfg],
            desktops: vec!["x-cinnamon".into()],
            langs: vec!["cs_CZ".into(), "cs".into()],
        };
        (d, dirs)
    }

    #[test]
    fn mime_from_the_glob_list() {
        let (d, dirs) = system("db-glob");
        let db = AppDb::load(&dirs);
        assert_eq!(db.mime_for("Notes.TXT"), "text/plain", "case-insensitive");
        assert_eq!(db.mime_for("backup.tar.gz"), "application/x-compressed-tar", "higher weight wins");
        assert_eq!(db.mime_for("x.gz"), "application/gzip");
        assert_eq!(db.mime_for("makefile"), "text/x-makefile");
        assert_eq!(db.mime_for("Makefile"), "application/octet-stream", "case-sensitive glob");
        assert_eq!(db.mime_for("no-extension"), "application/octet-stream");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn apps_for_a_type_default_first() {
        let (d, dirs) = system("db-apps");
        let db = AppDb::load(&dirs);
        let ids = |m: &str| db.apps_for(m).iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        // Desktop-specific default (Cinnamon) first; terminal and hidden apps left out.
        assert_eq!(ids("text/plain"), ["kate.desktop", "gedit.desktop"]);
        // Added association, own type, then the parent type's apps.
        assert_eq!(ids("text/x-log"), ["gedit.desktop", "kate.desktop"]);
        // A missing default is skipped; IDs of subdirectories use '-'; removed
        // associations are hidden; the user's copy hides the system one.
        assert_eq!(ids("image/png"), ["kde-okular.desktop", "viewer.desktop"]);
        assert_eq!(db.app("viewer.desktop").unwrap().name, "My Viewer CZ", "the user's copy");
        assert_eq!(db.app("kate.desktop").unwrap().name, "Kate CZ", "localized name");
        assert_eq!(db.default_for("text/x-logfile").unwrap().id, "gedit.desktop", "alias");
        assert!(db.apps_for("application/x-unknown").is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn exec_lines_become_argument_lists() {
        let f = Path::new("/run/user/1000/rolidscp-edit/ab/my file.txt");
        let file = "/run/user/1000/rolidscp-edit/ab/my file.txt";
        assert_eq!(exec_argv("gedit %U", f).unwrap(), ["gedit", file]);
        assert_eq!(exec_argv("kate -b %U %i %c", f).unwrap(), ["kate", "-b", file]);
        assert_eq!(exec_argv("viewer", f).unwrap(), ["viewer", file], "appended");
        assert_eq!(
            exec_argv(r#""/opt/My App/app" --open=%f --pct=100%%"#, f).unwrap(),
            ["/opt/My App/app", &format!("--open={file}"), "--pct=100%"]
        );
        assert_eq!(exec_argv(r#"sh -c "echo \"\$1\"""#, f).unwrap(), ["sh", "-c", "echo \"$1\"", file]);
        assert!(exec_argv("\"oops", f).is_err());
        // A file name is never split or interpreted (no shell).
        let evil = Path::new("/tmp/a; rm -rf ~ $(x).txt");
        assert_eq!(exec_argv("gedit %f", evil).unwrap(), ["gedit", "/tmp/a; rm -rf ~ $(x).txt"]);
    }

    #[test]
    fn files_that_could_run_something() {
        assert!(risky("deploy.sh", Some(0o644), b"echo").is_some());
        assert!(risky("tool", Some(0o755), b"\x7fELF").is_some());
        assert!(risky("no-extension", Some(0o644), b"#!/bin/sh\n").is_some());
        assert!(risky("Launch.DESKTOP", Some(0o644), b"[Desktop Entry]").is_some());
        assert!(risky("index.php", Some(0o640), b"<?php").is_none());
        assert!(risky("photo.png", Some(0o644), b"\x89PNG").is_none());
    }
}
