//! Process logging and crash reports.
//!
//! Logs go to stderr and a daily file under the OS local data directory.
//! Crash reports are plain text: version, OS, panic message and a backtrace.
//! They never include screenshot bytes. Callers must not log image buffers.

use std::path::{Path, PathBuf};

use tracing_subscriber::fmt::writer::MakeWriterExt;

/// How many daily log files to keep for one app. Older files are deleted on startup.
const LOG_FILES_TO_KEEP: usize = 7;

/// Keeps the background log writer alive. Drop it when the process exits.
#[must_use = "dropping LogGuard shuts down the log writer"]
pub struct LogGuard {
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

/// Directory that holds rotating logs and crash reports.
///
/// `SNAPPER_LOG_DIR` overrides the OS path so tests can use a temporary folder.
pub fn log_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os("SNAPPER_LOG_DIR") {
        if !override_dir.is_empty() {
            return PathBuf::from(override_dir);
        }
    }
    directories::ProjectDirs::from("au", "jusso-dev", "snapper")
        .map(|dirs| dirs.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

/// Start logging for `app`.
///
/// The filter is `debug` when `verbose` is set, otherwise `info`. `RUST_LOG`
/// replaces that default when it is set and non-empty. Daily files are named
/// `{app}.log.YYYY-MM-DD`. At most [`LOG_FILES_TO_KEEP`] of those files are kept.
pub fn init(app: &str, verbose: bool) -> LogGuard {
    let dir = log_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "snapper: could not create log directory {}: {err}",
            dir.display()
        );
    }
    prune_old_logs(&dir, app, LOG_FILES_TO_KEEP);

    let file_appender = tracing_appender::rolling::daily(&dir, format!("{app}.log"));
    let (non_blocking, worker) = tracing_appender::non_blocking(file_appender);
    let rust_log = std::env::var("RUST_LOG").ok();
    let directive = filter_directive(verbose, rust_log.as_deref());
    let filter = tracing_subscriber::EnvFilter::try_new(&directive)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // ANSI codes would be written into the log file as well as the terminal.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(non_blocking.and(std::io::stderr))
        .try_init();

    install_panic_hook(dir, app.to_owned());
    tracing::info!("logging initialised");

    LogGuard { _worker: worker }
}

fn filter_directive(verbose: bool, rust_log: Option<&str>) -> String {
    match rust_log.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value.to_owned(),
        None if verbose => "debug".to_owned(),
        None => "info".to_owned(),
    }
}

fn install_panic_hook(dir: PathBuf, app: String) {
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown".to_owned());
        let backtrace = std::backtrace::Backtrace::force_capture();
        match write_crash_report(&dir, &app, &message, &location, &backtrace.to_string()) {
            Ok(path) => eprintln!("snapper crashed: {message}\nreport: {}", path.display()),
            Err(err) => {
                eprintln!("snapper crashed: {message}\ncould not write crash report: {err}")
            }
        }
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_owned()
    }
}

fn write_crash_report(
    dir: &Path,
    app: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = unique_crash_path(dir);
    let body = format_crash_report(app, message, location, backtrace);
    std::fs::write(&path, body)?;
    Ok(path)
}

fn unique_crash_path(dir: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("crash-{stamp}.txt"));
    if !path.exists() {
        return path;
    }
    dir.join(format!("crash-{stamp}-{}.txt", std::process::id()))
}

/// Plain-text crash report. The arguments are the only content that is written.
fn format_crash_report(app: &str, message: &str, location: &str, backtrace: &str) -> String {
    format!(
        "app: {app}\nversion: {version}\nos: {os}\nlocation: {location}\nmessage: {message}\n\nbacktrace:\n{backtrace}\n",
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
    )
}

fn prune_old_logs(dir: &Path, app: &str, keep: usize) {
    let prefix = format!("{app}.log.");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let today_name = format!("{prefix}{today}");
    let mut names = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&prefix))
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    names.sort();
    let today_exists = names.iter().any(|name| name == &today_name);
    // Leave room for today's file when it has not been created yet.
    let limit = if today_exists {
        keep
    } else {
        keep.saturating_sub(1)
    };
    let excess = names.len().saturating_sub(limit);
    for name in names.into_iter().take(excess) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_directive_defaults_and_rust_log_override() {
        assert_eq!(filter_directive(false, None), "info");
        assert_eq!(filter_directive(true, None), "debug");
        assert_eq!(filter_directive(false, Some("trace")), "trace");
        assert_eq!(filter_directive(true, Some("")), "debug");
        assert_eq!(filter_directive(false, Some("   ")), "info");
        assert_eq!(
            filter_directive(true, Some("info,snapper=debug")),
            "info,snapper=debug"
        );
    }

    #[test]
    fn crash_report_is_text_and_has_no_image_payload() {
        let png_header = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB";
        let report = format_crash_report("snapper-cli", "boom", "main.rs:1:1", "frame 0");
        assert!(report.contains("app: snapper-cli"));
        assert!(report.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains(&format!("os: {}", std::env::consts::OS)));
        assert!(report.contains("location: main.rs:1:1"));
        assert!(report.contains("message: boom"));
        assert!(report.contains("frame 0"));
        assert!(!report.contains(png_header));
        assert!(report.is_char_boundary(report.len()));
    }

    #[test]
    fn prune_keeps_seven_daily_files_and_leaves_crash_reports() {
        let dir = tempfile::tempdir().expect("tempdir");
        for day in 1..=9 {
            let name = format!("snapper-cli.log.2020-01-{day:02}");
            std::fs::write(dir.path().join(name), b"log line\n").expect("write log");
        }
        std::fs::write(dir.path().join("crash-old.txt"), b"crash\n").expect("write crash");

        prune_old_logs(dir.path(), "snapper-cli", 7);

        let names = std::fs::read_dir(dir.path())
            .expect("read")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let mut logs: Vec<_> = names
            .iter()
            .filter(|name| name.starts_with("snapper-cli.log."))
            .cloned()
            .collect();
        logs.sort();
        assert_eq!(
            logs,
            vec![
                "snapper-cli.log.2020-01-04".to_owned(),
                "snapper-cli.log.2020-01-05".to_owned(),
                "snapper-cli.log.2020-01-06".to_owned(),
                "snapper-cli.log.2020-01-07".to_owned(),
                "snapper-cli.log.2020-01-08".to_owned(),
                "snapper-cli.log.2020-01-09".to_owned(),
            ]
        );
        assert!(names.iter().any(|name| name == "crash-old.txt"));
    }
}
