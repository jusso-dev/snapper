mod overlay;

fn main() {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let _log = snapper_core::logging::init("snapper-app", verbose);
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
