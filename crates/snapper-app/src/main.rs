mod dispatch;
mod editor;
mod hotkeys;
mod login;
mod output;
mod overlay;
mod record_ui;
mod redact;
mod scroll_capture;
mod settings_ui;
mod shell;
mod single_instance;
mod tray;
mod update;
mod workflow;

use std::sync::{mpsc, Arc, Mutex};

use snapper_core::{Rect, Settings};

use dispatch::Command;
use tray::Action;

fn main() {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let _log = snapper_core::logging::init("snapper-app", verbose);
    let args: Vec<String> = std::env::args().skip(1).collect();

    let listener = match single_instance::claim(&args) {
        Ok(single_instance::Claim::Forwarded) => std::process::exit(0),
        Ok(single_instance::Claim::Primary(listener)) => listener,
        Err(err) => {
            eprintln!("single instance failed: {err}");
            std::process::exit(1);
        }
    };

    // Debug tool: print the overlay result and leave. A second process
    // forwards the flag instead of opening its own overlay.
    if args.iter().any(|arg| arg == "--debug-overlay") {
        match debug_overlay() {
            Ok(result) => println!("{result:?}"),
            Err(err) => {
                eprintln!("overlay failed: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mut settings = load_settings();
    if let Err(err) = login::apply(settings.start_at_login) {
        tracing::warn!(error = %err, "could not update start at login");
    }
    if settings.check_updates {
        let _ = std::thread::Builder::new()
            .name("snapper-update".to_owned())
            .spawn(|| {
                if let Err(err) = update::check_quiet() {
                    tracing::debug!(error = %err, "update check failed");
                }
            });
    }

    let ipc = spawn_ipc(listener);

    match open_editor_argument(&args) {
        Ok(Some(path)) => match load_editor_image(&path) {
            Ok(image) => {
                tracing::info!(path = %path, "opening editor");
                editor::run_editor(snapper_editor::Document::new(image), settings.clone());
            }
            Err(err) => {
                eprintln!("editor failed: {err}");
                std::process::exit(1);
            }
        },
        Ok(None) => {}
        Err(err) => {
            eprintln!("editor failed: {err}");
            std::process::exit(1);
        }
    }

    if open_settings_requested(&args) {
        settings = load_settings();
        settings_ui::run(settings.clone());
    }

    let mut last_region: Option<Rect> = None;
    loop {
        settings = load_settings();
        match shell::run(&settings, &ipc) {
            Ok(Some(command)) => {
                if !perform(command, &settings, &mut last_region) {
                    break;
                }
            }
            Ok(None) => break,
            Err(err) => {
                eprintln!("snapper failed: {err}");
                std::process::exit(1);
            }
        }
    }
}

fn perform(command: Command, settings: &Settings, last_region: &mut Option<Rect>) -> bool {
    match command {
        Command::Action(Action::Quit) => false,
        Command::Action(action) => dispatch::perform(action, settings, last_region),
        Command::OpenEditor(path) => {
            match load_editor_image(&path) {
                Ok(image) => {
                    tracing::info!(path = %path, "opening editor");
                    editor::run_editor(snapper_editor::Document::new(image), settings.clone());
                }
                Err(err) => output::notify("Snapper", &format!("editor failed: {err}")),
            }
            true
        }
        Command::DebugOverlay => {
            match debug_overlay() {
                Ok(result) => println!("{result:?}"),
                Err(err) => output::notify("Snapper", &format!("overlay failed: {err}")),
            }
            true
        }
        Command::OpenSettings => {
            settings_ui::run(settings.clone());
            true
        }
    }
}

fn spawn_ipc(
    listener: interprocess::local_socket::Listener,
) -> Arc<Mutex<mpsc::Receiver<Vec<String>>>> {
    let (tx, rx) = mpsc::channel();
    if let Err(err) = std::thread::Builder::new()
        .name("snapper-ipc".to_owned())
        .spawn(move || single_instance::serve(listener, tx))
    {
        eprintln!("single instance listener failed: {err}");
        std::process::exit(1);
    }
    Arc::new(Mutex::new(rx))
}

/// Settings that fail to load fall back to defaults. The unreadable file is
/// copied to `config.toml.bad` first, because the next save from the settings
/// window replaces it.
fn load_settings() -> Settings {
    Settings::load().unwrap_or_else(|err| {
        eprintln!("settings failed, using defaults: {err}");
        let path = snapper_core::settings::config_path();
        let backup = path.with_extension("toml.bad");
        let message = match std::fs::copy(&path, &backup) {
            Ok(_) => format!(
                "settings could not be read ({err}). Defaults are in use, and the old file was copied to {}.",
                backup.display()
            ),
            Err(_) => format!("settings could not be read ({err}). Defaults are in use."),
        };
        output::notify("Snapper", &message);
        Settings::default()
    })
}

fn debug_overlay() -> Result<overlay::OverlayResult, String> {
    let capturer = snapper_capture::default_capturer();
    let monitors = capturer.monitors().map_err(|err| err.to_string())?;
    let windows = match capturer.windows() {
        Ok(windows) => windows,
        Err(snapper_capture::CaptureError::Unsupported(_)) => Vec::new(),
        Err(err) => return Err(err.to_string()),
    };
    let frozen = capturer.capture_all().map_err(|err| err.to_string())?;
    Ok(overlay::run_overlay(frozen, monitors, windows))
}

fn open_settings_requested(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--open-settings")
}

fn open_editor_argument(args: &[String]) -> Result<Option<String>, &'static str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--open-editor" {
            return match args.next() {
                Some(path) => Ok(Some(path.clone())),
                None => Err("--open-editor needs an image path"),
            };
        }
    }
    Ok(None)
}

fn load_editor_image(path: &str) -> Result<snapper_core::Image, String> {
    let image = image::open(path).map_err(|err| err.to_string())?;
    let rgba = image.to_rgba8();
    Ok(snapper_core::Image {
        width: rgba.width(),
        height: rgba.height(),
        scale_factor: 1.0,
        rgba: rgba.into_raw(),
    })
}
