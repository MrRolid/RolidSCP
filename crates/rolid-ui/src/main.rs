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
    // Background runtime for all I/O; the UI thread never blocks on disk or network.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("rolid-io")
        .build()
        .expect("tokio runtime");
    let handle = rt.handle().clone();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("RolidSCP")
            .with_app_id("rolidscp")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
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
