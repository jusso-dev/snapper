//! Save one PNG per monitor and a stitched `all.png` in the current directory.
//!
//! On macOS without Screen Recording permission this prints a permission
//! message and exits.

use std::path::Path;
use std::process::ExitCode;

use snapper_capture::CaptureError;
use snapper_core::Image;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CaptureError::PermissionDenied) => {
            eprintln!(
                "Screen Recording permission is required. On macOS, enable Snapper in \
System Settings > Privacy & Security > Screen & System Audio Recording, then run this again."
            );
            ExitCode::from(3)
        }
        Err(CaptureError::Unsupported(message)) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
        Err(err) => {
            eprintln!("capture failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), CaptureError> {
    let capturer = snapper_capture::default_capturer();
    let monitors = capturer.monitors()?;
    if monitors.is_empty() {
        return Err(CaptureError::Backend("no monitors found".to_owned()));
    }
    for monitor in &monitors {
        let image = capturer.capture_monitor(monitor.id)?;
        let path = format!("monitor-{}.png", monitor.id);
        save_png(&image, Path::new(&path))?;
        eprintln!(
            "saved {path} ({}x{} at {},{} scale {})",
            image.width, image.height, monitor.rect.x, monitor.rect.y, monitor.scale_factor
        );
    }
    let all = capturer.capture_all()?;
    save_png(&all, Path::new("all.png"))?;
    eprintln!("saved all.png ({}x{})", all.width, all.height);
    Ok(())
}

fn save_png(image: &Image, path: &Path) -> Result<(), CaptureError> {
    let buffer = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .ok_or_else(|| {
            CaptureError::Backend(format!(
                "image {}x{} does not match its {} byte buffer",
                image.width,
                image.height,
                image.rgba.len()
            ))
        })?;
    buffer
        .save(path)
        .map_err(|err| CaptureError::Backend(err.to_string()))
}
