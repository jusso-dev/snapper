//! Scrolling capture.
//!
//! The stitcher lives in `snapper-capture`. This module grabs frames, sends
//! wheel events, and stops when the page stops moving, the image is tall
//! enough, or the user presses Stop. Wayland is refused: synthetic input is
//! not allowed there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use snapper_capture::{default_capturer, scroll::stitch};
use snapper_core::{Image, Rect, Settings};

use crate::output;
use crate::overlay::{self, OverlayResult};

const MIN_OVERLAP: u32 = 8;

/// Grab, scroll, and wait injected so tests need no display and no `enigo`.
pub struct ScrollSession<G, W, D> {
    pub grab: G,
    pub wheel: W,
    pub wait: D,
    pub stop: Arc<AtomicBool>,
    pub max_height: u32,
}

/// Capture until the frame repeats, the stitch reaches `max_height`, or `stop` is set.
pub fn run_session<G, W, D>(session: &mut ScrollSession<G, W, D>) -> Result<Image, String>
where
    G: FnMut() -> Result<Image, String>,
    W: FnMut() -> Result<(), String>,
    D: FnMut(),
{
    let mut frames = Vec::new();
    loop {
        if session.stop.load(Ordering::Relaxed) {
            break;
        }
        let frame = (session.grab)()?;
        if frames
            .last()
            .is_some_and(|previous: &Image| same_frame(previous, &frame))
        {
            break;
        }
        frames.push(frame);
        if stitch(&frames, MIN_OVERLAP).height >= session.max_height {
            break;
        }
        if session.stop.load(Ordering::Relaxed) {
            break;
        }
        (session.wheel)()?;
        if session.stop.load(Ordering::Relaxed) {
            break;
        }
        (session.wait)();
    }
    if frames.is_empty() {
        return Err("scrolling capture produced no frames".to_owned());
    }
    Ok(stitch(&frames, MIN_OVERLAP))
}

fn same_frame(left: &Image, right: &Image) -> bool {
    left.width == right.width && left.height == right.height && left.rgba == right.rgba
}

/// Select a region, scroll it, and return the stitched image.
///
/// `Ok(None)` means the user cancelled the selection.
pub fn run(settings: &Settings) -> Result<Option<Image>, String> {
    let capturer = default_capturer();
    if capturer.name() == "wayland" {
        return Err(
            "scrolling capture is not available on Wayland. Synthetic wheel events are not allowed there. Use an X11 session, or set SNAPPER_FORCE_X11=1 when XWayland capture is what you want.".to_owned(),
        );
    }
    let monitors = capturer.monitors().map_err(|err| err.to_string())?;
    if monitors.is_empty() {
        return Err("capture failed: no monitors found".to_owned());
    }
    let windows = match capturer.windows() {
        Ok(windows) => windows,
        Err(snapper_capture::CaptureError::Unsupported(_)) => Vec::new(),
        Err(err) => return Err(err.to_string()),
    };
    let frozen = capturer.capture_all().map_err(|err| err.to_string())?;
    let region = match overlay::run_overlay(frozen, monitors.clone(), windows) {
        OverlayResult::Region(rect) => rect,
        OverlayResult::Window(_) | OverlayResult::Cancelled => return Ok(None),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stop_loop = Arc::clone(&stop);
    let delay = settings.scroll_delay_ms;
    let max_height = settings.scroll_max_height;
    let worker = std::thread::Builder::new()
        .name("snapper-scroll".to_owned())
        .spawn(move || {
            let mut session = ScrollSession {
                grab: || grab_region(region),
                wheel: scroll_down,
                wait: move || std::thread::sleep(std::time::Duration::from_millis(delay)),
                stop: stop_loop,
                max_height,
            };
            run_session(&mut session)
        })
        .map_err(|err| err.to_string())?;
    stop_window(Arc::clone(&stop));
    stop.store(true, Ordering::Relaxed);
    match worker.join() {
        Ok(result) => result.map(Some),
        Err(_) => Err("scrolling capture stopped unexpectedly".to_owned()),
    }
}

fn grab_region(region: Rect) -> Result<Image, String> {
    let capturer = default_capturer();
    let monitors = capturer.monitors().map_err(|err| err.to_string())?;
    let image = capturer.capture_all().map_err(|err| err.to_string())?;
    let origin_x = monitors
        .iter()
        .map(|monitor| monitor.rect.x)
        .min()
        .unwrap_or(0);
    let origin_y = monitors
        .iter()
        .map(|monitor| monitor.rect.y)
        .min()
        .unwrap_or(0);
    snapper_capture::crop(
        &image,
        Rect {
            x: region.x.saturating_sub(origin_x),
            y: region.y.saturating_sub(origin_y),
            width: region.width,
            height: region.height,
        },
    )
    .map_err(|err| err.to_string())
}

fn scroll_down() -> Result<(), String> {
    use enigo::{Axis, Mouse};
    let mut enigo =
        enigo::Enigo::new(&enigo::Settings::default()).map_err(|err| err.to_string())?;
    enigo
        .scroll(3, Axis::Vertical)
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn stop_window(stop: Arc<AtomicBool>) {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Scrolling capture")
            .with_inner_size([220.0, 80.0])
            .with_always_on_top()
            .with_resizable(false),
        run_and_return: true,
        persist_window: false,
        ..Default::default()
    };
    let app = StopApp { stop };
    if let Err(err) = eframe::run_native(
        "Snapper scroll",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    ) {
        output::notify("Snapper", &format!("could not show Stop: {err}"));
    }
}

struct StopApp {
    stop: Arc<AtomicBool>,
}

impl eframe::App for StopApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        if self.stop.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            return;
        }
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Scrolling capture");
            if ui.button("Stop").clicked() {
                self.stop.store(true, Ordering::Relaxed);
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seed: u8) -> Image {
        let mut rgba = vec![0; 4 * 6 * 20];
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            pixel[0] = seed.wrapping_add(index as u8);
            pixel[3] = 255;
        }
        Image {
            width: 6,
            height: 20,
            scale_factor: 1.0,
            rgba,
        }
    }

    #[test]
    fn the_session_stops_when_a_frame_repeats() {
        let frames = [frame(1), frame(2), frame(2)];
        let mut index = 0;
        let mut wheels = 0;
        let mut session = ScrollSession {
            grab: || {
                let image = frames[index].clone();
                index += 1;
                Ok(image)
            },
            wheel: || {
                wheels += 1;
                Ok(())
            },
            wait: || {},
            stop: Arc::new(AtomicBool::new(false)),
            max_height: 30_000,
        };
        let image = run_session(&mut session).expect("stitch");
        assert!(image.height >= 20);
        assert!(image.height < 20 * 3);
        assert_eq!(wheels, 2);
    }

    #[test]
    fn the_session_stops_at_max_height() {
        let mut index = 0u8;
        let mut session = ScrollSession {
            grab: || {
                index = index.wrapping_add(1);
                Ok(frame(index))
            },
            wheel: || Ok(()),
            wait: || {},
            stop: Arc::new(AtomicBool::new(false)),
            max_height: 25,
        };
        let image = run_session(&mut session).expect("stitch");
        assert!(image.height <= 25 || image.height < 80);
        assert!(image.height >= 20);
    }

    #[test]
    fn the_user_can_stop_before_the_next_scroll() {
        let mut grabbed = 0;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let mut session = ScrollSession {
            grab: || {
                grabbed += 1;
                Ok(frame(grabbed))
            },
            wheel: || {
                flag.store(true, Ordering::Relaxed);
                Ok(())
            },
            wait: || {},
            stop,
            max_height: 30_000,
        };
        let image = run_session(&mut session).expect("stitch");
        assert_eq!(grabbed, 1);
        assert_eq!(image.height, 20);
    }
}
