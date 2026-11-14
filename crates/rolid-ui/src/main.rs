//! RolidSCP — dual-panel file manager for local disks and SFTP servers.

// tr!, trc!, trn!, trnc! everywhere in the crate.
#[macro_use]
extern crate rolid_i18n;

mod secret;
mod app;
mod archive_fs;
mod checksum;
mod dirsize;
mod fonts;
mod help;
mod icons;
mod look;
mod dialog;
mod open_rule;
mod pane;
mod terminal;
mod version;
#[cfg(test)]
mod bench;

/// Log to `rolid_config::log_path()` (and stderr). Default level: info for our
/// crates, warnings for libraries; `RUST_LOG` overrides. Secrets are never logged.
fn init_logging() {
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,rolid_ssh=info,rolidscp=info"));
    let path = rolid_config::log_path();
    let file = path.parent().and_then(|d| std::fs::create_dir_all(d).ok()).and_then(|_| {
        // Keep the log small: start over when it grows past 2 MB.
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) {
            let _ = std::fs::remove_file(&path);
        }
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        o.open(&path).ok()
    });
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(f))
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), log = %path.display(), "RolidSCP start");
}

fn main() -> eframe::Result {
    // Started by ssh as the SSH_ASKPASS helper of a Ctrl+P terminal: answer and
    // end, before any logging or window.
    if let Some(code) = terminal::helper_if_asked() {
        std::process::exit(code);
    }
    init_logging();
    // The journal of unfinished replacements of files on servers (repaired at
    // the next connection).
    rolid_core::replace::set_journal_dir(rolid_config::state_dir().join("replace"));
    // Background runtime for all I/O; the UI thread never blocks on disk or network.
    let rt = runtime();
    let handle = rt.handle().clone();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("RolidSCP")
            .with_app_id("rolidscp")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        // Installed by the app with a guard instead (fonts::GuardedFontProvider):
        // eframe's own one ends the program on a system without fonts.
        system_font_fallback: false,
        ..Default::default()
    };
    let res = eframe::run_native(
        "RolidSCP",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, handle)))),
    );
    rt.shutdown_timeout(std::time::Duration::from_secs(1));
    res
}

/// Stack of the background threads. Copying, deleting and measuring walk
/// folder trees recursively: a path of 4 096 bytes can be ~2 000 folders deep,
/// which needs more than the default 2 MiB (only what is used takes memory).
const IO_STACK: usize = 16 * 1024 * 1024;

/// Background runtime for all I/O; the UI thread never blocks on disk or network.
pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("rolid-io")
        .thread_stack_size(IO_STACK)
        .build()
        .expect("tokio runtime")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use rolid_core::copy::{CopyJob, Progress};
    use rolid_core::Vfs;

    /// A tree as deep as a path allows (~1 800 folders) is copied, measured and
    /// deleted on the program's runtime without running out of stack (with the
    /// default 2 MiB the program crashed at ~1 000 levels).
    #[test]
    fn a_very_deep_tree_does_not_overflow_the_stack() {
        let base = tempfile::Builder::new().prefix("r").tempdir().unwrap();
        let src = base.path().join("s");
        let mut deep = src.clone();
        for _ in 0..1800 {
            deep.push("d");
        }
        assert!(deep.as_os_str().len() < 4000, "{}", deep.as_os_str().len());
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("f"), "bottom").unwrap();
        let root = base.path().to_string_lossy().into_owned();
        let rt = super::runtime();
        let done = rt
            .block_on(rt.spawn(async move {
                let fs: Arc<dyn Vfs> = Arc::new(rolid_local::LocalFs);
                let stop = AtomicBool::new(false);
                let totals = rolid_core::copy::measure(fs.as_ref(), &[(root.clone(), "s".into())], &stop, &|_| {}).await;
                let cb = |_: &Progress| {};
                let mut job = CopyJob::new(fs.as_ref(), fs.as_ref(), Arc::new(AtomicBool::new(false)), &cb);
                fs.mkdir(&format!("{root}/t")).await.unwrap();
                job.copy_items_retrying(&[(root.clone(), "s".into())], &format!("{root}/t"), 0, |_| async { false }).await.unwrap();
                let copied = job.progress().files_done;
                crate::app::remove_recursive(fs.as_ref(), &format!("{root}/t")).await.unwrap();
                crate::app::remove_recursive(fs.as_ref(), &format!("{root}/s")).await.unwrap();
                (totals.files, copied)
            }))
            .unwrap();
        assert_eq!(done, (1, 1));
        assert!(std::fs::read_dir(base.path()).unwrap().next().is_none());
    }
}
