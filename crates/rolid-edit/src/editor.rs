//! External editor command: `program args… {file}` (the placeholder is replaced by
//! the file path; without it the path is appended).
//!
//! Terminal editors (vi, nano, mcedit…) are never started directly from the GUI –
//! only through an explicit terminal command, e.g.
//! `x-terminal-emulator -e nano {file}`.

use std::path::Path;

use rolid_text::{txt, Text};

pub const PLACEHOLDER: &str = "{file}";

/// Editors that need a terminal.
const TERMINAL_EDITORS: &[&str] = &[
    "vi", "vim", "nvim", "view", "nano", "pico", "mcedit", "micro", "joe", "jed", "ne", "ed", "ex",
    "hx", "helix", "kak", "emacs-nox", "mg", "tilde", "dte",
];
/// Graphical editors accepted from `$VISUAL` / `$EDITOR`.
const GUI_EDITORS: &[&str] = &[
    "gedit", "gnome-text-editor", "kate", "kwrite", "mousepad", "xed", "pluma", "leafpad",
    "featherpad", "geany", "code", "codium", "code-oss", "subl", "sublime_text", "gvim", "nvim-qt",
    "zed", "lite-xl", "notepadqq", "bluefish", "notepad", "notepad++", "xdg-open",
];
/// Terminal emulators: an explicit terminal command may run a terminal editor.
const TERMINALS: &[&str] = &[
    "x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "mate-terminal", "xterm",
    "uxterm", "tilix", "terminator", "alacritty", "kitty", "foot", "wezterm", "lxterminal", "qterminal",
    "urxvt", "st", "sakura", "guake",
];
/// Linux defaults, in order.
const LINUX_DEFAULTS: &[&str] = &["gedit", "kate", "mousepad", "xed", "code --wait"];

fn base(prog: &str) -> String {
    let b = Path::new(prog).file_name().and_then(|s| s.to_str()).unwrap_or(prog);
    b.strip_suffix(".exe").unwrap_or(b).to_ascii_lowercase()
}

pub fn is_terminal_editor(prog: &str) -> bool {
    TERMINAL_EDITORS.contains(&base(prog).as_str())
}

pub fn is_gui_editor(prog: &str) -> bool {
    GUI_EDITORS.contains(&base(prog).as_str())
}

/// Split a command line into words: spaces separate, '…' and "…" quote, \ escapes.
pub fn split_command(cmd: &str) -> Result<Vec<String>, Text> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => match chars.next() {
                Some(n) => cur.push(n),
                None => return Err(txt!("the command ends with the character \\")),
            },
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
        if quote.is_some() {
            in_word = true;
        }
    }
    if quote.is_some() {
        return Err(txt!("unclosed quotes in the command"));
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// Program and arguments for opening `file`. Refuses a terminal editor that is not
/// run through a terminal emulator.
pub fn build(cmd: &str, file: &Path) -> Result<(String, Vec<String>), Text> {
    let words = split_command(cmd)?;
    let Some((prog, rest)) = words.split_first() else {
        return Err(txt!("the editor command is empty"));
    };
    if is_terminal_editor(prog) {
        return Err(txt!(
            "“{prog}” is a terminal editor and cannot be started directly from a graphical application. Set a command with a terminal, e.g. “x-terminal-emulator -e {prog} {placeholder}”.",
            prog = prog,
            placeholder = PLACEHOLDER
        ));
    }
    let path = file.to_string_lossy();
    let mut args: Vec<String> = rest.iter().map(|a| a.replace(PLACEHOLDER, &path)).collect();
    if !rest.iter().any(|a| a.contains(PLACEHOLDER)) {
        args.push(path.into_owned());
    }
    Ok((prog.clone(), args))
}

/// Default command. `env` reads environment variables, `exists` says whether a
/// program is on the PATH.
pub fn default_command(env: impl Fn(&str) -> Option<String>, exists: impl Fn(&str) -> bool) -> String {
    if cfg!(windows) {
        return format!("notepad {PLACEHOLDER}");
    }
    if cfg!(target_os = "macos") {
        return format!("open -t {PLACEHOLDER}");
    }
    for var in ["VISUAL", "EDITOR"] {
        if let Some(v) = env(var).filter(|v| !v.trim().is_empty()) {
            let words = split_command(&v).unwrap_or_default();
            if let Some(p) = words.first() {
                if is_gui_editor(p) && !is_terminal_editor(p) && exists(p) {
                    return format!("{} {PLACEHOLDER}", v.trim());
                }
            }
        }
    }
    for c in LINUX_DEFAULTS {
        let prog = c.split_whitespace().next().unwrap_or(c);
        if exists(prog) {
            return format!("{c} {PLACEHOLDER}");
        }
    }
    format!("xdg-open {PLACEHOLDER}")
}

/// Is `prog` an executable on the PATH (or an existing path)?
pub fn on_path(prog: &str) -> bool {
    let p = Path::new(prog);
    if p.components().count() > 1 {
        return p.is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|d| {
                let f = d.join(prog);
                f.is_file() || (cfg!(windows) && d.join(format!("{prog}.exe")).is_file())
            })
        })
        .unwrap_or(false)
}

/// Default for this system.
pub fn detect_default() -> String {
    default_command(|v| std::env::var(v).ok(), on_path)
}

/// Is the program of `cmd` a terminal emulator? (informational)
pub fn uses_terminal(cmd: &str) -> bool {
    split_command(cmd).ok().and_then(|w| w.first().map(|p| TERMINALS.contains(&base(p).as_str()))).unwrap_or(false)
}

/// Start the editor without waiting for it (a thread reaps the process).
pub fn launch(cmd: &str, file: &Path) -> Result<(), Text> {
    let (prog, args) = build(cmd, file)?;
    let mut child = std::process::Command::new(&prog)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| txt!("editor “{prog}” cannot be started: {error}", prog = &prog, error = e.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting() {
        assert_eq!(split_command("code --wait {file}").unwrap(), ["code", "--wait", "{file}"]);
        assert_eq!(
            split_command(r#""/opt/My Editor/ed" -n '{file}' a\ b"#).unwrap(),
            ["/opt/My Editor/ed", "-n", "{file}", "a b"]
        );
        assert_eq!(split_command("  gedit  ").unwrap(), ["gedit"]);
        assert_eq!(split_command(r#"x "" y"#).unwrap(), ["x", "", "y"]);
        assert!(split_command("gedit 'oops").is_err());
    }

    #[test]
    fn placeholder() {
        let f = Path::new("/run/user/1000/rolidscp-edit/ab/my file.php");
        let (p, a) = build("code --wait {file}", f).unwrap();
        assert_eq!(p, "code");
        assert_eq!(a, ["--wait", "/run/user/1000/rolidscp-edit/ab/my file.php"]);
        let (_, a) = build("gedit", f).unwrap();
        assert_eq!(a, ["/run/user/1000/rolidscp-edit/ab/my file.php"], "appended without placeholder");
        let (_, a) = build("subl --file={file}:1", f).unwrap();
        assert_eq!(a, ["--file=/run/user/1000/rolidscp-edit/ab/my file.php:1"]);
    }

    #[test]
    fn terminal_editors_need_a_terminal() {
        let f = Path::new("/tmp/x");
        for c in ["vi {file}", "nano", "/usr/bin/mcedit {file}", "vim"] {
            assert!(build(c, f).is_err(), "{c}");
        }
        let (p, a) = build("x-terminal-emulator -e nano {file}", f).unwrap();
        assert_eq!((p.as_str(), a.as_slice()), ("x-terminal-emulator", &["-e".to_string(), "nano".into(), "/tmp/x".into()][..]));
        assert!(uses_terminal("x-terminal-emulator -e nano {file}"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn default_order() {
        let none = |_: &str| None;
        // Nothing installed: xdg-open.
        assert_eq!(default_command(none, |_| false), "xdg-open {file}");
        // First available in order gedit, kate, mousepad, xed, code --wait.
        assert_eq!(default_command(none, |p| p == "xed" || p == "code"), "xed {file}");
        assert_eq!(default_command(none, |p| p == "code"), "code --wait {file}");
        // $EDITOR only when graphical and installed.
        let env = |v: &str| (v == "EDITOR").then(|| "nano".to_string());
        assert_eq!(default_command(env, |p| p == "nano" || p == "kate"), "kate {file}");
        let env = |v: &str| (v == "VISUAL").then(|| "geany".to_string());
        assert_eq!(default_command(env, |p| p == "geany" || p == "gedit"), "geany {file}");
        let env = |v: &str| (v == "VISUAL").then(|| "geany".to_string());
        assert_eq!(default_command(env, |p| p == "gedit"), "gedit {file}", "not installed");
    }
}
