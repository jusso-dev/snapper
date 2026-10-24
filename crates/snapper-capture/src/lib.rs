//! Capture backends.
//!
//! [`default_capturer`] returns [`XcapCapturer`] on Windows, macOS and X11.
//! On Linux, a set `WAYLAND_DISPLAY` selects the XDG portal capturer unless
//! `SNAPPER_FORCE_X11` is set. See `docs/platforms/linux-wayland.md`.

pub mod scroll;
mod stitch;
mod wayland_backend;
mod xcap_backend;

#[cfg(target_os = "linux")]
pub use wayland_backend::WaylandPortalCapturer;

pub use stitch::{stitch, PlacedCapture};
pub use xcap_backend::XcapCapturer;

use snapper_core::{Image, Rect};

/// A monitor in physical pixels on the virtual desktop.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    /// Position and size in physical pixels.
    pub rect: Rect,
    pub scale_factor: f32,
    pub is_primary: bool,
    /// Top-left in the windowing system's logical coordinates, where a
    /// window covering this monitor is placed. Usually `rect` divided by
    /// `scale_factor`; on macOS it is the display origin in points.
    pub logical_origin: (f32, f32),
}

/// A window the user can capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_name: String,
    pub rect: Rect,
    pub is_minimised: bool,
}

/// Errors a capture backend can return.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The platform or this build cannot perform the request.
    #[error("{0}")]
    Unsupported(&'static str),
    /// The OS denied screen recording permission.
    #[error("screen recording permission denied")]
    PermissionDenied,
    /// A backend call failed. The string is the backend's own message.
    #[error("{0}")]
    Backend(String),
}

/// Trait every platform backend implements.
pub trait Capturer: Send + Sync {
    /// Short name used in logs, such as `xcap` or `wayland`.
    fn name(&self) -> &'static str;

    /// Monitors in virtual-desktop order, coordinates in physical pixels.
    fn monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError>;

    /// Capture one monitor by [`MonitorInfo::id`].
    fn capture_monitor(&self, id: u32) -> Result<Image, CaptureError>;

    /// Capture every monitor into one image of the virtual desktop.
    fn capture_all(&self) -> Result<Image, CaptureError>;

    /// Visible windows, frontmost first.
    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError>;

    /// Capture one window by [`WindowInfo::id`].
    fn capture_window(&self, id: u64) -> Result<Image, CaptureError>;
}

/// The front-most window whose rectangle contains `(x, y)`.
///
/// `windows` is front-first, which is the order [`Capturer::windows`] returns.
/// The right and bottom edges are exclusive, so adjacent windows do not both
/// contain the shared edge. A zero-size rectangle contains nothing.
pub fn window_at(windows: &[WindowInfo], x: i32, y: i32) -> Option<&WindowInfo> {
    windows
        .iter()
        .find(|window| rect_contains(&window.rect, x, y))
}

fn rect_contains(rect: &Rect, x: i32, y: i32) -> bool {
    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width);
    let bottom = top + i64::from(rect.height);
    let x = i64::from(x);
    let y = i64::from(y);
    x >= left && y >= top && x < right && y < bottom
}

/// Crop `rect` from an image whose origin is the image's top-left pixel.
///
/// Coordinates outside the image are clipped. The crop is empty when the
/// rectangle misses the image entirely.
pub fn crop(image: &Image, rect: Rect) -> Result<Image, CaptureError> {
    if image.width == 0 || image.height == 0 {
        return Err(CaptureError::Backend("image has no pixels".to_owned()));
    }
    let x0 = rect.x.max(0).min(image.width as i32) as u32;
    let y0 = rect.y.max(0).min(image.height as i32) as u32;
    let x1 = rect
        .x
        .saturating_add(rect.width as i32)
        .max(0)
        .min(image.width as i32) as u32;
    let y1 = rect
        .y
        .saturating_add(rect.height as i32)
        .max(0)
        .min(image.height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Err(CaptureError::Backend(
            "the region is outside the screenshot".to_owned(),
        ));
    }
    let out_w = x1 - x0;
    let out_h = y1 - y0;
    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];
    for row in 0..out_h {
        let src = (((y0 + row) * image.width + x0) * 4) as usize;
        let dst = (row * out_w * 4) as usize;
        let len = (out_w * 4) as usize;
        let Some(source) = image.rgba.get(src..src + len) else {
            return Err(CaptureError::Backend(
                "the region is outside the screenshot".to_owned(),
            ));
        };
        rgba[dst..dst + len].copy_from_slice(source);
    }
    Ok(Image {
        width: out_w,
        height: out_h,
        scale_factor: image.scale_factor,
        rgba,
    })
}

/// Backend selected for this process.
///
/// On Linux, `WAYLAND_DISPLAY` selects the XDG portal unless `SNAPPER_FORCE_X11`
/// is set. Every other case uses [`XcapCapturer`].
pub fn default_capturer() -> Box<dyn Capturer + Send + Sync> {
    let use_portal = wayland_backend::portal_selected(
        cfg!(target_os = "linux"),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("SNAPPER_FORCE_X11").is_some(),
    );
    #[cfg(target_os = "linux")]
    if use_portal {
        return Box::new(WaylandPortalCapturer::new());
    }
    #[cfg(not(target_os = "linux"))]
    let _ = use_portal;
    Box::new(XcapCapturer)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn wayland_display_selects_the_portal_only_on_linux() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let previous_force = std::env::var_os("SNAPPER_FORCE_X11");
        std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
        std::env::remove_var("SNAPPER_FORCE_X11");
        let portal = default_capturer();
        std::env::set_var("SNAPPER_FORCE_X11", "1");
        let forced = default_capturer();
        restore("WAYLAND_DISPLAY", previous_wayland);
        restore("SNAPPER_FORCE_X11", previous_force);

        assert_eq!(forced.name(), "xcap");
        if cfg!(target_os = "linux") {
            assert_eq!(portal.name(), "wayland");
            assert!(matches!(
                portal.windows(),
                Err(CaptureError::Unsupported(message)) if message.contains("Wayland")
            ));
        } else {
            // The portal crate is Linux-only, so a stray WAYLAND_DISPLAY stays on xcap.
            assert_eq!(portal.name(), "xcap");
        }
    }

    #[test]
    fn other_sessions_use_xcap() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let previous_force = std::env::var_os("SNAPPER_FORCE_X11");
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("SNAPPER_FORCE_X11");
        let capturer = default_capturer();
        restore("WAYLAND_DISPLAY", previous_wayland);
        restore("SNAPPER_FORCE_X11", previous_force);
        assert_eq!(capturer.name(), "xcap");
    }

    fn window(id: u64, x: i32, y: i32, width: u32, height: u32) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("window {id}"),
            app_name: "app".to_owned(),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            is_minimised: false,
        }
    }

    #[test]
    fn window_at_picks_the_front_window() {
        let front = window(1, 0, 0, 10, 10);
        let behind = window(2, 5, 5, 10, 10);
        let windows = [front, behind];
        assert_eq!(window_at(&windows, 6, 6).map(|window| window.id), Some(1));
        assert_eq!(window_at(&windows, 12, 12).map(|window| window.id), Some(2));
    }

    #[test]
    fn window_at_misses_outside_and_the_exclusive_edge() {
        let windows = [window(1, -4, -2, 4, 2)];
        assert_eq!(window_at(&windows, -4, -2).map(|window| window.id), Some(1));
        assert_eq!(window_at(&windows, -1, -1).map(|window| window.id), Some(1));
        assert_eq!(window_at(&windows, 0, -1), None);
        assert_eq!(window_at(&windows, -1, 0), None);
        assert_eq!(window_at(&windows, -5, -2), None);
        assert_eq!(window_at(&[], 0, 0), None);
    }

    #[test]
    fn window_at_ignores_a_zero_size_rect() {
        let windows = [window(1, 0, 0, 0, 10)];
        assert_eq!(window_at(&windows, 0, 0), None);
    }
}
