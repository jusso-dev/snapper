//! Region recording window.
//!
//! A small Stop button sits outside the selected region. The recording itself
//! is a GIF (gifski) or an MP4 (`ffmpeg`). A missing `ffmpeg` is reported and
//! the GIF choice stays available.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use snapper_capture::default_capturer;
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
    if let Some((x, y)) = cursor_in_region(region) {
        stamp_cursor(&mut cropped, x, y, false);
    }
    Ok(cropped)
}

fn cursor_in_region(region: Rect) -> Option<(i32, i32)> {
    use enigo::Mouse;
    let enigo = enigo::Enigo::new(&enigo::Settings::default()).ok()?;
    let (x, y) = enigo.location().ok()?;
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
                eframe::egui::Stroke::new(3.0, eframe::egui::Color32::from_rgb(220, 40, 40)),
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
    let y = if region.y > 80 {
        region.y - 72
    } else {
        region.y + region.height as i32 + 8
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Recording")
            .with_inner_size([280.0, 88.0])
            .with_position(eframe::egui::pos2(region.x as f32, y as f32))
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
