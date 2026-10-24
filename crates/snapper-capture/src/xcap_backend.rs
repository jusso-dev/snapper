//! Screen capture through [`xcap`] on Windows, macOS and X11.

use snapper_core::{Image, Rect};

use crate::stitch::{stitch, PlacedCapture};
use crate::{CaptureError, Capturer, MonitorInfo, WindowInfo};

/// Captures monitors with `xcap`.
///
/// Window capture is intentionally unsupported here; that lands in a later issue.
#[derive(Debug, Default)]
pub struct XcapCapturer;

impl XcapCapturer {
    fn monitor(&self, id: u32) -> Result<xcap::Monitor, CaptureError> {
        xcap::Monitor::all()
            .map_err(map_xcap_error)?
            .into_iter()
            .find(|monitor| monitor.id().ok() == Some(id))
            .ok_or_else(|| CaptureError::Backend(format!("unknown monitor {id}")))
    }
}

impl Capturer for XcapCapturer {
    fn name(&self) -> &'static str {
        "xcap"
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError> {
        let mut infos = Vec::new();
        for monitor in xcap::Monitor::all().map_err(map_xcap_error)? {
            infos.push(monitor_info(&monitor)?);
        }
        if cfg!(target_os = "macos") {
            place_points_in_pixels(&mut infos);
        }
        infos.sort_by(|left, right| {
            left.rect
                .y
                .cmp(&right.rect.y)
                .then(left.rect.x.cmp(&right.rect.x))
                .then(left.id.cmp(&right.id))
        });
        Ok(infos)
    }

    fn capture_monitor(&self, id: u32) -> Result<Image, CaptureError> {
        let monitor = self.monitor(id)?;
        let info = self
            .monitors()?
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| CaptureError::Backend(format!("unknown monitor {id}")))?;
        let captured = monitor.capture_image().map_err(map_xcap_error)?;
        let image = image_from_rgba(captured, info.scale_factor);
        if image.width != info.rect.width || image.height != info.rect.height {
            return Err(CaptureError::Backend(format!(
                "monitor {id} bitmap is {}x{} but its rect is {}x{}",
                image.width, image.height, info.rect.width, info.rect.height
            )));
        }
        reject_black_frame_on_macos(&image)?;
        Ok(image)
    }

    fn capture_all(&self) -> Result<Image, CaptureError> {
        let monitors = self.monitors()?;
        if monitors.is_empty() {
            return Err(CaptureError::Backend("no monitors found".to_owned()));
        }
        let scale = monitors
            .iter()
            .find(|monitor| monitor.is_primary)
            .map(|monitor| monitor.scale_factor)
            .unwrap_or(monitors[0].scale_factor);
        let mut parts = Vec::with_capacity(monitors.len());
        for monitor in monitors {
            let image = self.capture_monitor(monitor.id)?;
            parts.push(PlacedCapture {
                x: monitor.rect.x,
                y: monitor.rect.y,
                image,
            });
        }
        let image = stitch(&parts, scale)?;
        reject_black_frame_on_macos(&image)?;
        Ok(image)
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        // `xcap` already returns windows front-first. Keep that order so
        // `window_at` hits the top window.
        let own_pid = std::process::id();
        let monitors = if cfg!(target_os = "macos") {
            self.monitors()?
        } else {
            Vec::new()
        };
        let mut infos = Vec::new();
        for window in xcap::Window::all().map_err(map_xcap_error)? {
            if let Some(mut info) = visible_window(&window, own_pid) {
                if cfg!(target_os = "macos") {
                    info.rect = window_points_to_pixels(info.rect, &monitors);
                }
                infos.push(info);
            }
        }
        Ok(infos)
    }

    fn capture_window(&self, id: u64) -> Result<Image, CaptureError> {
        let own_pid = std::process::id();
        let window = xcap::Window::all()
            .map_err(map_xcap_error)?
            .into_iter()
            .find(|window| window.id().ok().map(u64::from) == Some(id))
            .ok_or_else(|| CaptureError::Backend(format!("unknown window {id}")))?;
        if window.pid().ok() == Some(own_pid) {
            return Err(CaptureError::Unsupported(
                "capturing Snapper's own windows is not supported",
            ));
        }
        if window.is_minimized().unwrap_or(false) {
            return Err(CaptureError::Unsupported(
                "minimised windows cannot be captured",
            ));
        }
        let scale = window
            .current_monitor()
            .ok()
            .and_then(|monitor| monitor.scale_factor().ok())
            .unwrap_or(1.0);
        let captured = window.capture_image().map_err(map_xcap_error)?;
        let image = image_from_rgba(captured, scale);
        reject_black_frame_on_macos(&image)?;
        Ok(image)
    }
}

/// A window the grab list should show: a real app window with area, not shell
/// chrome and not this process.
fn visible_window(window: &xcap::Window, own_pid: u32) -> Option<WindowInfo> {
    let pid = window.pid().ok()?;
    let width = window.width().ok()?;
    let height = window.height().ok()?;
    let title = window.title().ok()?;
    let app_name = window.app_name().unwrap_or_default();
    let minimised = window.is_minimized().unwrap_or(false);
    if !keep_window(&title, &app_name, width, height, pid, own_pid, minimised) {
        return None;
    }
    let rect = Rect {
        x: window.x().ok()?,
        y: window.y().ok()?,
        width,
        height,
    };
    Some(WindowInfo {
        id: u64::from(window.id().ok()?),
        title,
        app_name,
        rect,
        is_minimised: minimised,
    })
}

fn keep_window(
    title: &str,
    app_name: &str,
    width: u32,
    height: u32,
    pid: u32,
    own_pid: u32,
    minimised: bool,
) -> bool {
    width > 0 && height > 0 && !minimised && pid != own_pid && !is_shell_chrome(title, app_name)
}

/// Taskbar, dock, and desktop windows. An empty title counts only when the
/// app is the shell (or has no app name). Other apps may have an empty title.
fn is_shell_chrome(title: &str, app_name: &str) -> bool {
    let title_l = title.trim().to_ascii_lowercase();
    let app_l = app_name.trim().to_ascii_lowercase();
    const SHELL_TITLES: &[&str] = &[
        "program manager",
        "progman",
        "shell_traywnd",
        "shell_secondarytraywnd",
        "taskbar",
        "desktop",
        "dock",
    ];
    const SHELL_APPS: &[&str] = &[
        "program manager",
        "progman",
        "shell_traywnd",
        "shell_secondarytraywnd",
        "taskbar",
        "desktop",
        "dock",
        "finder",
        "explorer",
        "explorer.exe",
        "windows explorer",
        "gnome-shell",
        "plasmashell",
        "window server",
    ];
    if SHELL_TITLES.contains(&title_l.as_str()) {
        return true;
    }
    title_l.is_empty() && (app_l.is_empty() || SHELL_APPS.contains(&app_l.as_str()))
}

fn monitor_info(monitor: &xcap::Monitor) -> Result<MonitorInfo, CaptureError> {
    let id = monitor.id().map_err(map_xcap_error)?;
    let name = monitor.name().unwrap_or_else(|_| format!("Monitor {id}"));
    let scale_factor = monitor.scale_factor().unwrap_or(1.0);
    let rect = snapper_core::Rect {
        x: monitor.x().map_err(map_xcap_error)?,
        y: monitor.y().map_err(map_xcap_error)?,
        width: monitor.width().map_err(map_xcap_error)?,
        height: monitor.height().map_err(map_xcap_error)?,
    };
    let safe_scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    Ok(MonitorInfo {
        id,
        name,
        rect,
        scale_factor,
        is_primary: monitor.is_primary().unwrap_or(false),
        logical_origin: (rect.x as f32 / safe_scale, rect.y as f32 / safe_scale),
    })
}

/// xcap reports macOS monitor and window bounds in points, but its bitmaps and
/// the [`MonitorInfo`] and [`WindowInfo`] rects are in physical pixels.
///
/// macOS has no single pixel space across displays with different scales. Each
/// display's origin is multiplied by the largest scale, and its size by its
/// own, so displays never overlap; a lower-scale display leaves a gap after
/// it. `logical_origin` keeps the origin in points for placing windows.
fn place_points_in_pixels(monitors: &mut [MonitorInfo]) {
    let largest = monitors
        .iter()
        .map(|monitor| safe(monitor.scale_factor))
        .fold(1.0f64, f64::max);
    for monitor in monitors {
        let own = safe(monitor.scale_factor);
        let points = monitor.rect;
        monitor.logical_origin = (points.x as f32, points.y as f32);
        monitor.rect = Rect {
            x: (f64::from(points.x) * largest).round() as i32,
            y: (f64::from(points.y) * largest).round() as i32,
            width: (f64::from(points.width) * own).round() as u32,
            height: (f64::from(points.height) * own).round() as u32,
        };
    }
}

/// A window rect in points to pixels, through the display holding its
/// top-left corner (or the first display when none does).
fn window_points_to_pixels(points: Rect, monitors: &[MonitorInfo]) -> Rect {
    let holds = |monitor: &&MonitorInfo| {
        let (left, top) = monitor.logical_origin;
        let scale = safe(monitor.scale_factor) as f32;
        let (x, y) = (points.x as f32, points.y as f32);
        x >= left
            && y >= top
            && x < left + monitor.rect.width as f32 / scale
            && y < top + monitor.rect.height as f32 / scale
    };
    let Some(monitor) = monitors.iter().find(holds).or_else(|| monitors.first()) else {
        return points;
    };
    let (left, top) = monitor.logical_origin;
    let scale = safe(monitor.scale_factor);
    Rect {
        x: monitor.rect.x + ((f64::from(points.x) - f64::from(left)) * scale).round() as i32,
        y: monitor.rect.y + ((f64::from(points.y) - f64::from(top)) * scale).round() as i32,
        width: (f64::from(points.width) * scale).round() as u32,
        height: (f64::from(points.height) * scale).round() as u32,
    }
}

fn safe(scale: f32) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        f64::from(scale)
    } else {
        1.0
    }
}

fn image_from_rgba(image: image::RgbaImage, scale_factor: f32) -> Image {
    let width = image.width();
    let height = image.height();
    Image {
        width,
        height,
        scale_factor,
        rgba: image.into_raw(),
    }
}

fn map_xcap_error(err: xcap::XCapError) -> CaptureError {
    let text = err.to_string();
    if cfg!(target_os = "macos") && is_permission_error(&text) {
        return CaptureError::PermissionDenied;
    }
    CaptureError::Backend(text)
}

/// Screen Recording failures on macOS mention permission or authorisation.
fn is_permission_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("permission")
        || lower.contains("screen recording")
        || lower.contains("not authorized")
}

/// macOS returns a black frame when Screen Recording permission is missing.
fn reject_black_frame_on_macos(image: &Image) -> Result<(), CaptureError> {
    if cfg!(target_os = "macos") && !image.rgba.is_empty() && is_black(image) {
        return Err(CaptureError::PermissionDenied);
    }
    Ok(())
}

fn is_black(image: &Image) -> bool {
    image
        .rgba
        .chunks(4)
        .all(|pixel| pixel.len() == 4 && pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: u32, x: i32, y: i32, width: u32, height: u32, scale: f32) -> MonitorInfo {
        MonitorInfo {
            id,
            name: format!("d{id}"),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            scale_factor: scale,
            is_primary: id == 1,
            logical_origin: (0.0, 0.0),
        }
    }

    #[test]
    fn retina_points_become_pixels() {
        let mut monitors = [display(1, 0, 0, 1440, 900, 2.0)];
        place_points_in_pixels(&mut monitors);
        assert_eq!(
            monitors[0].rect,
            Rect {
                x: 0,
                y: 0,
                width: 2880,
                height: 1800,
            }
        );
        assert_eq!(monitors[0].logical_origin, (0.0, 0.0));
    }

    #[test]
    fn mixed_scale_displays_do_not_overlap() {
        // A 2x laptop with a 1x display to its right and one to its left.
        let mut monitors = [
            display(1, 0, 0, 1512, 982, 2.0),
            display(2, 1512, 0, 1920, 1080, 1.0),
            display(3, -1920, 0, 1920, 1080, 1.0),
        ];
        place_points_in_pixels(&mut monitors);
        let [laptop, right, left] = &monitors;
        assert_eq!((laptop.rect.x, laptop.rect.width), (0, 3024));
        assert_eq!((right.rect.x, right.rect.width), (3024, 1920));
        assert_eq!((left.rect.x, left.rect.width), (-3840, 1920));
        assert!(left.rect.x + left.rect.width as i32 <= laptop.rect.x);
        assert_eq!(right.logical_origin, (1512.0, 0.0));
    }

    #[test]
    fn window_points_map_through_their_display() {
        let mut monitors = [
            display(1, 0, 0, 1512, 982, 2.0),
            display(2, 1512, 0, 1920, 1080, 1.0),
        ];
        place_points_in_pixels(&mut monitors);
        let on_laptop = Rect {
            x: 100,
            y: 50,
            width: 400,
            height: 300,
        };
        assert_eq!(
            window_points_to_pixels(on_laptop, &monitors),
            Rect {
                x: 200,
                y: 100,
                width: 800,
                height: 600,
            }
        );
        let on_external = Rect {
            x: 1612,
            y: 50,
            width: 400,
            height: 300,
        };
        assert_eq!(
            window_points_to_pixels(on_external, &monitors),
            Rect {
                x: 3124,
                y: 50,
                width: 400,
                height: 300,
            }
        );
    }

    #[test]
    fn backend_name_is_xcap() {
        assert_eq!(XcapCapturer.name(), "xcap");
    }

    #[test]
    fn permission_like_errors_are_recognised() {
        assert!(is_permission_error(
            "Screen Recording permission is required"
        ));
        assert!(is_permission_error("The user is not authorized"));
        assert!(!is_permission_error("The handle is invalid. (0x80070006)"));
    }

    #[test]
    fn an_all_black_frame_is_detected() {
        let black = Image {
            width: 2,
            height: 1,
            scale_factor: 1.0,
            rgba: vec![0, 0, 0, 255, 0, 0, 0, 0],
        };
        let coloured = Image {
            width: 1,
            height: 1,
            scale_factor: 1.0,
            rgba: vec![0, 0, 1, 255],
        };
        assert!(is_black(&black));
        assert!(!is_black(&coloured));
    }

    #[test]
    fn shell_chrome_and_own_windows_are_filtered() {
        assert!(keep_window("Notes", "Code", 10, 10, 4, 9, false));
        assert!(!keep_window("Notes", "Code", 10, 10, 4, 4, false));
        assert!(!keep_window("Notes", "Code", 0, 10, 4, 9, false));
        assert!(!keep_window("Notes", "Code", 10, 0, 4, 9, false));
        assert!(!keep_window("Notes", "Code", 10, 10, 4, 9, true));
        assert!(!keep_window(
            "Program Manager",
            "explorer.exe",
            800,
            600,
            4,
            9,
            false
        ));
        assert!(!keep_window("", "explorer.exe", 800, 600, 4, 9, false));
        assert!(!keep_window("", "Windows Explorer", 1600, 48, 4, 9, false));
        assert!(keep_window(
            "Documents",
            "Windows Explorer",
            640,
            480,
            4,
            9,
            false
        ));
        assert!(!keep_window("Dock", "Dock", 40, 400, 4, 9, false));
        assert!(!keep_window("", "", 20, 20, 4, 9, false));
        assert!(keep_window("", "Code", 20, 20, 4, 9, false));
        assert!(keep_window(
            "Documents",
            "explorer.exe",
            640,
            480,
            4,
            9,
            false
        ));
    }
}
