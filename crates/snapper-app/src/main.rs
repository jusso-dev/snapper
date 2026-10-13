mod editor;
mod output;
mod overlay;

fn main() {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let _log = snapper_core::logging::init("snapper-app", verbose);
    match open_editor_path() {
        Ok(Some(path)) => match load_editor_image(&path) {
            Ok(image) => {
                let settings = snapper_core::Settings::load().unwrap_or_else(|err| {
                    eprintln!("settings failed, using defaults: {err}");
                    snapper_core::Settings::default()
                });
                tracing::info!(path = %path, "opening editor");
                editor::run_editor(snapper_editor::Document::new(image), settings);
                return;
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
    if std::env::args().any(|arg| arg == "--debug-overlay") {
        match debug_overlay() {
            Ok(result) => println!("{result:?}"),
            Err(err) => {
                eprintln!("overlay failed: {err}");
                std::process::exit(1);
            }
        }
        return;
    }
    println!(
        "snapper {} (skeleton, GUI lands in M1)",
        env!("CARGO_PKG_VERSION")
    );
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

fn open_editor_path() -> Result<Option<String>, &'static str> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--open-editor" {
            return match args.next() {
                Some(path) => Ok(Some(path)),
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
