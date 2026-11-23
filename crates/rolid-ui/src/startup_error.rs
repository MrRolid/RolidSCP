//! A start that fails before the window opens (a missing library, no
//! graphics driver) must not end silently: started from a menu, nobody sees
//! the terminal. The error goes to the log and stderr and is shown in a small
//! window: by zenity or kdialog when the system has one, otherwise drawn with
//! libX11 alone (present on every system with an X server or XWayland).

use std::sync::atomic::{AtomicBool, Ordering};

/// Set when the program's window is there; later errors are not start errors.
static STARTED: AtomicBool = AtomicBool::new(false);
/// One message per start, also when a panic follows an error.
static SHOWN: AtomicBool = AtomicBool::new(false);

pub fn started() {
    STARTED.store(true, Ordering::Relaxed);
}

/// Shows start errors from panics too (a library that cannot be loaded makes
/// winit's keyboard handling panic).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if !STARTED.load(Ordering::Relaxed) {
            let what = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            report(&what);
        }
    }));
}

/// The text of the message (also for the log).
pub fn message(error: &str) -> String {
    tr!(
        "RolidSCP could not open its window.\n\n{error}\n\nThe program needs the graphics libraries of your system: EGL or Vulkan (Mesa), X11 or Wayland, and libxkbcommon. Install them with your package manager, or install the deb or rpm package, which brings them along.\n\nThe log: {log}",
        error = error.trim().to_string(),
        log = rolid_config::log_path().display().to_string()
    )
}

/// Log, print and show the error (once).
pub fn report(error: &str) {
    if SHOWN.swap(true, Ordering::Relaxed) {
        return;
    }
    // The language of the system: the settings may be what failed to load.
    rolid_i18n::set_language(None, false, None);
    let text = message(error);
    tracing::error!("start failed: {error}");
    eprintln!("{text}");
    let title = tr!("RolidSCP could not start");
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    if tool(&["zenity", "--error", "--no-markup", "--width=560", "--title", &title, "--text", &text])
        || tool(&["kdialog", "--title", &title, "--error", &text])
    {
        return;
    }
    if std::env::var_os("DISPLAY").is_some() {
        if let Err(e) = xlib::show(&title, &text) {
            tracing::warn!("the start error could not be shown in a window: {e}");
        }
    }
}

/// Runs a dialog tool; false when the system does not have it.
fn tool(args: &[&str]) -> bool {
    std::process::Command::new(args[0])
        .args(&args[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.code().is_some())
}

/// Lines of at most `width` characters (words kept whole where they fit).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let fits = line.chars().count() + 1 + word.chars().count() <= width;
            if !line.is_empty() && !fits {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
            while line.chars().count() > width {
                let cut: String = line.chars().take(width).collect();
                line = line.chars().skip(width).collect();
                out.push(cut);
            }
        }
        out.push(line);
    }
    out
}

mod xlib {
    use std::ffi::CString;
    use std::os::raw::c_int;
    use std::ptr;

    use x11_dl::xlib;

    /// A plain window with the text and an OK area; closed by a key, a click
    /// or the window manager.
    pub fn show(title: &str, text: &str) -> Result<(), String> {
        let x = xlib::Xlib::open().map_err(|e| e.to_string())?;
        let lines: Vec<CString> = super::wrap(text, 78)
            .into_iter()
            .map(|l| CString::new(l.replace('\0', " ")).unwrap_or_default())
            .collect();
        // SAFETY: plain Xlib calls on a display opened here and closed at the
        // end; every pointer passed is valid for the call.
        unsafe {
            libc::setlocale(libc::LC_ALL, c"".as_ptr());
            let d = (x.XOpenDisplay)(ptr::null());
            if d.is_null() {
                return Err("no X display".into());
            }
            // A font set draws UTF-8 (translations); without one, Latin-1 only.
            let mut missing: *mut *mut std::os::raw::c_char = ptr::null_mut();
            let mut n_missing: c_int = 0;
            let mut def: *mut std::os::raw::c_char = ptr::null_mut();
            let fs = if (x.XSupportsLocale)() != 0 {
                (x.XCreateFontSet)(d, c"-*-*-medium-r-normal--14-*-*-*-*-*-*-*,fixed".as_ptr(), &mut missing, &mut n_missing, &mut def)
            } else {
                ptr::null_mut()
            };
            if !missing.is_null() {
                (x.XFreeStringList)(missing);
            }
            let (line_h, char_w) = if fs.is_null() {
                (15, 7)
            } else {
                let w = (x.Xutf8TextEscapement)(fs, c"MMMMMMMMMM".as_ptr(), 10) / 10;
                (17, w.max(6))
            };
            let screen = (x.XDefaultScreen)(d);
            let width = (char_w * 80 + 32).min((x.XDisplayWidth)(d, screen) - 40).max(300);
            let height = (line_h * (lines.len() as c_int + 3) + 24).min((x.XDisplayHeight)(d, screen) - 40);
            let (black, white) = ((x.XBlackPixel)(d, screen), (x.XWhitePixel)(d, screen));
            let win = (x.XCreateSimpleWindow)(d, (x.XRootWindow)(d, screen), 40, 40, width as u32, height as u32, 1, black, white);
            let t = CString::new(title.replace('\0', " ")).unwrap_or_default();
            (x.XStoreName)(d, win, t.as_ptr());
            let mut del = (x.XInternAtom)(d, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (x.XSetWMProtocols)(d, win, &mut del, 1);
            (x.XSelectInput)(d, win, xlib::ExposureMask | xlib::KeyPressMask | xlib::ButtonPressMask);
            (x.XMapWindow)(d, win);
            let gc = (x.XDefaultGC)(d, screen);
            let mut ev: xlib::XEvent = std::mem::zeroed();
            loop {
                (x.XNextEvent)(d, &mut ev);
                match ev.get_type() {
                    xlib::Expose => {
                        (x.XSetForeground)(d, gc, black);
                        for (i, l) in lines.iter().enumerate() {
                            let y = 16 + line_h * (i as c_int + 1);
                            let len = l.as_bytes().len() as c_int;
                            if fs.is_null() {
                                (x.XDrawString)(d, win, gc, 16, y, l.as_ptr(), len);
                            } else {
                                (x.Xutf8DrawString)(d, win, fs, gc, 16, y, l.as_ptr(), len);
                            }
                        }
                        let ok = c"OK";
                        let y = height - 14;
                        (x.XDrawRectangle)(d, win, gc, width / 2 - 30, y - line_h, 60, (line_h + 6) as u32);
                        if fs.is_null() {
                            (x.XDrawString)(d, win, gc, width / 2 - 7, y, ok.as_ptr(), 2);
                        } else {
                            (x.Xutf8DrawString)(d, win, fs, gc, width / 2 - 8, y, ok.as_ptr(), 2);
                        }
                    }
                    xlib::KeyPress | xlib::ButtonPress => break,
                    xlib::ClientMessage if ev.client_message.data.get_long(0) as u64 == del => break,
                    _ => {}
                }
            }
            if !fs.is_null() {
                (x.XFreeFontSet)(d, fs);
            }
            (x.XCloseDisplay)(d);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_keeps_words_and_cuts_only_what_is_too_long() {
        assert_eq!(wrap("a bb ccc", 4), vec!["a bb", "ccc"]);
        assert_eq!(wrap("one\n\ntwo", 10), vec!["one", "", "two"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert!(wrap(&"word ".repeat(100), 78).iter().all(|l| l.chars().count() <= 78));
    }

    #[test]
    fn the_message_names_the_error_and_the_log() {
        let m = message("Library libxkbcommon-x11.so could not be loaded.");
        assert!(m.contains("libxkbcommon-x11.so could not be loaded"));
        assert!(m.contains("rolidscp.log"));
    }
}
