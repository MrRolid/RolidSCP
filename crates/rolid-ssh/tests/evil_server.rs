//! A broken or hostile server (roadmap 0.8.18): names with `..`, `/` and
//! control characters, sizes that lie, links that point out of the folder, a
//! huge and an endless listing, a file without end, and a server that stops
//! answering in the middle. The program must not write outside the target
//! folder, crash or hang.
//!
//! The server is an SSH server in this test (russh with russh-sftp's server
//! side) that answers from a made-up tree.

mod common;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rolid_core::copy::{CopyJob, Progress};
use rolid_core::Vfs;
use rolid_local::LocalFs;
use rolid_ssh::{ConnectParams, Link, RemoteFs, SshConnection};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet};
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version};

const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

/// What a made-up path is.
#[derive(Clone)]
enum Node {
    Dir(Vec<String>),
    /// A file: the size it claims, and what reading it gives.
    File { claims: u64, body: Body },
    /// A link: where it points (the answer of realpath).
    Link(String),
    /// A folder with this many names (`f0000000`…), in batches of 500.
    Many(usize),
    /// A folder whose listing never ends.
    Endless,
    /// A folder whose listing is never answered.
    Stuck,
}

#[derive(Clone, Copy)]
enum Body {
    Bytes(usize),
    Endless,
}

fn tree() -> HashMap<String, Node> {
    let mut t = HashMap::new();
    let evil = [
        "../escape.txt",
        "..",
        ".",
        "a/b.txt",
        "/etc-passwd",
        "../../../../../../tmp/rolidscp-evil",
        "",
        "evil\u{1b}[31mred\nnew line\u{7}.txt",
        "nul\0byte.txt",
        "liar-small.bin",
        "liar-big.bin",
        "endless.bin",
        "link-out",
        "dirlink",
        "ok.txt",
        "sub",
    ];
    t.insert("/".into(), Node::Dir(vec!["home".into(), "evil".into(), "huge".into(), "endless".into(), "stuck".into()]));
    t.insert("/home".into(), Node::Dir(vec![]));
    t.insert("/evil".into(), Node::Dir(evil.iter().map(|s| s.to_string()).collect()));
    for name in evil {
        let p = format!("/evil/{name}");
        let n = match name {
            "liar-small.bin" => Node::File { claims: 10, body: Body::Bytes(3 << 20) },
            "liar-big.bin" => Node::File { claims: 1 << 40, body: Body::Bytes(100) },
            "endless.bin" => Node::File { claims: 1000, body: Body::Endless },
            "link-out" => Node::Link("/etc/passwd".into()),
            "dirlink" => Node::Link("/".into()),
            "sub" => Node::Dir(vec!["../../../../tmp/rolidscp-evil-deep".into(), "fine.txt".into()]),
            "." | ".." => continue,
            _ => Node::File { claims: 1000, body: Body::Bytes(1000) },
        };
        t.insert(p, n);
    }
    t.insert("/evil/sub/fine.txt".into(), Node::File { claims: 10, body: Body::Bytes(10) });
    t.insert("/evil/sub/../../../../tmp/rolidscp-evil-deep".into(), Node::File { claims: 10, body: Body::Bytes(10) });
    t.insert("/etc/passwd".into(), Node::File { claims: 30, body: Body::Bytes(30) });
    t.insert("/huge".into(), Node::Many(300_000));
    t.insert("/endless".into(), Node::Endless);
    t.insert("/stuck".into(), Node::Stuck);
    t
}

fn attrs(n: &Node) -> FileAttributes {
    let mut a = FileAttributes::empty();
    match n {
        Node::Dir(_) | Node::Many(_) | Node::Endless | Node::Stuck => {
            a.permissions = Some(S_IFDIR | 0o755);
        }
        Node::File { claims, .. } => {
            a.permissions = Some(S_IFREG | 0o644);
            a.size = Some(*claims);
        }
        Node::Link(_) => a.permissions = Some(S_IFLNK | 0o777),
    }
    a.mtime = Some(1_700_000_000);
    a.atime = Some(1_700_000_000);
    a
}

fn file(name: String, a: FileAttributes) -> File {
    File { longname: name.clone(), filename: name, attrs: a }
}

struct Sftp {
    tree: HashMap<String, Node>,
    /// Open handles: path, and for folders how far the listing got.
    handles: HashMap<String, (String, usize)>,
    next: u64,
}

impl Sftp {
    fn node(&self, path: &str) -> Result<&Node, StatusCode> {
        self.tree.get(path).ok_or(StatusCode::NoSuchFile)
    }
    fn handle(&mut self, path: String) -> String {
        self.next += 1;
        let h = format!("h{}", self.next);
        self.handles.insert(h.clone(), (path, 0));
        h
    }
    fn ok(id: u32) -> Status {
        Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en".into() }
    }
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }
    async fn init(&mut self, _version: u32, _ext: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let real = match path.as_str() {
            "." | "" => "/home".to_string(),
            p => match self.tree.get(p) {
                Some(Node::Link(to)) => to.clone(),
                _ => p.to_string(),
            },
        };
        Ok(Name { id, files: vec![File::dummy(real)] })
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let n = match self.node(&path)? {
            Node::Link(to) => self.node(&to.clone())?.clone(),
            n => n.clone(),
        };
        Ok(Attrs { id, attrs: attrs(&n) })
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs { id, attrs: attrs(self.node(&path)?) })
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        self.node(&path)?;
        Ok(Handle { id, handle: self.handle(path) })
    }
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let (path, done) = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        let files: Vec<File> = match self.node(&path)?.clone() {
            Node::Dir(names) if done == 0 => names
                .iter()
                .map(|n| {
                    let a = self.tree.get(&format!("{path}/{n}")).map(attrs).unwrap_or_else(|| attrs(&Node::Dir(vec![])));
                    file(n.clone(), a)
                })
                .collect(),
            Node::Many(total) if done < total => (done..(done + 500).min(total))
                .map(|i| file(format!("f{i:07}"), attrs(&Node::File { claims: 1, body: Body::Bytes(1) })))
                .collect(),
            Node::Endless => (done..done + 500).map(|i| file(format!("e{i:09}"), attrs(&Node::Dir(vec![])))).collect(),
            Node::Stuck => std::future::pending().await,
            _ => return Err(StatusCode::Eof),
        };
        self.handles.get_mut(&handle).unwrap().1 = done + files.len().max(1);
        Ok(Name { id, files })
    }
    async fn open(&mut self, id: u32, filename: String, _flags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        self.node(&filename)?;
        Ok(Handle { id, handle: self.handle(filename) })
    }
    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        let (path, _) = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        let body = match self.node(&path)? {
            Node::File { body, .. } => *body,
            Node::Link(to) => match self.node(&to.clone())? {
                Node::File { body, .. } => *body,
                _ => return Err(StatusCode::Failure),
            },
            _ => return Err(StatusCode::Failure),
        };
        let end = match body {
            Body::Bytes(n) => n as u64,
            Body::Endless => u64::MAX,
        };
        if offset >= end {
            return Err(StatusCode::Eof);
        }
        let n = (end - offset).min(u64::from(len)) as usize;
        Ok(Data { id, data: vec![b'x'; n] })
    }
    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(Self::ok(id))
    }
}

struct Ssh {
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: server::ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        match (name, self.channels.remove(&id)) {
            ("sftp", Some(ch)) => {
                session.channel_success(id)?;
                russh_sftp::server::run(ch.into_stream(), Sftp { tree: tree(), handles: HashMap::new(), next: 0 }).await;
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
    async fn exec_request(&mut self, id: ChannelId, _data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        // No shell: every command fails.
        session.channel_failure(id)?;
        Ok(())
    }
}

/// The server on a free port; the connection to it (the program's own code).
async fn connect(dir: &std::path::Path) -> (Arc<Link>, RemoteFs) {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::Password);
    let key = common::host_key();
    let fingerprint = key.public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
    let config = Arc::new(server::Config { methods, keys: vec![key], ..Default::default() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(s) = server::run_stream(config, sock, Ssh { channels: HashMap::new() }).await {
                    let _ = s.await;
                }
            });
        }
    });
    std::fs::create_dir_all(dir).unwrap();
    let params = ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "u".into(),
        password: Some("x".into()),
        known_hosts: Some(dir.join("known_hosts")),
        accept_fingerprint: Some(fingerprint),
        skip_agent: true,
        skip_default_keys: true,
        request_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let conn = SshConnection::connect(&params).await.expect("connect");
    let link = Link::new(conn);
    let fs = RemoteFs::new(link.clone());
    (link, fs)
}

/// Every path under `root` (relative), to see what was written.
fn all_under(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn names_sizes_and_links_of_a_hostile_server_stay_in_the_target() {
    let dir = common::tmpdir("evil-copy");
    let (_link, fs) = connect(&dir).await;

    let list = tokio::time::timeout(Duration::from_secs(30), fs.list("/evil")).await.expect("the listing ends").expect("listed");
    let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
    assert!(!names.contains(&".") && !names.contains(&".."), "{names:?}");
    assert!(names.contains(&"a/b.txt"), "the listing shows what the server says: {names:?}");

    // A sandbox: the target folder inside it, nothing else may change there.
    let sandbox = dir.join("sandbox");
    let target = sandbox.join("one/two/target");
    std::fs::create_dir_all(&target).unwrap();
    let before = all_under(&sandbox);
    static NOOP: fn(&Progress) = |_| {};
    let mut job = CopyJob::new(&fs, &LocalFs, Arc::new(AtomicBool::new(false)), &NOOP).keep_going();
    let t = Instant::now();
    let r = tokio::time::timeout(
        Duration::from_secs(120),
        job.copy_items_retrying(&[("/".into(), "evil".into())], &target.to_string_lossy(), 0, |_| async { false }),
    )
    .await
    .expect("the copy ends");
    eprintln!("copy: {r:?} in {:?}", t.elapsed());
    for f in job.failures() {
        eprintln!("  not copied: {:?}: {}", f.path, f.error);
    }
    let after = all_under(&sandbox);
    let new: Vec<&String> = after.iter().filter(|p| !before.contains(p)).collect();
    for p in &new {
        assert!(p.starts_with("one/two/target/evil"), "written outside the target folder: {p:?}");
    }
    assert!(!std::path::Path::new("/tmp/rolidscp-evil").exists() && !std::path::Path::new("/tmp/rolidscp-evil-deep").exists());
    assert!(!dir.join("escape.txt").exists() && !sandbox.join("one/two/escape.txt").exists());
    let got = target.join("evil");
    let names: Vec<String> = std::fs::read_dir(&got).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    for bad in ["a", "etc-passwd", "escape.txt", "", "nul"] {
        assert!(!names.iter().any(|n| n == bad || n.starts_with("nul\0")), "{bad:?} in {names:?}");
    }
    let failed: Vec<&str> = job.failures().iter().map(|f| f.path.as_str()).collect();
    for refused in ["/evil/../escape.txt", "/evil/a/b.txt", "/evil/endless.bin"] {
        assert!(failed.contains(&refused), "{refused} not refused: {failed:?}");
    }
    // A name with control characters is a valid name on this disk: copied as it is.
    assert!(names.iter().any(|n| n.starts_with("evil\u{1b}[31mred\nnew line")), "{names:?}");
    // Sizes that lie: the file is what the server sent, as long as it ends.
    assert_eq!(std::fs::metadata(got.join("liar-small.bin")).unwrap().len(), 3 << 20);
    assert_eq!(std::fs::metadata(got.join("liar-big.bin")).unwrap().len(), 100);
    // Without end: stopped after its size plus the allowed growth (MAX_GROWTH), nothing left.
    assert!(!got.join("endless.bin").exists() && !got.join("endless.bin.filepart").exists(), "{names:?}");
    // A link out of the folder: a plain file here with what the server gave.
    let link = std::fs::symlink_metadata(got.join("link-out")).unwrap();
    assert!(link.file_type().is_file());
    // A link to a folder is not followed.
    assert!(!got.join("dirlink").exists());
    assert!(got.join("sub/fine.txt").exists());
    assert_eq!(std::fs::read_dir(got.join("sub")).unwrap().count(), 1, "only the good name of sub/");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn huge_endless_and_unanswered_listings_end() {
    let dir = common::tmpdir("evil-list");
    let (_link, fs) = connect(&dir).await;

    let t = Instant::now();
    let huge = tokio::time::timeout(Duration::from_secs(120), fs.list("/huge")).await.expect("ends").expect("listed");
    assert_eq!(huge.len(), 300_000);
    eprintln!("300 000 names in {:?}", t.elapsed());

    let t = Instant::now();
    let endless = tokio::time::timeout(Duration::from_secs(300), fs.list("/endless")).await.expect("an endless listing is stopped");
    let e = endless.expect_err("an endless listing is an error");
    assert!(e.to_string().contains("does not end"), "{e}");
    assert!(!e.is_connection_lost(), "{e}");
    eprintln!("endless listing stopped after {:?}", t.elapsed());

    // A listing the server never answers: the request times out (3 s in this
    // test, 90 s in the program) and the connection is made again.
    let t = Instant::now();
    let stuck = tokio::time::timeout(Duration::from_secs(60), fs.list("/stuck")).await.expect("an unanswered listing ends");
    assert!(stuck.is_err());
    assert!(t.elapsed() < Duration::from_secs(30), "{:?}", t.elapsed());
    eprintln!("unanswered listing: {:?} after {:?}", stuck.err(), t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_on_a_hostile_server_ends() {
    let dir = common::tmpdir("evil-search");
    let (link, fs) = connect(&dir).await;
    let (_, conn) = link.current().unwrap();
    let cancel = AtomicBool::new(false);
    let hits = std::sync::Mutex::new(Vec::new());
    let emit = |h: rolid_search::Hit| hits.lock().unwrap().push(h.rel);
    let sink = rolid_search::Sink::new(&emit, 10_000, &cancel);
    let q = rolid_search::Query { root: "/evil".into(), name: "*".into(), ..Default::default() };
    let r = tokio::time::timeout(Duration::from_secs(60), rolid_search::remote::search_remote(&conn, &fs, &q, &sink)).await.expect("the search ends");
    eprintln!("search: {r:?}, {} hits", hits.lock().unwrap().len());
    assert!(r.is_ok());
    for h in hits.lock().unwrap().iter() {
        assert!(!h.split('/').any(|p| p == ".."), "a hit climbs out: {h:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
