//! Remote search by running `find` (and `grep` for content) on the server.
//!
//! Security: every user-provided value (root, masks, search text) is passed as a
//! single-quoted shell word via [`sh_quote`]; nothing is ever interpolated
//! unquoted. Records are NUL-separated, so file names with newlines or tabs
//! cannot forge results.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use rolid_core::{Entry, EntryKind, Vfs};
use rolid_ssh::SshConnection;
use rolid_text::{txt, Text};

use crate::{rel_path, search_vfs, Hit, KindFilter, Matcher, Method, NameMode, Outcome, Query, Sink};

/// Quote `s` as one POSIX shell word. Works in sh/bash/zsh/dash and fish.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escape glob metacharacters so `s` matches literally inside a `find -name` pattern.
pub fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `find -printf` format: type, target type, size, mtime, mode, path, NUL.
const FMT: &str = r"%y\t%Y\t%s\t%T@\t%m\t%p\0";

/// Server-side name filter (reduces traffic; results are re-checked locally).
fn find_name_args(q: &Query) -> String {
    let flag = if q.case_sensitive { "-name" } else { "-iname" };
    let pat = q.name.trim();
    if pat.is_empty() {
        return String::new();
    }
    let masks: Vec<String> = match q.name_mode {
        NameMode::Glob => pat
            .split(';')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(|m| if m.contains(['*', '?', '[']) { m.to_string() } else { format!("*{}*", glob_escape(m)) })
            .collect(),
        NameMode::Contains => vec![format!("*{}*", glob_escape(pat))],
        NameMode::Regex => return String::new(),
    };
    let alts: Vec<String> = masks.iter().map(|m| format!("{flag} {}", sh_quote(m))).collect();
    format!(r"\( {} \) ", alts.join(" -o "))
}

fn find_prefix(q: &Query, kind: &str) -> String {
    // Hidden entries (and whole hidden directories) are pruned on the server.
    let prune = if q.hidden { String::new() } else { r"-name '.*' -prune -o ".to_string() };
    format!("find {} -mindepth 1 {prune}\\( {kind}{} ", sh_quote(&q.root), find_name_args(q))
}

/// Command for a name-only search.
pub fn find_command(q: &Query) -> String {
    let kind = match q.kind {
        KindFilter::Files => "-type f ",
        KindFilter::Dirs => "-type d ",
        KindFilter::Any => "",
    };
    format!("{}-printf {} \\) 2>/dev/null", find_prefix(q, kind), sh_quote(FMT))
}

/// Command for a content search: `find` selects files, `grep -l` keeps the
/// matching ones, a second `find` prints their metadata in the same format.
pub fn grep_command(q: &Query) -> String {
    let mut grep = String::from("grep -IlZs");
    if !q.case_sensitive {
        grep += " -i";
    }
    grep += if q.content_regex { " -E" } else { " -F" };
    grep += &format!(" -e {} --", sh_quote(&q.content));
    let meta = sh_quote(r#"find "$@" -prune -printf "$0""#);
    format!(
        "{}-print0 \\) 2>/dev/null | xargs -0 -r {grep} 2>/dev/null | xargs -0 -r sh -c {meta} {} 2>/dev/null",
        find_prefix(q, "-type f "),
        sh_quote(FMT)
    )
}

/// Wrap `pipeline` so it is killed when the SSH channel closes.
///
/// Without a PTY, closing the channel does not stop remote processes (a `grep`
/// that prints nothing never even gets SIGPIPE). A watcher waits for EOF on the
/// channel's stdin and then terminates the pipeline: the whole process group when
/// the shell leads it (OpenSSH runs each session via `setsid`), otherwise just the
/// pipeline's subshell and its direct children. When the pipeline ends normally, the watcher is killed first.
/// Runs under `sh` explicitly, so it works whatever the login shell is.
pub fn guarded(pipeline: &str) -> String {
    let script = format!(
        r#"exec 3<&0
( {pipeline} ) </dev/null 3<&- &
p=$!
( cat <&3 >/dev/null 2>&1
  if [ "$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ')" = "$$" ]; then kill -TERM 0; else pkill -TERM -P $p; kill -TERM $p; fi
) >/dev/null 2>&1 &
w=$!
exec 3<&-
wait $p
s=$?
kill $w 2>/dev/null
exit $s"#
    );
    format!("sh -c {}", sh_quote(&script))
}

/// Parse one `FMT` record (without the trailing NUL).
pub fn parse_record(rec: &[u8]) -> Option<(String, Entry)> {
    let s = String::from_utf8_lossy(rec);
    let mut it = s.splitn(6, '\t');
    let (y, ty, size, mtime, mode, path) = (it.next()?, it.next()?, it.next()?, it.next()?, it.next()?, it.next()?);
    let kind = match y {
        "f" => EntryKind::File,
        "d" => EntryKind::Dir,
        "l" => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    let secs: f64 = mtime.parse().ok()?;
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let entry = Entry {
        name,
        kind,
        link_to_dir: kind == EntryKind::Symlink && ty == "d",
        size: if kind == EntryKind::File { size.parse().unwrap_or(0) } else { 0 },
        modified: (secs >= 0.0).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(secs)),
        mode: u32::from_str_radix(mode, 8).ok(),
    };
    Some((path.to_string(), entry))
}

/// What the server can do; probed once per search (one round trip).
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub gnu_find: bool,
    pub gnu_grep: bool,
}

pub async fn probe(conn: &SshConnection, cancel: &std::sync::atomic::AtomicBool) -> Caps {
    let mut out = Vec::new();
    let cmd = "find / -maxdepth 0 -printf F 2>/dev/null; echo; grep --version 2>/dev/null | head -n 1";
    let r = conn
        .exec_stream(cmd, cancel, &mut |d| {
            out.extend_from_slice(d);
            out.len() < 4096
        })
        .await;
    if r.is_err() {
        return Caps::default(); // no shell (e.g. SFTP-only account)
    }
    let s = String::from_utf8_lossy(&out);
    let mut lines = s.lines();
    Caps {
        gnu_find: lines.next() == Some("F"),
        gnu_grep: lines.next().is_some_and(|l| l.contains("GNU grep")),
    }
}

/// Search on a server: `find`/`grep` when available, otherwise walk via `fs` (SFTP).
pub async fn search_remote(conn: &SshConnection, fs: &dyn Vfs, q: &Query, sink: &Sink<'_>) -> Result<Outcome, Text> {
    let started = Instant::now();
    if !q.root.starts_with('/') {
        return Err(txt!("searching on a server needs an absolute path"));
    }
    let m = Matcher::new(q)?;
    let caps = probe(conn, sink.cancel).await;
    let need_grep = !q.content.is_empty();
    if !caps.gnu_find || (need_grep && !caps.gnu_grep) {
        return search_vfs(fs, q, sink).await;
    }
    let cmd = guarded(&if need_grep { grep_command(q) } else { find_command(q) });
    let buf = Mutex::new(Vec::<u8>::new());
    let mut on_data = |data: &[u8]| -> bool {
        let mut b = buf.lock().unwrap();
        b.extend_from_slice(data);
        let mut keep_going = true;
        let mut consumed = 0;
        while let Some(pos) = b[consumed..].iter().position(|c| *c == 0) {
            let rec = &b[consumed..consumed + pos];
            consumed += pos + 1;
            let Some((path, entry)) = parse_record(rec) else { continue };
            if !m.name_matches(&entry.name) || !m.meta_matches(&entry) {
                continue;
            }
            let dir = rolid_ssh::posix_parent(&path).unwrap_or_else(|| "/".into());
            let hit = Hit { rel: rel_path(&q.root, &path), dir, entry, line: None };
            if !sink.push(hit) {
                keep_going = false;
                break;
            }
        }
        b.drain(..consumed);
        keep_going && !sink.stopped()
    };
    conn.exec_stream(&cmd, sink.cancel, &mut on_data).await.map_err(|e| e.text())?;
    let mut o = sink.outcome(Method::RemoteShell, started);
    o.elapsed = started.elapsed();
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("abc"), "'abc'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote("$(rm -rf /); `x` \"y\""), "'$(rm -rf /); `x` \"y\"'");
        assert_eq!(glob_escape("a*b?[c]"), r"a\*b\?\[c\]");
    }

    /// Run the generated command through a real `sh` and check that hostile input
    /// stays literal (no command substitution, no word splitting).
    #[test]
    #[cfg(unix)]
    fn quoting_is_literal_in_real_shell() {
        let evil = "x'; touch /tmp/rolid-pwned-$$; echo '$(id) `id` \"q\" \\ \n end";
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", sh_quote(evil)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), evil);
    }

    #[test]
    fn commands() {
        let q = Query { root: "/var/www".into(), name: "*.php;conf".into(), ..Default::default() };
        let c = find_command(&q);
        assert!(c.starts_with("find '/var/www' -mindepth 1 -name '.*' -prune -o \\( \\( -iname '*.php' -o -iname '*conf*' \\) "), "{c}");
        let q = Query { root: "/r".into(), content: "a'b".into(), case_sensitive: true, hidden: true, ..Default::default() };
        let c = grep_command(&q);
        assert!(c.contains(r"grep -IlZs -F -e 'a'\''b' --"), "{c}");
        assert!(!c.contains("-prune -o"), "{c}");
    }

    #[test]
    fn records() {
        let (p, e) = parse_record(b"f\tf\t123\t1700000000.5\t644\t/a/b c\td.txt").unwrap();
        assert_eq!(p, "/a/b c\td.txt");
        assert_eq!((e.name.as_str(), e.kind, e.size, e.mode), ("b c\td.txt", EntryKind::File, 123, Some(0o644)));
        let (_, e) = parse_record(b"l\td\t7\t1.0\t777\t/x/link").unwrap();
        assert!(e.link_to_dir && e.is_dir_like());
        assert!(parse_record(b"garbage").is_none());
    }

    /// Execute generated find/grep commands locally with the system's GNU tools
    /// and compare with the expected file set.
    #[test]
    #[cfg(target_os = "linux")]
    fn commands_run_with_gnu_tools() {
        let d = std::env::temp_dir().join(format!("rolid-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub dir")).unwrap();
        std::fs::create_dir_all(d.join(".git")).unwrap();
        std::fs::write(d.join("sub dir/it's.txt"), "find THIS\n").unwrap();
        std::fs::write(d.join("a.txt"), "nothing\n").unwrap();
        std::fs::write(d.join(".git/x.txt"), "this\n").unwrap();
        let root = d.to_string_lossy().into_owned();
        let run = |cmd: String| -> Vec<String> {
            let out = std::process::Command::new("sh").arg("-c").arg(&cmd).output().unwrap();
            let mut v: Vec<String> = out
                .stdout
                .split(|b| *b == 0)
                .filter(|r| !r.is_empty())
                .map(|r| parse_record(r).expect("record").0)
                .map(|p| rel_path(&root, &p))
                .collect();
            v.sort();
            v
        };
        let q = Query { root: root.clone(), name: "*.txt".into(), ..Default::default() };
        assert_eq!(run(find_command(&q)), ["a.txt", "sub dir/it's.txt"]);
        let q = Query { root: root.clone(), content: "this".into(), ..Default::default() };
        assert_eq!(run(grep_command(&q)), ["sub dir/it's.txt"]);
        let q = Query { root: root.clone(), content: "this".into(), hidden: true, ..Default::default() };
        assert_eq!(run(grep_command(&q)), [".git/x.txt", "sub dir/it's.txt"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The guard must kill a pipeline that produces no output once stdin closes,
    /// and must not get in the way when the pipeline finishes by itself.
    #[test]
    #[cfg(target_os = "linux")]
    fn guard_kills_pipeline_on_eof() {
        use std::io::Read;
        use std::process::{Command, Stdio};
        let alive = |pid: &str| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|s| !s.contains(") Z "))
                .unwrap_or(false)
        };
        let dir = std::env::temp_dir().join(format!("rolid-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // With and without being the process-group leader (setsid = like sshd).
        for (i, leader) in [true, false].into_iter().enumerate() {
            let pidfile = dir.join(format!("pid{i}"));
            let pipe = format!("sleep 30 & echo $! > {}; wait", sh_quote(&pidfile.to_string_lossy()));
            let cmd = guarded(&pipe);
            let mut c = if leader { Command::new("setsid") } else { Command::new("sh") };
            if leader {
                c.args(["-w", "sh", "-c", &cmd]);
            } else {
                c.args(["-c", &cmd]);
            }
            let mut child = c.stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
            let mut pid = String::new();
            for _ in 0..50 {
                pid = std::fs::read_to_string(&pidfile).unwrap_or_default().trim().to_string();
                if !pid.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(alive(&pid), "sleep should run (leader={leader})");
            drop(child.stdin.take()); // = SSH channel closed
            let t = Instant::now();
            child.wait().unwrap();
            for _ in 0..40 {
                if !alive(&pid) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(!alive(&pid), "sleep must be killed (leader={leader})");
            assert!(t.elapsed() < Duration::from_secs(5));
        }
        // Normal completion: output arrives and the command exits with stdin still open.
        let mut child = Command::new("setsid")
            .args(["-w", "sh", "-c", &guarded("printf hi; exit 3")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = String::new();
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(out, "hi");
        assert_eq!(child.wait().unwrap().code(), Some(3));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
