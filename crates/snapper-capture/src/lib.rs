//! Capture backends.
//!
//! [`default_capturer`] returns [`XcapCapturer`] on Windows, macOS and X11.
//! When `WAYLAND_DISPLAY` is set it returns a stub that reports
//! [`CaptureError::Unsupported`] until the Wayland backend lands.

mod stitch;
mod xcap_backend;

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
}

/// A window the user can capture.
///
/// Window enumeration is filled in by a later issue. The trait includes it so
/// callers can depend on one backend interface.
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

/// Backend selected for this process.
///
/// A set `WAYLAND_DISPLAY` selects the Wayland stub. Every other session uses
/// [`XcapCapturer`].
pub fn default_capturer() -> Box<dyn Capturer + Send + Sync> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Box::new(WaylandStub)
    } else {
        Box::new(XcapCapturer)
    }
}

struct WaylandStub;

impl Capturer for WaylandStub {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError> {
        Err(CaptureError::Unsupported(
            "Wayland capture is not implemented yet",
        ))
    }

    fn capture_monitor(&self, _id: u32) -> Result<Image, CaptureError> {
        Err(CaptureError::Unsupported(
            "Wayland capture is not implemented yet",
        ))
    }

    fn capture_all(&self) -> Result<Image, CaptureError> {
        Err(CaptureError::Unsupported(
            "Wayland capture is not implemented yet",
        ))
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        Err(CaptureError::Unsupported(
            "window list on Wayland is not supported",
        ))
    }

    fn capture_window(&self, _id: u64) -> Result<Image, CaptureError> {
        Err(CaptureError::Unsupported(
            "window capture on Wayland is not supported",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn wayland_display_selects_the_stub() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous = std::env::var_os("WAYLAND_DISPLAY");
        std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
        let capturer = default_capturer();
        let monitors = capturer.monitors();
        let windows = capturer.windows();
        let window = capturer.capture_window(1);
        match previous {
            Some(value) => std::env::set_var("WAYLAND_DISPLAY", value),
            None => std::env::remove_var("WAYLAND_DISPLAY"),
        }

        assert_eq!(capturer.name(), "wayland");
        assert!(matches!(monitors, Err(CaptureError::Unsupported(_))));
        assert!(matches!(windows, Err(CaptureError::Unsupported(_))));
        assert!(matches!(window, Err(CaptureError::Unsupported(_))));
    }

    #[test]
    fn other_sessions_use_xcap() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous = std::env::var_os("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_DISPLAY");
        let capturer = default_capturer();
        match previous {
            Some(value) => std::env::set_var("WAYLAND_DISPLAY", value),
            None => std::env::remove_var("WAYLAND_DISPLAY"),
        }
        assert_eq!(capturer.name(), "xcap");
    }
}
