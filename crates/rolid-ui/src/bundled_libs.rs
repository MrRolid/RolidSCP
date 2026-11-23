//! Libraries the AppImage carries for systems that lack them.
//!
//! winit, xkbcommon-dl and wayland-sys open the X11, keyboard and Wayland
//! libraries with `dlopen` at run time. The AppImage leaves out what every
//! system with a screen has (libX11, libxcb, libEGL, libwayland-client,
//! fontconfig: the AppImage exclude list) and carries the rest in
//! `usr/lib/rolidscp`. Each of them is taken from the system when the system
//! has it, and only otherwise from that folder, loaded by its full path:
//! later `dlopen` calls by name then find the loaded copy. Nothing else in the
//! process (the graphics driver) or started by it (terminal, editor) sees the
//! folder, unlike `LD_LIBRARY_PATH` or an rpath.
//!
//! The deb and rpm packages have no such folder: nothing happens there.

use std::ffi::CString;
use std::path::{Path, PathBuf};

/// In load order: a library after those it needs. Keep in sync with
/// `packaging/build-appimage.sh`.
pub const LIBS: &[&str] = &[
    "libXext.so.6",
    "libXrender.so.1",
    "libXfixes.so.3",
    "libXi.so.6",
    "libXcursor.so.1",
    "libxcb-xkb.so.1",
    "libxkbcommon.so.0",
    "libxkbcommon-x11.so.0",
    "libwayland-cursor.so.0",
    "libwayland-egl.so.1",
];

/// The folder next to the program (`<prefix>/lib/rolidscp`), when it exists.
pub fn folder() -> Option<PathBuf> {
    let exe = std::fs::canonicalize("/proc/self/exe").ok()?;
    let dir = exe.parent()?.parent()?.join("lib/rolidscp");
    dir.is_dir().then_some(dir)
}

/// What happened to one library.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    System,
    Bundled,
    /// Neither: the system lacks it and the copy did not load (the error).
    Missing(String),
}

fn open(name: &str) -> Result<(), String> {
    let c = CString::new(name).map_err(|e| e.to_string())?;
    // SAFETY: a plain dlopen of a library by name or path; the handle is kept
    // open for the life of the process on purpose (never dlclose'd).
    let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if !h.is_null() {
        return Ok(());
    }
    // SAFETY: dlerror returns a thread-local string or null.
    let e = unsafe { libc::dlerror() };
    Err(if e.is_null() {
        "unknown error".into()
    } else {
        // SAFETY: non-null, NUL-terminated, valid until the next dl* call.
        unsafe { std::ffi::CStr::from_ptr(e) }.to_string_lossy().into_owned()
    })
}

/// Load each library of `dir`: the system's first, the bundled copy otherwise.
pub fn load_from(dir: &Path) -> Vec<(&'static str, Source)> {
    LIBS.iter()
        .filter(|n| dir.join(n).is_file())
        .map(|&n| {
            let src = if open(n).is_ok() {
                Source::System
            } else {
                match open(&dir.join(n).to_string_lossy()) {
                    Ok(()) => Source::Bundled,
                    Err(e) => Source::Missing(e),
                }
            };
            (n, src)
        })
        .collect()
}

/// libxkbcommon refuses to start without the keyboard data folder
/// (`/usr/share/X11/xkb`, package xkb-data or xkeyboard-config), although on
/// X11 and Wayland the layout comes from the server and no file is read. Where
/// the folder is missing, the AppImage's empty one stands in. Call first in
/// `main`, before any other thread: it sets an environment variable.
pub fn prepare_env() -> Option<PathBuf> {
    let dir = folder()?;
    let xkb = dir.parent()?.parent()?.join("share/rolidscp/xkb");
    if std::env::var_os("XKB_CONFIG_ROOT").is_some() || Path::new("/usr/share/X11/xkb").is_dir() || !xkb.is_dir() {
        return None;
    }
    std::env::set_var("XKB_CONFIG_ROOT", &xkb);
    Some(xkb)
}

/// At start, before any window: see the module documentation.
pub fn load() {
    let Some(dir) = folder() else { return };
    for (name, src) in load_from(&dir) {
        match src {
            Source::System => tracing::debug!(name, "library from the system"),
            Source::Bundled => tracing::info!(name, "library missing on the system: the bundled copy is used"),
            // Usually harmless: e.g. the Wayland cursor library on a system without Wayland.
            Source::Missing(e) => tracing::info!(name, error = %e, "library missing on the system, the bundled copy did not load"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deb and rpm installs and `cargo run` have no folder: nothing loads.
    #[test]
    fn no_folder_next_to_a_normal_build() {
        assert_eq!(folder(), None);
    }

    /// A library the system has is taken from it, one it lacks from the
    /// folder (a copy of a system library under a name the system has not).
    #[test]
    fn the_system_first_the_copy_otherwise() {
        let d = std::env::temp_dir().join(format!("rolidscp-libs-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // Only names from LIBS are looked at; an empty folder loads nothing.
        assert!(load_from(&d).is_empty());
        // A broken file under a listed name the system has: the system's is used.
        if open("libXext.so.6").is_ok() {
            std::fs::write(d.join("libXext.so.6"), b"not a library").unwrap();
            assert_eq!(load_from(&d), vec![("libXext.so.6", Source::System)]);
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
