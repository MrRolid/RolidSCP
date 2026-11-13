//! Dragging files out of the window to other applications on X11: RolidSCP
//! becomes the source of an XDND (version 5) drag.
//!
//! winit keeps the pointer grab of the pressed button, so the window still
//! gets the release, Esc and sets the cursor. A separate thread with its own
//! pure-Rust X connection (x11rb, the version winit uses, no unsafe code)
//! asks where the pointer is, finds the XDND-aware window under it, sends it
//! XdndEnter / XdndPosition / XdndLeave / XdndDrop from a hidden helper window
//! and answers its requests for the data (the `XdndSelection`):
//! - `text/uri-list`: `file://` URIs of the local files;
//! - `XdndDirectSave0` (XDS, files of a server or an archive): the target
//!   writes the full path of the new file into our window's property and
//!   asks; we take the folder from it and download there.
//!
//! When the pointer comes back over our window, the target gets XdndLeave and
//! the drag inside the window continues.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Pause between two pointer queries.
const PERIOD: Duration = Duration::from_millis(20);
/// After the release: how long to wait for the target's (first) answer.
const STATUS_WAIT: Duration = Duration::from_millis(500);
/// After the data was given: how long to wait for XdndFinished.
const FINISH_WAIT: Duration = Duration::from_secs(30);

/// The XDND version we speak.
pub const VERSION: u8 = 5;

/// What the drag offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// `text/uri-list` ready at once (local files); `None` = prepared on request.
    pub uri_list: Option<Vec<u8>>,
    /// XDS offered with this file name (files of a server or an archive).
    pub xds_name: Option<String>,
    /// Shift moves (local files only).
    pub allow_move: bool,
}

impl Offer {
    /// The data types in the order of preference.
    pub fn types(&self) -> Vec<&'static str> {
        let mut t = Vec::new();
        if self.xds_name.is_some() {
            t.push(XDS);
        }
        t.push(URI_LIST);
        t
    }
}

pub const URI_LIST: &str = "text/uri-list";
pub const XDS: &str = "XdndDirectSave0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Move,
}

/// The action asked from the target.
pub fn action_for(allow_move: bool, shift: bool) -> Action {
    if allow_move && shift { Action::Move } else { Action::Copy }
}

/// Percent-encode a local path as a `file://` URI (RFC 8089, empty host).
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// `text/uri-list`: one URI per line, CRLF line ends (RFC 2483).
pub fn uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::new();
    for p in paths {
        out.push_str(&file_uri(p));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

fn percent_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            let h = std::str::from_utf8(s.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Why an XDS answer of the target cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XdsError {
    /// Not a `file://` URI of an absolute path.
    NotFile,
    /// A file on another computer.
    OtherHost,
}

/// The folder the target wants the file in: the parent of the `file://` URI it
/// wrote into our `XdndDirectSave0` property. `host` is our host name.
pub fn xds_dir(uri: &[u8], host: &str) -> Result<PathBuf, XdsError> {
    use std::os::unix::ffi::OsStrExt;
    let uri: &[u8] = uri.split(|&b| b == 0 || b == b'\r' || b == b'\n').next().unwrap_or_default();
    let rest = uri.strip_prefix(b"file://").ok_or(XdsError::NotFile)?;
    let slash = rest.iter().position(|&b| b == b'/').ok_or(XdsError::NotFile)?;
    let h = &rest[..slash];
    if !(h.is_empty() || h == b"localhost" || h == host.as_bytes()) {
        return Err(XdsError::OtherHost);
    }
    let path = percent_decode(&rest[slash..]).ok_or(XdsError::NotFile)?;
    if path.contains(&0) {
        return Err(XdsError::NotFile);
    }
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&path));
    let dir = path.parent().ok_or(XdsError::NotFile)?;
    if path.file_name().is_none() || !dir.is_absolute() {
        return Err(XdsError::NotFile);
    }
    Ok(dir.to_path_buf())
}

/// Our host name (without libc: the kernel's).
fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// A request of the target for the data, decided without the X server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Targets,
    UriList,
    /// XDS with the value of our property (the URI the target chose).
    DirectSave(Vec<u8>),
    Other,
}

/// Answer to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The list of types.
    Targets,
    Data(Vec<u8>),
    /// Not available: the conversion fails.
    Refuse,
    /// The data is being prepared: answer later.
    Wait,
}

/// The data side of a drag: what to answer to the target's requests. The app
/// is asked (events) for what it has to prepare.
#[derive(Debug)]
pub struct Exchange {
    offer: Offer,
    host: String,
    /// A request waiting for the app.
    waiting: Option<Waiting>,
    /// How the data went (for the log).
    pub method: Option<Method>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    UriList,
    DirectSave,
}

/// How the files were handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    UriList,
    /// Downloaded into a temporary folder first, then a uri-list.
    TempFolder,
    Xds,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Method::UriList => "uri-list",
            Method::TempFolder => "temporary folder + uri-list",
            Method::Xds => "XDS (XdndDirectSave0)",
        }
    }
}

/// What the app must do for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    /// Download into a temporary folder and give the uri-list ([`Exchange::data`]).
    UriList,
    /// Download into this folder (XDS) and confirm ([`Exchange::saved`]).
    SaveTo(PathBuf),
}

impl Exchange {
    pub fn new(offer: Offer) -> Exchange {
        Exchange { offer, host: host_name(), waiting: None, method: None }
    }

    #[cfg(test)]
    fn with_host(offer: Offer, host: &str) -> Exchange {
        Exchange { offer, host: host.into(), waiting: None, method: None }
    }

    pub fn offer(&self) -> &Offer {
        &self.offer
    }

    pub fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// A request of the target.
    pub fn request(&mut self, ask: Ask) -> (Reply, Option<Need>) {
        match ask {
            Ask::Targets => (Reply::Targets, None),
            Ask::UriList => {
                if let Some(d) = &self.offer.uri_list {
                    self.method.get_or_insert(Method::UriList);
                    return (Reply::Data(d.clone()), None);
                }
                match self.waiting {
                    // Another part of the target asks while the download runs:
                    // it waits for the same data.
                    Some(Waiting::UriList) => return (Reply::Wait, None),
                    Some(_) => return (Reply::Refuse, None),
                    None => {}
                }
                self.waiting = Some(Waiting::UriList);
                self.method = Some(Method::TempFolder);
                (Reply::Wait, Some(Need::UriList))
            }
            Ask::DirectSave(uri) => {
                if self.offer.xds_name.is_none() || self.waiting.is_some() {
                    return (Reply::Refuse, None);
                }
                self.method = Some(Method::Xds);
                match xds_dir(&uri, &self.host) {
                    Ok(dir) => {
                        self.waiting = Some(Waiting::DirectSave);
                        (Reply::Wait, Some(Need::SaveTo(dir)))
                    }
                    // "E": an error, the target gives up (no fallback offered).
                    Err(_) => (Reply::Data(b"E".to_vec()), None),
                }
            }
            Ask::Other => (Reply::Refuse, None),
        }
    }

    /// The temporary download is done (`Some` = its uri-list) or failed.
    pub fn data(&mut self, list: Option<Vec<u8>>) -> Option<Reply> {
        if self.waiting != Some(Waiting::UriList) {
            return None;
        }
        self.waiting = None;
        Some(match list {
            Some(d) => {
                self.offer.uri_list = Some(d.clone());
                Reply::Data(d)
            }
            None => Reply::Refuse,
        })
    }

    /// XDS: the download into the target's folder was queued (`true`) or not.
    pub fn saved(&mut self, ok: bool) -> Option<Reply> {
        if self.waiting != Some(Waiting::DirectSave) {
            return None;
        }
        self.waiting = None;
        Some(Reply::Data(if ok { b"S".to_vec() } else { b"E".to_vec() }))
    }
}

/// The kind of a request, for the debug log (without the data).
fn ask_kind(a: &Ask) -> &'static str {
    match a {
        Ask::Targets => "targets",
        Ask::UriList => "uri-list",
        Ask::DirectSave(_) => "direct-save",
        Ask::Other => "other",
    }
}

/// The kind of an answer, for the debug log (without the data).
fn reply_kind(r: &Reply) -> &'static str {
    match r {
        Reply::Targets => "targets",
        Reply::Data(_) => "data",
        Reply::Refuse => "refuse",
        Reply::Wait => "wait",
    }
}

/// From the app to the thread.
#[derive(Debug)]
pub enum Cmd {
    /// Esc, or the drag ended inside the window.
    Cancel,
    Data(Option<Vec<u8>>),
    Saved(bool),
}

/// From the thread to the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Need(Need),
    Ended(End),
}

/// How a drag out ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct End {
    /// The target application (its WM_CLASS), if there was one.
    pub target: Option<String>,
    pub method: Option<Method>,
    pub result: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The target took the files (with this action).
    Done(Action),
    /// The target refused the drop or the data.
    Refused,
    /// Released where nothing accepts files.
    NoTarget,
    /// The button was released over our own window.
    Inside,
    Canceled,
    /// The target did not confirm in time.
    NoAnswer,
    Failed(String),
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Done(Action::Copy) => "copied".into(),
            Outcome::Done(Action::Move) => "moved".into(),
            Outcome::Refused => "refused by the target".into(),
            Outcome::NoTarget => "released where no application accepts files".into(),
            Outcome::Inside => "back in the window".into(),
            Outcome::Canceled => "canceled".into(),
            Outcome::NoAnswer => "the target did not confirm the drop".into(),
            Outcome::Failed(e) => format!("failed: {e}"),
        }
    }
}

/// Where the drag is now (read by the UI every frame).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The pointer is outside our window.
    pub outside: bool,
    /// The application under the pointer accepts the files.
    pub accepted: bool,
    /// The button was released outside: the drop is being handed over.
    pub dropped: bool,
}

/// A drag out in progress (the thread runs until the drag ends).
pub struct Source {
    cmd: Sender<Cmd>,
    events: Receiver<Event>,
    status: Arc<Mutex<Status>>,
}

impl Source {
    /// Start the source for our window `win`.
    pub fn start(win: u32, offer: Offer, repaint: impl Fn() + Send + 'static) -> Source {
        let (cmd, cmd_rx) = mpsc::channel();
        let (ev_tx, events) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let st = status.clone();
        let ev2 = ev_tx.clone();
        let spawned = std::thread::Builder::new().name("xdnd-source".into()).spawn(move || {
            let send = |e: Event| {
                let _ = ev_tx.send(e);
                repaint();
            };
            let end = match x11::run(win, offer, &cmd_rx, &st, &send) {
                Ok(end) => end,
                Err((target, e)) => End { target, method: None, result: Outcome::Failed(e) },
            };
            send(Event::Ended(end));
        });
        if let Err(e) = spawned {
            let _ = ev2.send(Event::Ended(End { target: None, method: None, result: Outcome::Failed(e.to_string()) }));
        }
        Source { cmd, events, status }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.cmd.send(c);
    }

    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

/// XDND messages without the X server (for the tests and the thread).
pub mod msg {
    /// XdndEnter: source, version and "more than 3 types", the first 3 types.
    pub fn enter(src: u32, version: u8, types: &[u32]) -> [u32; 5] {
        let more = u32::from(types.len() > 3);
        let t = |i: usize| types.get(i).copied().unwrap_or(0);
        [src, (u32::from(version) << 24) | more, t(0), t(1), t(2)]
    }

    pub fn position(src: u32, x: i16, y: i16, time: u32, action: u32) -> [u32; 5] {
        [src, 0, ((x as u16 as u32) << 16) | (y as u16 as u32), time, action]
    }

    pub fn leave(src: u32) -> [u32; 5] {
        [src, 0, 0, 0, 0]
    }

    pub fn drop(src: u32, time: u32) -> [u32; 5] {
        [src, 0, time, 0, 0]
    }

    /// XdndStatus: (accepted, action).
    pub fn status(data: [u32; 5]) -> (bool, u32) {
        (data[1] & 1 == 1, data[4])
    }

    /// XdndFinished (version 5): (accepted, action); older versions say nothing.
    pub fn finished(data: [u32; 5], version: u8) -> (bool, Option<u32>) {
        if version >= 5 { (data[1] & 1 == 1, Some(data[2])) } else { (true, None) }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod x11 {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
        SelectionRequestEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::Event as XEvent;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    type Err = (Option<String>, String);

    struct Atoms {
        aware: u32,
        proxy: u32,
        enter: u32,
        position: u32,
        status: u32,
        leave: u32,
        drop: u32,
        finished: u32,
        selection: u32,
        type_list: u32,
        copy: u32,
        move_: u32,
        targets: u32,
        uri_list: u32,
        xds: u32,
        text_plain: u32,
        stamp: u32,
    }

    fn atoms(c: &RustConnection) -> Result<Atoms, String> {
        let names = [
            "XdndAware",
            "XdndProxy",
            "XdndEnter",
            "XdndPosition",
            "XdndStatus",
            "XdndLeave",
            "XdndDrop",
            "XdndFinished",
            "XdndSelection",
            "XdndTypeList",
            "XdndActionCopy",
            "XdndActionMove",
            "TARGETS",
            URI_LIST,
            XDS,
            "text/plain",
            "_ROLIDSCP_TIMESTAMP",
        ];
        let cookies: Vec<_> = names.iter().map(|n| c.intern_atom(false, n.as_bytes())).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let mut a = Vec::new();
        for ck in cookies {
            a.push(ck.reply().map_err(|e| e.to_string())?.atom);
        }
        Ok(Atoms {
            aware: a[0],
            proxy: a[1],
            enter: a[2],
            position: a[3],
            status: a[4],
            leave: a[5],
            drop: a[6],
            finished: a[7],
            selection: a[8],
            type_list: a[9],
            copy: a[10],
            move_: a[11],
            targets: a[12],
            uri_list: a[13],
            xds: a[14],
            text_plain: a[15],
            stamp: a[16],
        })
    }

    /// A window that accepts drops.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Target {
        /// The XdndAware window.
        win: Window,
        /// Where the messages go (its proxy, or itself).
        to: Window,
        version: u8,
    }

    /// Current exchange with one target.
    struct Current {
        t: Target,
        accepted: bool,
        /// Waiting for XdndStatus since.
        pending: Option<Instant>,
        last: Option<((i16, i16), u32)>,
        /// The data came after the target asked during the motion: ask
        /// for a new status (the target accepts only once it has the data).
        resend: bool,
    }

    /// A held request was answered. After the drop the wait for XdndFinished
    /// starts again; before it (the target asked during the motion) the drag
    /// goes on and the target is asked for a new status.
    fn answered(phase: &mut Phase, cur: &mut Option<Current>) {
        match phase {
            Phase::Dropped(_) => *phase = Phase::Dropped(Instant::now()),
            Phase::Tracking | Phase::Releasing(_) => {
                if let Some(c) = cur.as_mut() {
                    c.resend = true;
                }
            }
        }
    }

    enum Phase {
        Tracking,
        /// Released; waiting for the target's status before dropping.
        Releasing(Instant),
        /// XdndDrop sent; waiting for XdndFinished.
        Dropped(Instant),
    }

    struct Ctx<'a> {
        c: RustConnection,
        a: Atoms,
        root: Window,
        our: Window,
        helper: Window,
        time: u32,
        ex: Exchange,
        /// Requests waiting for the data (all get the same answer).
        held: Vec<SelectionRequestEvent>,
        status: &'a Mutex<Status>,
    }

    fn e2s(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    pub fn run(
        our: u32,
        offer: Offer,
        cmds: &Receiver<Cmd>,
        status: &Mutex<Status>,
        send: &dyn Fn(Event),
    ) -> Result<End, Err> {
        let (c, screen) = RustConnection::connect(None).map_err(|e| (None, e2s(e)))?;
        let root = c.setup().roots[screen].root;
        let a = atoms(&c).map_err(|e| (None, e))?;
        let helper = c.generate_id().map_err(|e| (None, e2s(e)))?;
        let aux = CreateWindowAux::new().override_redirect(1).event_mask(EventMask::PROPERTY_CHANGE);
        c.create_window(0, helper, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux).map_err(|e| (None, e2s(e)))?;
        let mut x = Ctx { c, a, root, our, helper, time: 0, ex: Exchange::new(offer), held: Vec::new(), status };
        let r = x.drag(cmds, send);
        let _ = x.c.destroy_window(x.helper);
        let _ = x.c.flush();
        r
    }

    impl Ctx<'_> {
        fn err(&self, t: Option<&Current>, e: impl std::fmt::Display) -> Err {
            (t.and_then(|t| self.class(t.t.win)), e.to_string())
        }

        /// A server timestamp (for the selection), and the data on the helper.
        fn setup(&mut self) -> Result<(), String> {
            let c = &self.c;
            c.change_property8(PropMode::REPLACE, self.helper, self.a.stamp, AtomEnum::STRING, b"").map_err(e2s)?;
            c.flush().map_err(e2s)?;
            let start = Instant::now();
            self.time = loop {
                match c.poll_for_event().map_err(e2s)? {
                    Some(XEvent::PropertyNotify(p)) if p.window == self.helper => break p.time,
                    Some(_) => {}
                    None if start.elapsed() > Duration::from_secs(2) => break 0,
                    None => std::thread::sleep(Duration::from_millis(2)),
                }
            };
            c.set_selection_owner(self.helper, self.a.selection, self.time).map_err(e2s)?;
            let types = self.type_atoms();
            c.change_property32(PropMode::REPLACE, self.helper, self.a.type_list, AtomEnum::ATOM, &types).map_err(e2s)?;
            if let Some(name) = self.ex.offer().xds_name.clone() {
                c.change_property8(PropMode::REPLACE, self.helper, self.a.xds, self.a.text_plain, name.as_bytes()).map_err(e2s)?;
            }
            c.flush().map_err(e2s)?;
            Ok(())
        }

        fn type_atoms(&self) -> Vec<u32> {
            self.ex.offer().types().iter().map(|t| if *t == XDS { self.a.xds } else { self.a.uri_list }).collect()
        }

        fn action_atom(&self, a: Action) -> u32 {
            match a {
                Action::Copy => self.a.copy,
                Action::Move => self.a.move_,
            }
        }

        fn send_msg(&self, t: &Target, kind: u32, data: [u32; 5]) -> Result<(), String> {
            let ev = ClientMessageEvent::new(32, t.win, kind, data);
            self.c.send_event(false, t.to, EventMask::NO_EVENT, ev).map_err(e2s)?;
            self.c.flush().map_err(e2s)
        }

        fn prop32(&self, w: Window, prop: u32, ty: impl Into<u32>) -> Option<u32> {
            let r = self.c.get_property(false, w, prop, ty.into(), 0, 1).ok()?.reply().ok()?;
            r.value32().and_then(|mut v| v.next())
        }

        /// The application name of a window (WM_CLASS), for the log.
        fn class(&self, w: Window) -> Option<String> {
            let r = self.c.get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64).ok()?.reply().ok()?;
            let parts: Vec<&[u8]> = r.value.split(|&b| b == 0).filter(|p| !p.is_empty()).collect();
            parts.last().map(|p| String::from_utf8_lossy(p).into_owned())
        }

        /// Under the pointer: our window (`None`) or the XDND-aware window (if any).
        fn under(&self, mut w: Window) -> Result<Option<Option<Target>>, String> {
            for _ in 0..32 {
                if w == 0 {
                    return Ok(Some(None));
                }
                if w == self.our {
                    return Ok(None);
                }
                if let Some(v) = self.prop32(w, self.a.aware, AtomEnum::ATOM) {
                    if v < 3 {
                        return Ok(Some(None));
                    }
                    let to = self
                        .prop32(w, self.a.proxy, AtomEnum::WINDOW)
                        .filter(|&p| self.prop32(p, self.a.proxy, AtomEnum::WINDOW) == Some(p))
                        .unwrap_or(w);
                    return Ok(Some(Some(Target { win: w, to, version: (v as u8).min(VERSION) })));
                }
                w = self.c.query_pointer(w).map_err(e2s)?.reply().map_err(e2s)?.child;
            }
            Ok(Some(None))
        }

        fn set_status(&self, f: impl FnOnce(&mut Status)) {
            f(&mut self.status.lock().unwrap());
        }

        fn leave(&self, cur: &mut Option<Current>) {
            if let Some(c) = cur.take() {
                let _ = self.send_msg(&c.t, self.a.leave, msg::leave(self.helper));
            }
            self.set_status(|s| s.accepted = false);
        }

        fn end(&self, cur: Option<&Current>, result: Outcome) -> End {
            End { target: cur.and_then(|c| self.class(c.t.win)), method: self.ex.method, result }
        }

        fn drag(&mut self, cmds: &Receiver<Cmd>, send: &dyn Fn(Event)) -> Result<End, Err> {
            self.setup().map_err(|e| (None, e))?;
            let mut cur: Option<Current> = None;
            let mut phase = Phase::Tracking;
            loop {
                // The app.
                while let Ok(cmd) = cmds.try_recv() {
                    match cmd {
                        Cmd::Cancel if matches!(phase, Phase::Dropped(_)) => {
                            // Too late to cancel the drop; refuse a waiting request.
                            if let Some(r) = self.ex.data(None).or_else(|| self.ex.saved(false)) {
                                self.answer(r).map_err(|e| self.err(cur.as_ref(), e))?;
                            }
                        }
                        Cmd::Cancel => {
                            self.leave(&mut cur);
                            return Ok(self.end(None, Outcome::Canceled));
                        }
                        Cmd::Data(d) => {
                            if let Some(r) = self.ex.data(d) {
                                self.answer(r).map_err(|e| self.err(cur.as_ref(), e))?;
                                answered(&mut phase, &mut cur);
                            }
                        }
                        Cmd::Saved(ok) => {
                            if let Some(r) = self.ex.saved(ok) {
                                self.answer(r).map_err(|e| self.err(cur.as_ref(), e))?;
                                answered(&mut phase, &mut cur);
                            }
                        }
                    }
                }
                // The X server.
                while let Some(ev) = self.c.poll_for_event().map_err(|e| self.err(cur.as_ref(), e))? {
                    match ev {
                        XEvent::ClientMessage(m) if m.type_ == self.a.status => {
                            let d = m.data.as_data32();
                            if let Some(c) = cur.as_mut().filter(|c| c.t.win == d[0]) {
                                let (ok, _) = msg::status(d);
                                tracing::debug!("drag out: status accepted={ok}");
                                c.accepted = ok;
                                c.pending = None;
                                self.set_status(|s| s.accepted = ok);
                            }
                        }
                        XEvent::ClientMessage(m) if m.type_ == self.a.finished => {
                            let d = m.data.as_data32();
                            if let (Phase::Dropped(_), Some(c)) = (&phase, cur.as_ref().filter(|c| c.t.win == d[0])) {
                                let (ok, action) = msg::finished(d, c.t.version);
                                tracing::debug!("drag out: finished ok={ok}");
                                let done = if action == Some(self.a.move_) { Action::Move } else { Action::Copy };
                                let result = if ok { Outcome::Done(done) } else { Outcome::Refused };
                                return Ok(self.end(Some(c), result));
                            }
                        }
                        XEvent::SelectionRequest(r) if r.selection == self.a.selection => {
                            self.request(r, send).map_err(|e| self.err(cur.as_ref(), e))?;
                        }
                        _ => {}
                    }
                }
                match phase {
                    Phase::Tracking => {
                        let q = self.c.query_pointer(self.root).map_err(e2s).and_then(|c| c.reply().map_err(e2s));
                        let q = q.map_err(|e| self.err(cur.as_ref(), e))?;
                        let button = u16::from(q.mask) & 0x100 != 0;
                        let shift = u16::from(q.mask) & 0x1 != 0;
                        let under = self.under(q.child).map_err(|e| self.err(cur.as_ref(), e))?;
                        let Some(found) = under else {
                            // Back over our window: the drag continues inside.
                            self.leave(&mut cur);
                            self.set_status(|s| s.outside = false);
                            if !button {
                                return Ok(self.end(None, Outcome::Inside));
                            }
                            std::thread::sleep(PERIOD);
                            continue;
                        };
                        self.set_status(|s| s.outside = true);
                        if cur.as_ref().map(|c| c.t) != found {
                            self.leave(&mut cur);
                            if let Some(t) = found {
                                let types = self.type_atoms();
                                self.send_msg(&t, self.a.enter, msg::enter(self.helper, t.version, &types))
                                    .map_err(|e| (self.class(t.win), e))?;
                                cur = Some(Current { t, accepted: false, pending: None, last: None, resend: false });
                            }
                        }
                        if let Some(c) = cur.as_mut() {
                            let action = self.action_atom(action_for(self.ex.offer().allow_move, shift));
                            let now = ((q.root_x, q.root_y), action);
                            let stale = c.pending.is_some_and(|t| t.elapsed() > Duration::from_millis(300));
                            if (c.last != Some(now) || c.resend) && (c.pending.is_none() || stale) {
                                c.resend = false;
                                let m = msg::position(self.helper, q.root_x, q.root_y, self.time, action);
                                self.send_msg(&c.t, self.a.position, m).map_err(|e| (self.class(c.t.win), e))?;
                                c.last = Some(now);
                                c.pending = Some(Instant::now());
                            }
                        }
                        if !button {
                            if cur.is_none() {
                                return Ok(self.end(None, Outcome::NoTarget));
                            }
                            phase = Phase::Releasing(Instant::now());
                        }
                    }
                    Phase::Releasing(since) => {
                        // The data the target asked for during the motion is
                        // still being downloaded: wait for it.
                        if self.ex.is_waiting() {
                            if since.elapsed() > FINISH_WAIT {
                                return Ok(self.end(cur.as_ref(), Outcome::NoAnswer));
                            }
                            std::thread::sleep(PERIOD);
                            continue;
                        }
                        let c = cur.as_mut().expect("a target");
                        if c.resend && c.pending.is_none() {
                            // Ask again where the pointer was released.
                            c.resend = false;
                            if let Some(((x, y), action)) = c.last {
                                let m = msg::position(self.helper, x, y, self.time, action);
                                self.send_msg(&c.t, self.a.position, m).map_err(|e| (self.class(c.t.win), e))?;
                                c.pending = Some(Instant::now());
                                phase = Phase::Releasing(Instant::now());
                                continue;
                            }
                        }
                        let c = cur.as_ref().expect("a target");
                        if c.pending.is_none() || since.elapsed() > STATUS_WAIT {
                            if !c.accepted {
                                let end = self.end(Some(c), Outcome::Refused);
                                self.leave(&mut cur);
                                return Ok(end);
                            }
                            self.send_msg(&c.t, self.a.drop, msg::drop(self.helper, self.time)).map_err(|e| self.err(Some(c), e))?;
                            self.set_status(|s| s.dropped = true);
                            tracing::debug!("drag out: drop sent");
                            phase = Phase::Dropped(Instant::now());
                        }
                    }
                    Phase::Dropped(since) => {
                        // While the app prepares the data, the target waits.
                        if !self.ex.is_waiting() && since.elapsed() > FINISH_WAIT {
                            return Ok(self.end(cur.as_ref(), Outcome::NoAnswer));
                        }
                    }
                }
                std::thread::sleep(if matches!(phase, Phase::Tracking) { PERIOD } else { Duration::from_millis(5) });
            }
        }

        /// A SelectionRequest of the target.
        fn request(&mut self, r: SelectionRequestEvent, send: &dyn Fn(Event)) -> Result<(), String> {
            let ask = if r.target == self.a.targets {
                Ask::Targets
            } else if r.target == self.a.uri_list {
                Ask::UriList
            } else if r.target == self.a.xds {
                let v = self.c.get_property(false, self.helper, self.a.xds, AtomEnum::ANY, 0, 4096).map_err(e2s)?.reply().map_err(e2s)?;
                Ask::DirectSave(v.value)
            } else {
                Ask::Other
            };
            tracing::debug!("drag out: request {} (after drop: {})", ask_kind(&ask), self.status.lock().unwrap().dropped);
            let (reply, need) = self.ex.request(ask);
            tracing::debug!("drag out: reply {}", reply_kind(&reply));
            if reply == Reply::Wait {
                self.held.push(r);
            } else {
                self.reply(&r, reply)?;
            }
            if let Some(n) = need {
                send(Event::Need(n));
            }
            Ok(())
        }

        /// Answer the held requests.
        fn answer(&mut self, reply: Reply) -> Result<(), String> {
            for r in std::mem::take(&mut self.held) {
                self.reply(&r, reply.clone())?;
            }
            Ok(())
        }

        fn reply(&self, r: &SelectionRequestEvent, reply: Reply) -> Result<(), String> {
            // Obsolete clients give no property: use the target.
            let prop = if r.property == 0 { r.target } else { r.property };
            let property = match reply {
                Reply::Targets => {
                    let mut t = vec![self.a.targets];
                    t.extend(self.type_atoms());
                    self.c.change_property32(PropMode::REPLACE, r.requestor, prop, AtomEnum::ATOM, &t).map_err(e2s)?;
                    prop
                }
                Reply::Data(d) => {
                    let ty = if r.target == self.a.xds { AtomEnum::STRING.into() } else { r.target };
                    self.c.change_property8(PropMode::REPLACE, r.requestor, prop, ty, &d).map_err(e2s)?;
                    prop
                }
                Reply::Refuse | Reply::Wait => 0,
            };
            let ev = SelectionNotifyEvent {
                response_type: SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: r.time,
                requestor: r.requestor,
                selection: r.selection,
                target: r.target,
                property,
            };
            self.c.send_event(false, r.requestor, EventMask::NO_EVENT, ev).map_err(e2s)?;
            self.c.flush().map_err(e2s)
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod x11 {
    use super::*;
    pub fn run(_: u32, _: Offer, _: &Receiver<Cmd>, _: &Mutex<Status>, _: &dyn Fn(Event)) -> Result<End, (Option<String>, String)> {
        Err((None, "not available on this system".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_with_spaces_and_diacritics() {
        let paths = vec![PathBuf::from("/home/u/Příliš žluťoučký kůň.txt"), PathBuf::from("/tmp/a b/100%#?&.md"), PathBuf::from("/x/plain-name_1.~")];
        let l = String::from_utf8(uri_list(&paths)).unwrap();
        assert_eq!(
            l,
            "file:///home/u/P%C5%99%C3%ADli%C5%A1%20%C5%BElu%C5%A5ou%C4%8Dk%C3%BD%20k%C5%AF%C5%88.txt\r\n\
             file:///tmp/a%20b/100%25%23%3F%26.md\r\n\
             file:///x/plain-name_1.~\r\n"
        );
        // Every URI decodes back to its path (as XDS reads them).
        for p in &paths {
            let u = file_uri(&p.join("child"));
            assert_eq!(xds_dir(u.as_bytes(), "h").unwrap(), *p);
        }
    }

    #[test]
    fn xds_folder_from_the_target() {
        assert_eq!(xds_dir(b"file:///home/u/Desktop/a.txt", "pc"), Ok(PathBuf::from("/home/u/Desktop")));
        assert_eq!(xds_dir(b"file://pc/home/u/a%20b/x.txt\0", "pc"), Ok(PathBuf::from("/home/u/a b")));
        assert_eq!(xds_dir(b"file://localhost/srv/x.txt\r\n", "pc"), Ok(PathBuf::from("/srv")));
        assert_eq!(xds_dir(b"file://other/home/x.txt", "pc"), Err(XdsError::OtherHost));
        for bad in [&b"http://pc/x.txt"[..], b"file://", b"file:///", b"x.txt", b"file:///a%2", b"file:///a%00b/c"] {
            assert_eq!(xds_dir(bad, "pc"), Err(XdsError::NotFile), "{}", String::from_utf8_lossy(bad));
        }
    }

    fn remote() -> Offer {
        Offer { uri_list: None, xds_name: Some("zpráva.txt".into()), allow_move: false }
    }

    #[test]
    fn xds_negotiation() {
        let mut ex = Exchange::with_host(remote(), "pc");
        assert_eq!(ex.offer().types(), vec![XDS, URI_LIST], "XDS first");
        assert_eq!(ex.request(Ask::Targets), (Reply::Targets, None));
        // The target chose a path: we get its folder and answer later.
        let (r, need) = ex.request(Ask::DirectSave(b"file:///home/u/Documents/zpr%C3%A1va.txt".to_vec()));
        assert_eq!(r, Reply::Wait);
        assert_eq!(need, Some(Need::SaveTo(PathBuf::from("/home/u/Documents"))));
        assert!(ex.is_waiting());
        assert_eq!(ex.data(Some(b"x".to_vec())), None, "not a uri-list request");
        assert_eq!(ex.saved(true), Some(Reply::Data(b"S".to_vec())));
        assert_eq!(ex.method, Some(Method::Xds));
        assert_eq!(ex.saved(true), None, "answered once");
        // A queuing failure: "E".
        let mut ex = Exchange::with_host(remote(), "pc");
        ex.request(Ask::DirectSave(b"file:///tmp/a".to_vec()));
        assert_eq!(ex.saved(false), Some(Reply::Data(b"E".to_vec())));
        // Another computer or garbage: "E" at once, nothing to download.
        let mut ex = Exchange::with_host(remote(), "pc");
        assert_eq!(ex.request(Ask::DirectSave(b"file://nas/share/a".to_vec())), (Reply::Data(b"E".to_vec()), None));
        // Local files offer no XDS.
        let mut ex = Exchange::with_host(Offer { uri_list: Some(b"file:///a\r\n".to_vec()), xds_name: None, allow_move: true }, "pc");
        assert_eq!(ex.offer().types(), vec![URI_LIST]);
        assert_eq!(ex.request(Ask::DirectSave(b"file:///tmp/a".to_vec())), (Reply::Refuse, None));
        assert_eq!(ex.request(Ask::UriList), (Reply::Data(b"file:///a\r\n".to_vec()), None));
        assert_eq!(ex.method, Some(Method::UriList));
        assert_eq!(ex.request(Ask::Other), (Reply::Refuse, None));
    }

    #[test]
    fn temp_folder_negotiation() {
        // A target without XDS asks for the uri-list: download first.
        let mut ex = Exchange::with_host(remote(), "pc");
        assert_eq!(ex.request(Ask::UriList), (Reply::Wait, Some(Need::UriList)));
        assert_eq!(ex.request(Ask::UriList), (Reply::Wait, None), "one download, every asker waits for it");
        assert_eq!(ex.request(Ask::DirectSave(b"file://pc/tmp/a".to_vec())), (Reply::Refuse, None), "no XDS while downloading");
        assert_eq!(ex.data(Some(b"file:///run/x/a\r\n".to_vec())), Some(Reply::Data(b"file:///run/x/a\r\n".to_vec())));
        assert_eq!(ex.method, Some(Method::TempFolder));
        // Asked again (some targets do): the same list, no new download.
        assert_eq!(ex.request(Ask::UriList), (Reply::Data(b"file:///run/x/a\r\n".to_vec()), None));
        // Canceled or failed download: the conversion fails.
        let mut ex = Exchange::with_host(remote(), "pc");
        ex.request(Ask::UriList);
        assert_eq!(ex.data(None), Some(Reply::Refuse));
    }

    #[test]
    fn actions_and_messages() {
        assert_eq!(action_for(true, false), Action::Copy);
        assert_eq!(action_for(true, true), Action::Move);
        assert_eq!(action_for(false, true), Action::Copy, "from a server only copies");
        assert_eq!(msg::enter(7, 5, &[1, 2]), [7, 5 << 24, 1, 2, 0]);
        assert_eq!(msg::enter(7, 5, &[1, 2, 3, 4]), [7, (5 << 24) | 1, 1, 2, 3], "more types: XdndTypeList");
        assert_eq!(msg::position(7, 300, 20, 99, 42), [7, 0, (300 << 16) | 20, 99, 42]);
        assert_eq!(msg::status([9, 3, 0, 0, 42]), (true, 42));
        assert_eq!(msg::status([9, 2, 0, 0, 0]), (false, 0));
        assert_eq!(msg::finished([9, 1, 42, 0, 0], 5), (true, Some(42)));
        assert_eq!(msg::finished([9, 0, 0, 0, 0], 5), (false, Some(0)));
        assert_eq!(msg::finished([9, 0, 0, 0, 0], 4), (true, None), "version 4 does not say");
    }
}
