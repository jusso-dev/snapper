//! Region recording window.
//!
//! A small Stop button sits outside the selected region. The recording itself
//! is a GIF (gifski) or an MP4 (`ffmpeg`). A missing `ffmpeg` is reported and
//! the GIF choice stays available.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use snapper_capture::{default_capturer, MonitorInfo};
use snapper_core::template::{self, TemplateContext};
use snapper_core::{Image, Rect, Settings};
use snapper_record::{stamp_cursor, RecordFormat, Recorder};

use crate::output;
use crate::overlay::{self, OverlayResult};

/// Pick a region and record until the user presses Stop.
pub fn run(settings: &Settings) {
    let capturer = default_capturer();
    if capturer.name() == "wayland" {
        output::notify(
            "Snapper",
            "screen recording on Wayland is not available yet. The XDG ScreenCast portal is a later step.",
        );
        return;
    }
    let monitors = match capturer.monitors() {
        Ok(monitors) if !monitors.is_empty() => monitors,
        Ok(_) => {
            output::notify("Snapper", "capture failed: no monitors found");
            return;
        }
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return;
        }
    };
    let windows = match capturer.windows() {
        Ok(windows) => windows,
        Err(snapper_capture::CaptureError::Unsupported(_)) => Vec::new(),
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return;
        }
    };
    let frozen = match capturer.capture_all() {
        Ok(image) => image,
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return;
        }
    };
    let region = match overlay::run_overlay(frozen, monitors, windows) {
        OverlayResult::Region(rect) => rect,
        OverlayResult::Window(_) | OverlayResult::Cancelled => return,
    };
    let choice = choose_format();
    let Some(format) = choice else {
        return;
    };
    let path = output_path(settings, format);
    match record_region(region, path, format) {
        Ok(path) => output::notify("Snapper", &format!("Saved {}", path.display())),
        Err(err) => output::notify("Snapper", &err),
    }
}

fn output_path(settings: &Settings, format: RecordFormat) -> PathBuf {
    let _ = std::fs::create_dir_all(&settings.output.directory);
    let ctx = TemplateContext {
        now: chrono::Local::now(),
        app: None,
        title: None,
        width: 0,
        height: 0,
        counter: 1,
    };
    let stem = template::sanitise(&template::render(&settings.output.filename_template, &ctx));
    let stem = if stem.is_empty() {
        "Snapper".to_owned()
    } else {
        stem
    };
    let ext = match format {
        RecordFormat::Gif => "gif",
        RecordFormat::Mp4 => "mp4",
    };
    template::unique_path(&settings.output.directory, &stem, ext)
}

fn record_region(region: Rect, path: PathBuf, format: RecordFormat) -> Result<PathBuf, String> {
    let mut out = path;
    if format == RecordFormat::Mp4 {
        out.set_extension("mp4");
    } else {
        out.set_extension("gif");
    }
    let recorder = Recorder::new(region, out);
    let region = recorder.region;
    let handle = recorder
        .start_with(move || capture_with_cursor(region))
        .map_err(|err| err.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    stop_window(region, Arc::clone(&stop));
    let path = handle.stop().map_err(|err| err.to_string())?;
    let _ = stop;
    Ok(path)
}

fn capture_with_cursor(region: Rect) -> Result<Image, snapper_record::RecordError> {
    let capturer = default_capturer();
    let monitors = capturer
        .monitors()
        .map_err(|err| snapper_record::RecordError::Backend(err.to_string()))?;
    let image = capturer
        .capture_all()
        .map_err(|err| snapper_record::RecordError::Backend(err.to_string()))?;
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
    let mut cropped = snapper_capture::crop(
        &image,
        Rect {
            x: region.x.saturating_sub(origin_x),
            y: region.y.saturating_sub(origin_y),
            width: region.width,
            height: region.height,
        },
    )
    .map_err(|err| snapper_record::RecordError::Backend(err.to_string()))?;
    if let Some((x, y)) = cursor_in_region(region, &monitors) {
        stamp_cursor(&mut cropped, x, y, false);
    }
    Ok(cropped)
}

fn cursor_in_region(region: Rect, monitors: &[MonitorInfo]) -> Option<(i32, i32)> {
    use enigo::Mouse;
    let enigo = enigo::Enigo::new(&enigo::Settings::default()).ok()?;
    let (x, y) = enigo.location().ok()?;
    // enigo reports points on macOS and physical pixels elsewhere.
    let (x, y) = if cfg!(target_os = "macos") {
        points_to_physical(monitors, x, y)?
    } else {
        (x, y)
    };
    let local_x = x - region.x;
    let local_y = y - region.y;
    if local_x < 0
        || local_y < 0
        || local_x >= region.width as i32
        || local_y >= region.height as i32
    {
        None
    } else {
        Some((local_x, local_y))
    }
}

fn scale(monitor: &MonitorInfo) -> f32 {
    if monitor.scale_factor.is_finite() && monitor.scale_factor > 0.0 {
        monitor.scale_factor
    } else {
        1.0
    }
}

/// A global point position to physical pixels, using the monitor it falls on.
fn points_to_physical(monitors: &[MonitorInfo], x: i32, y: i32) -> Option<(i32, i32)> {
    monitors.iter().find_map(|monitor| {
        let scale = scale(monitor);
        let left = monitor.rect.x as f32 / scale;
        let top = monitor.rect.y as f32 / scale;
        let (x, y) = (x as f32, y as f32);
        let inside = x >= left
            && y >= top
            && x < left + monitor.rect.width as f32 / scale
            && y < top + monitor.rect.height as f32 / scale;
        inside.then(|| {
            (
                monitor.rect.x + ((x - left) * scale).round() as i32,
                monitor.rect.y + ((y - top) * scale).round() as i32,
            )
        })
    })
}

/// The logical top and bottom edges and left edge of a physical `region`.
fn logical_region(monitors: &[MonitorInfo], region: Rect) -> (f32, f32, f32) {
    let monitor = monitors.iter().find(|monitor| {
        region.x >= monitor.rect.x
            && region.y >= monitor.rect.y
            && region.x < monitor.rect.x + monitor.rect.width as i32
            && region.y < monitor.rect.y + monitor.rect.height as i32
    });
    let scale = monitor.map_or(1.0, scale);
    let left = region.x as f32 / scale;
    let top = region.y as f32 / scale;
    (left, top, top + region.height as f32 / scale)
}

struct StopApp {
    stop: Arc<AtomicBool>,
}

impl eframe::App for StopApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Recording. A red border is drawn around this button, outside the region, so the capture is not covered.");
            let response = ui.button("Stop");
            let rect = response.rect.expand(4.0);
            ui.painter().rect_stroke(
                rect,
                0.0,
                eframe::egui::Stroke::new(3.0_f32, eframe::egui::Color32::from_rgb(220, 40, 40)),
            );
            if response.clicked() {
                self.stop.store(true, Ordering::Relaxed);
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn stop_window(region: Rect, stop: Arc<AtomicBool>) {
    // The viewport is placed in logical points; `region` is in physical pixels.
    let monitors = default_capturer().monitors().unwrap_or_default();
    let (x, top, bottom) = logical_region(&monitors, region);
    let y = if top > 80.0 { top - 72.0 } else { bottom + 8.0 };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Recording")
            .with_inner_size([280.0, 88.0])
            .with_position(eframe::egui::pos2(x, y))
            .with_always_on_top()
            .with_resizable(false),
        run_and_return: true,
        persist_window: false,
        ..Default::default()
    };
    let app = StopApp {
        stop: Arc::clone(&stop),
    };
    let _ = eframe::run_native(
        "Snapper record",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    );
    stop.store(true, Ordering::Relaxed);
}

fn choose_format() -> Option<RecordFormat> {
    let (tx, rx) = std::sync::mpsc::channel();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Record")
            .with_inner_size([280.0, 110.0])
            .with_resizable(false),
        run_and_return: true,
        persist_window: false,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Snapper record format",
        options,
        Box::new(move |_cc| Ok(Box::new(ChooseSend { tx: Some(tx) }))),
    );
    rx.try_recv().ok()
}

struct ChooseSend {
    tx: Option<std::sync::mpsc::Sender<RecordFormat>>,
}

impl eframe::App for ChooseSend {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Record this region");
            ui.horizontal(|ui| {
                if ui.button("GIF").clicked() {
                    if let Some(tx) = self.tx.take() {
                        let _ = tx.send(RecordFormat::Gif);
                    }
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                }
                if ui.button("MP4").clicked() {
                    if let Some(tx) = self.tx.take() {
                        let _ = tx.send(RecordFormat::Mp4);
                    }
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                }
            });
        });
    }
}
