//! Cursor position while files are dragged in from another application on X11.
//!
//! During an XDND drag winit reports no pointer movement, so the panels could
//! only all be highlighted. Here a separate thread asks the X server where the
//! pointer is (XQueryPointer on our window, own pure-Rust connection via
//! x11rb – the same version winit uses, no unsafe code), at most every 30 ms
//! and only while a drag is going on. The UI thread never waits for the X
//! server: it only reads the last answer. On any error the tracker gives up
//! quietly and the panels are highlighted all together, as before.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{pos2, Pos2};

/// Pause between two queries.
pub const PERIOD: Duration = Duration::from_millis(30);
/// Keep querying this long after the UI last asked (covers the drop itself).
const LINGER: Duration = Duration::from_millis(800);
/// An answer older than this is not used.
pub const FRESH: Duration = Duration::from_millis(400);

/// One answer of the X server, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawPointer {
    /// Pointer on the screen (root window).
    pub root: (i32, i32),
    /// Top-left corner of our window's content on the screen (inside the
    /// window manager's frame).
    pub origin: (i32, i32),
}

/// Pointer position in egui points: relative to the window content (the frame
/// of the window manager is not part of it) and divided by the HiDPI scale.
pub fn to_points(p: RawPointer, pixels_per_point: f32) -> Pos2 {
    let ppp = if pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    pos2((p.root.0 - p.origin.0) as f32 / ppp, (p.root.1 - p.origin.1) as f32 / ppp)
}

/// How the window is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// An X11 session (or a Wayland one with `WAYLAND_DISPLAY` emptied).
    X11,
    /// A Wayland session, the window through XWayland: "On Wayland, use X11"
    /// in the settings.
    XWayland,
    /// A Wayland session, the window a Wayland one: no dragging to and from
    /// other programs (winit 0.30 has none on Wayland).
    Wayland,
    /// Neither display is set.
    Unknown,
}

/// "On Wayland, use X11", as read at the start (it takes effect only then).
static X11_ON_WAYLAND: AtomicBool = AtomicBool::new(false);

/// The backend for the displays that are set and the setting.
pub fn backend_for(display: bool, wayland: bool, x11_on_wayland: bool) -> Backend {
    match (display, wayland) {
        (true, false) => Backend::X11,
        (true, true) if x11_on_wayland => Backend::XWayland,
        (_, true) => Backend::Wayland,
        (false, false) => Backend::Unknown,
    }
}

fn env_set(v: &str) -> bool {
    std::env::var_os(v).is_some_and(|s| !s.is_empty())
}

/// Is this a Wayland session where X11 (XWayland) can be chosen? (Both
/// displays are set.)
pub fn x11_can_be_chosen() -> bool {
    env_set("DISPLAY") && env_set("WAYLAND_DISPLAY")
}

/// At the start, before the window: remember the setting and say which
/// backend the window gets.
pub fn choose(x11_on_wayland: bool) -> Backend {
    X11_ON_WAYLAND.store(x11_on_wayland, Ordering::Relaxed);
    let b = backend();
    match b {
        Backend::XWayland => tracing::info!("window system: X11 through XWayland in a Wayland session (Settings: On Wayland, use X11)"),
        Backend::Wayland if x11_on_wayland => tracing::warn!("window system: Wayland; X11 was chosen in the settings, but no X display (XWayland) is set"),
        Backend::Wayland => tracing::info!("window system: Wayland (dragging to and from other programs is not available)"),
        Backend::X11 => tracing::info!("window system: X11"),
        Backend::Unknown => {}
    }
    b
}

/// The backend of this run.
pub fn backend() -> Backend {
    backend_for(env_set("DISPLAY"), env_set("WAYLAND_DISPLAY"), X11_ON_WAYLAND.load(Ordering::Relaxed))
}

/// The window is an X11 one (also through XWayland when chosen in the
/// settings): our XDND code works. Not a Wayland window, where XWayland
/// would give wrong positions.
pub fn is_x11() -> bool {
    matches!(backend(), Backend::X11 | Backend::XWayland)
}

#[derive(Default)]
struct Shared {
    last: Option<(RawPointer, Instant)>,
    failed: bool,
    /// Successful queries so far and why the querying stopped.
    ok: u64,
    error: Option<String>,
}

/// The background querier.
pub struct Tracker {
    wake: Sender<()>,
    shared: Arc<Mutex<Shared>>,
}

impl Tracker {
    /// Start the thread with a query function (`Err` = give up for good).
    pub fn spawn(mut query: impl FnMut() -> Result<RawPointer, String> + Send + 'static) -> Tracker {
        let (wake, rx): (Sender<()>, Receiver<()>) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let sh = shared.clone();
        let spawned = std::thread::Builder::new().name("x11-pointer".into()).spawn(move || {
            let mut wanted: Option<Instant> = None;
            loop {
                // Idle: sleep until the UI asks again.
                if wanted.is_none_or(|w| w.elapsed() > LINGER) {
                    if rx.recv().is_err() {
                        return;
                    }
                    wanted = Some(Instant::now());
                }
                while rx.try_recv().is_ok() {
                    wanted = Some(Instant::now());
                }
                match query() {
                    Ok(p) => {
                        let mut s = sh.lock().unwrap();
                        s.last = Some((p, Instant::now()));
                        s.ok += 1;
                    }
                    Err(e) => {
                        let mut s = sh.lock().unwrap();
                        s.failed = true;
                        s.error = Some(e);
                        return;
                    }
                }
                std::thread::sleep(PERIOD);
            }
        });
        if let Err(e) = spawned {
            let mut s = shared.lock().unwrap();
            s.failed = true;
            s.error = Some(format!("cannot start the thread: {e}"));
        }
        Tracker { wake, shared }
    }

    /// Called every frame while files are dragged over the window.
    pub fn want(&self) {
        let _ = self.wake.send(());
    }

    /// The last known position, if recent enough.
    pub fn latest(&self) -> Option<RawPointer> {
        let s = self.shared.lock().unwrap();
        s.last.filter(|(_, t)| t.elapsed() <= FRESH).map(|(p, _)| p)
    }

    pub fn failed(&self) -> bool {
        self.shared.lock().unwrap().failed
    }

    /// Successful queries so far, and the error that stopped them.
    pub fn status(&self) -> (u64, Option<String>) {
        let s = self.shared.lock().unwrap();
        (s.ok, s.error.clone())
    }
}

/// Query function for the real X server: our window `win` on `DISPLAY`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn x11_query(win: u32) -> impl FnMut() -> Result<RawPointer, String> + Send + 'static {
    use x11rb::protocol::xproto::ConnectionExt;
    let mut conn: Option<x11rb::rust_connection::RustConnection> = None;
    move || {
        if conn.is_none() {
            let (c, _) = x11rb::rust_connection::RustConnection::connect(None).map_err(|e| e.to_string())?;
            conn = Some(c);
        }
        let c = conn.as_ref().unwrap();
        let r = c.query_pointer(win).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        if !r.same_screen {
            return Err("the pointer is on another screen".into());
        }
        // win_x / win_y are relative to our window's content: the origin on
        // the screen is the difference.
        let root = (r.root_x as i32, r.root_y as i32);
        Ok(RawPointer { root, origin: (root.0 - r.win_x as i32, root.1 - r.win_y as i32) })
    }
}

/// X11 window id of the app window (from raw-window-handle).
pub fn window_id(frame: &eframe::Frame) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_follows_the_displays_and_the_setting() {
        assert_eq!(backend_for(true, false, false), Backend::X11);
        assert_eq!(backend_for(true, false, true), Backend::X11, "an X11 session stays X11");
        assert_eq!(backend_for(true, true, false), Backend::Wayland);
        assert_eq!(backend_for(true, true, true), Backend::XWayland);
        assert_eq!(backend_for(false, true, true), Backend::Wayland, "no XWayland: Wayland");
        assert_eq!(backend_for(false, false, true), Backend::Unknown);
    }

    #[test]
    fn screen_to_window_points() {
        // Window content at (104, 78) on the screen: x 100 + 4 px frame,
        // y 50 + 28 px title bar.
        let p = RawPointer { root: (500, 300), origin: (104, 78) };
        assert_eq!(to_points(p, 1.0), pos2(396.0, 222.0));
        // HiDPI: twice as many pixels per point.
        assert_eq!(to_points(p, 2.0), pos2(198.0, 111.0));
        // Pointer left of the window: negative, outside every panel.
        let out = RawPointer { root: (50, 300), origin: (104, 78) };
        assert!(to_points(out, 1.0).x < 0.0);
        assert_eq!(to_points(p, 0.0), pos2(396.0, 222.0), "a broken scale is treated as 1");
    }

    #[test]
    fn a_failing_query_gives_up_quietly() {
        let t = Tracker::spawn(|| Err("connection to the X server failed".into()));
        t.want();
        let start = Instant::now();
        while !t.failed() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(t.failed());
        assert_eq!(t.latest(), None, "no position: all panels are highlighted");
        t.want(); // the thread is gone: asking again does nothing
    }

    #[test]
    fn queries_only_while_wanted_and_answers_go_stale() {
        let n = Arc::new(Mutex::new(0u32));
        let n2 = n.clone();
        let t = Tracker::spawn(move || {
            *n2.lock().unwrap() += 1;
            Ok(RawPointer { root: (10, 20), origin: (0, 0) })
        });
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(*n.lock().unwrap(), 0, "nothing before a drag");
        t.want();
        let start = Instant::now();
        while t.latest().is_none() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(t.latest(), Some(RawPointer { root: (10, 20), origin: (0, 0) }));
        // After the drag it stops (LINGER) and the answer gets stale (FRESH).
        std::thread::sleep(LINGER + FRESH + Duration::from_millis(200));
        let count = *n.lock().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(*n.lock().unwrap(), count, "stopped");
        assert!(count <= ((LINGER.as_millis() + 300) / PERIOD.as_millis()) as u32 + 2, "at most every 30 ms: {count}");
        assert_eq!(t.latest(), None);
    }
}
