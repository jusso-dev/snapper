//! Run a tray or hotkey [`Action`] once the shell has returned.
//!
//! Overlay and the editor each take the event loop, so they run from here,
//! after the hidden shell has closed, and not from inside its `update`.

use snapper_capture::{default_capturer, CaptureError, MonitorInfo};
use snapper_core::{Image, Rect, Settings};
use snapper_editor::Document;

use crate::editor;
use crate::output;
use crate::overlay::{self, OverlayResult};
use crate::tray::Action;

/// Work the shell, or a forwarded command line, asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Action(Action),
    OpenEditor(String),
    OpenSettings,
    DebugOverlay,
}

/// The first flag in `args` that the resident process should act on.
pub fn command_from_args(args: &[String]) -> Option<Command> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open-editor" => {
                let path = args.next()?.clone();
                return Some(Command::OpenEditor(path));
            }
            "--open-settings" => return Some(Command::OpenSettings),
            "--debug-overlay" => return Some(Command::DebugOverlay),
            _ => {}
        }
    }
    None
}

/// Perform `action`. Returns false when the process should exit.
pub fn perform(action: Action, settings: &Settings, last_region: &mut Option<Rect>) -> bool {
    match action {
        // Region and window both use the selection overlay. A drag is a
        // region. A click, or a double-click, is the window under the pointer.
        Action::CaptureRegion => capture_region(settings, last_region),
        Action::CaptureWindow => capture_region(settings, last_region),
        Action::CaptureFull => capture_full(settings),
        Action::CaptureLastRegion => capture_last(settings, last_region),
        Action::Record => output::notify("Snapper", "recording is not available yet"),
        Action::OpenFolder => open_folder(settings),
        Action::Settings => crate::settings_ui::run(settings.clone()),
        Action::Quit => return false,
    }
    true
}

fn capture_region(settings: &Settings, last_region: &mut Option<Rect>) {
    let capturer = default_capturer();
    let monitors = match capturer.monitors() {
        Ok(monitors) if !monitors.is_empty() => monitors,
        Ok(_) => {
            output::notify("Snapper", "capture failed: no monitors found");
            return;
        }
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    let windows = match capturer.windows() {
        Ok(windows) => windows,
        Err(CaptureError::Unsupported(_)) => Vec::new(),
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    let frozen = match capturer.capture_all() {
        Ok(image) => image,
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    let source = clone_image(&frozen);
    let origin = desktop_origin(&monitors);
    match overlay::run_overlay(frozen, monitors, windows) {
        OverlayResult::Region(rect) => {
            *last_region = Some(rect);
            match crop_desktop(&source, rect, origin.0, origin.1) {
                Ok(image) => deliver(image, settings),
                Err(err) => output::notify("Snapper", &err),
            }
        }
        OverlayResult::Window(id) => match capturer.capture_window(id) {
            Ok(image) => deliver(image, settings),
            Err(err) => notify_capture(err),
        },
        OverlayResult::Cancelled => {}
    }
}

fn capture_full(settings: &Settings) {
    let capturer = default_capturer();
    let image = match capturer.monitors() {
        Ok(monitors) => {
            let primary = monitors
                .iter()
                .find(|monitor| monitor.is_primary)
                .or(monitors.first());
            match primary {
                Some(monitor) => capturer.capture_monitor(monitor.id),
                None => capturer.capture_all(),
            }
        }
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    match image {
        Ok(image) => deliver(image, settings),
        Err(err) => notify_capture(err),
    }
}

fn capture_last(settings: &Settings, last_region: &Option<Rect>) {
    let Some(rect) = *last_region else {
        output::notify("Snapper", "no region has been captured yet");
        return;
    };
    let capturer = default_capturer();
    let monitors = match capturer.monitors() {
        Ok(monitors) => monitors,
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    let image = match capturer.capture_all() {
        Ok(image) => image,
        Err(err) => {
            notify_capture(err);
            return;
        }
    };
    let origin = desktop_origin(&monitors);
    match crop_desktop(&image, rect, origin.0, origin.1) {
        Ok(image) => deliver(image, settings),
        Err(err) => output::notify("Snapper", &err),
    }
}

fn deliver(image: Image, settings: &Settings) {
    if image.width == 0 || image.height == 0 {
        output::notify("Snapper", "the capture was empty");
        return;
    }
    if settings.output.open_editor {
        editor::run_editor(Document::new(image), settings.clone());
        return;
    }
    let ctx = output::context_for(&image);
    output::run(&image, &ctx, settings);
}

fn open_folder(settings: &Settings) {
    let directory = &settings.output.directory;
    if let Err(err) = std::fs::create_dir_all(directory) {
        output::notify(
            "Snapper",
            &format!("could not create the screenshot folder: {err}"),
        );
        return;
    }
    if let Err(err) = opener::open(directory) {
        output::notify(
            "Snapper",
            &format!("could not open the screenshot folder: {err}"),
        );
    }
}

fn notify_capture(err: CaptureError) {
    output::notify("Snapper", &format!("capture failed: {err}"));
}

fn clone_image(image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        scale_factor: image.scale_factor,
        rgba: image.rgba.clone(),
    }
}

fn desktop_origin(monitors: &[MonitorInfo]) -> (i32, i32) {
    let min_x = monitors.iter().map(|monitor| monitor.rect.x).min();
    let min_y = monitors.iter().map(|monitor| monitor.rect.y).min();
    (min_x.unwrap_or(0), min_y.unwrap_or(0))
}

/// Crop `rect` from a stitched desktop whose top-left is `(origin_x, origin_y)`.
fn crop_desktop(image: &Image, rect: Rect, origin_x: i32, origin_y: i32) -> Result<Image, String> {
    let x = rect.x.saturating_sub(origin_x);
    let y = rect.y.saturating_sub(origin_y);
    let x0 = x.max(0).min(image.width as i32) as u32;
    let y0 = y.max(0).min(image.height as i32) as u32;
    let x1 = x
        .saturating_add(rect.width as i32)
        .max(0)
        .min(image.width as i32) as u32;
    let y1 = y
        .saturating_add(rect.height as i32)
        .max(0)
        .min(image.height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Err("the region is outside the screenshot".to_owned());
    }
    let out_w = x1 - x0;
    let out_h = y1 - y0;
    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];
    for row in 0..out_h {
        let src = (((y0 + row) * image.width + x0) * 4) as usize;
        let dst = (row * out_w * 4) as usize;
        let len = (out_w * 4) as usize;
        let Some(source) = image.rgba.get(src..src + len) else {
            return Err("the region is outside the screenshot".to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_flags_map_to_commands() {
        assert_eq!(command_from_args(&[]), None);
        assert_eq!(command_from_args(&["--verbose".to_owned()]), None);
        assert_eq!(
            command_from_args(&["--debug-overlay".to_owned()]),
            Some(Command::DebugOverlay)
        );
        assert_eq!(
            command_from_args(&["--open-editor".to_owned(), "shot.png".to_owned()]),
            Some(Command::OpenEditor("shot.png".to_owned()))
        );
        assert_eq!(command_from_args(&["--open-editor".to_owned()]), None);
        assert_eq!(
            command_from_args(&["--open-settings".to_owned()]),
            Some(Command::OpenSettings)
        );
    }

    #[test]
    fn crop_uses_the_virtual_desktop_origin() {
        let image = Image {
            width: 4,
            height: 2,
            scale_factor: 1.5,
            rgba: (0..8)
                .flat_map(|index| [index, index, index, 255])
                .collect(),
        };
        let rect = Rect {
            x: -1,
            y: 0,
            width: 2,
            height: 1,
        };
        let cropped = crop_desktop(&image, rect, -2, 0).expect("crop");
        assert_eq!((cropped.width, cropped.height), (2, 1));
        assert_eq!(cropped.scale_factor, 1.5);
        // Local x is 1, so the pixels are indexes 1 and 2.
        assert_eq!(&cropped.rgba[..4], &[1, 1, 1, 255]);
        assert_eq!(&cropped.rgba[4..], &[2, 2, 2, 255]);
    }

    #[test]
    fn crop_outside_the_image_is_an_error() {
        let image = Image {
            width: 2,
            height: 2,
            scale_factor: 1.0,
            rgba: vec![0; 16],
        };
        let rect = Rect {
            x: 10,
            y: 10,
            width: 2,
            height: 2,
        };
        assert!(crop_desktop(&image, rect, 0, 0).is_err());
    }
}
