//! Wayland screenshots through the XDG Desktop Portal.
//!
//! The portal does not expose a monitor layout or a window list. Capture
//! returns one image of the screen the compositor chose to share. Region
//! selection is a crop of that image, done by Snapper later.

use std::path::Path;

use snapper_core::{Image, Rect};

use crate::{CaptureError, MonitorInfo};

/// Monitor id used for the single synthetic Wayland output.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const WAYLAND_MONITOR_ID: u32 = 1;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const WINDOW_LIST_UNSUPPORTED: &str = "window list is not supported on Wayland";
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const WINDOW_CAPTURE_UNSUPPORTED: &str = "window capture is not supported on Wayland";

/// `true` when this process should use the portal instead of `xcap`.
///
/// `on_linux` is split out so the rule can be tested on other operating systems.
pub fn portal_selected(on_linux: bool, wayland_display: bool, force_x11: bool) -> bool {
    on_linux && wayland_display && !force_x11
}

/// Load a portal screenshot and delete it.
///
/// The file is removed whether or not it was a readable image.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn load_screenshot_file(path: &Path) -> Result<Image, CaptureError> {
    let opened = image::open(path);
    let removed = std::fs::remove_file(path);
    let dynamic = opened.map_err(|err| CaptureError::Backend(err.to_string()))?;
    if let Err(err) = removed {
        if err.kind() != std::io::ErrorKind::NotFound {
            return Err(CaptureError::Backend(format!(
                "could not remove portal screenshot {}: {err}",
                path.display()
            )));
        }
    }
    let rgba = dynamic.to_rgba8();
    Ok(Image {
        width: rgba.width(),
        height: rgba.height(),
        // The portal does not report a scale factor. Callers treat the
        // bitmap size as the monitor size.
        scale_factor: 1.0,
        rgba: rgba.into_raw(),
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn monitor_from_image(image: &Image) -> MonitorInfo {
    MonitorInfo {
        id: WAYLAND_MONITOR_ID,
        name: "Wayland".to_owned(),
        rect: Rect {
            x: 0,
            y: 0,
            width: image.width,
            height: image.height,
        },
        scale_factor: image.scale_factor,
        is_primary: true,
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use crate::{Capturer, WindowInfo};

    use super::*;

    /// Screenshot backend for Wayland sessions.
    pub struct WaylandPortalCapturer {
        rt: tokio::runtime::Runtime,
    }

    impl WaylandPortalCapturer {
        pub fn new() -> Self {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for the Wayland portal");
            Self { rt }
        }

        /// Full-screen capture. `interactive` asks the compositor to show its
        /// own region dialog. The normal path passes `false` and lets Snapper
        /// crop the image.
        pub fn capture(&self, interactive: bool) -> Result<Image, CaptureError> {
            self.rt.block_on(request_screenshot(interactive))
        }

        /// Portal dialog where the user picks the area.
        pub fn capture_interactive(&self) -> Result<Image, CaptureError> {
            self.capture(true)
        }
    }

    impl Capturer for WaylandPortalCapturer {
        fn name(&self) -> &'static str {
            "wayland"
        }

        fn monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError> {
            // The portal has no layout API. One capture gives the image size.
            let image = self.capture(false)?;
            Ok(vec![monitor_from_image(&image)])
        }

        fn capture_monitor(&self, id: u32) -> Result<Image, CaptureError> {
            if id != WAYLAND_MONITOR_ID {
                return Err(CaptureError::Backend(format!("unknown monitor {id}")));
            }
            self.capture(false)
        }

        fn capture_all(&self) -> Result<Image, CaptureError> {
            self.capture(false)
        }

        fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
            Err(CaptureError::Unsupported(WINDOW_LIST_UNSUPPORTED))
        }

        fn capture_window(&self, _id: u64) -> Result<Image, CaptureError> {
            Err(CaptureError::Unsupported(WINDOW_CAPTURE_UNSUPPORTED))
        }
    }

    async fn request_screenshot(interactive: bool) -> Result<Image, CaptureError> {
        let response = ashpd::desktop::screenshot::Screenshot::request()
            .interactive(interactive)
            .send()
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;
        let path = response.uri().to_file_path().map_err(|_| {
            CaptureError::Backend(format!(
                "portal screenshot URI is not a local file: {}",
                response.uri()
            ))
        })?;
        load_screenshot_file(&path)
    }

    fn portal_error(err: impl std::fmt::Display) -> CaptureError {
        let text = err.to_string();
        if text.eq_ignore_ascii_case("cancelled") {
            CaptureError::Backend("screenshot was cancelled".to_owned())
        } else {
            CaptureError::Backend(text)
        }
    }
}

#[cfg(target_os = "linux")]
pub use portal::WaylandPortalCapturer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_selection_requires_linux_wayland_and_not_force_x11() {
        assert!(portal_selected(true, true, false));
        assert!(!portal_selected(true, true, true));
        assert!(!portal_selected(true, false, false));
        assert!(!portal_selected(false, true, false));
        assert!(!portal_selected(false, true, true));
    }

    #[test]
    fn screenshot_file_is_loaded_and_removed() {
        let path = std::env::temp_dir().join(format!(
            "snapper-portal-{}-{}.png",
            std::process::id(),
            "ok"
        ));
        let _ = std::fs::remove_file(&path);
        image::RgbaImage::from_pixel(2, 3, image::Rgba([9, 8, 7, 255]))
            .save(&path)
            .expect("write png");
        let image = load_screenshot_file(&path).expect("load");
        assert!(!path.exists());
        assert_eq!((image.width, image.height, image.scale_factor), (2, 3, 1.0));
        assert_eq!(&image.rgba[..4], &[9, 8, 7, 255]);
    }

    #[test]
    fn unreadable_screenshot_is_still_removed() {
        let path = std::env::temp_dir().join(format!(
            "snapper-portal-{}-{}.png",
            std::process::id(),
            "bad"
        ));
        std::fs::write(&path, b"not a png").expect("write");
        let err = load_screenshot_file(&path).expect_err("bad image");
        assert!(matches!(err, CaptureError::Backend(_)));
        assert!(!path.exists());
    }
}
