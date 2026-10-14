//! `snapper-cli` commands.
//!
//! Exit codes: 0 success, 2 usage, 3 screen-recording permission denied,
//! 4 capture, OCR, or recording failed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::{Parser, Subcommand, ValueEnum};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use snapper_capture::{crop, default_capturer, CaptureError, MonitorInfo, WindowInfo};
use snapper_core::settings::ImageFormat;
use snapper_core::template::{self, TemplateContext};
use snapper_core::{Image, Rect, Settings};
use snapper_ocr::{default_engine, join_lines};
use snapper_record::Recorder;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_PERMISSION: i32 = 3;
pub const EXIT_FAILED: i32 = 4;

#[derive(Debug, Parser)]
#[command(
    name = "snapper-cli",
    version,
    about = "Capture, list, edit, OCR and record screenshots"
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
    /// Print machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Log at debug level.
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Capture the desktop, a monitor, a window, or a rectangle.
    Capture(CaptureArgs),
    /// List monitors or windows.
    List {
        /// `monitors` or `windows`.
        what: ListWhat,
    },
    /// Open an image in the Snapper editor.
    Edit {
        /// Path of the image to open.
        path: PathBuf,
    },
    /// Recognise text in an image file or a captured region.
    Ocr {
        /// An image path, or a region `x,y,width,height`.
        target: String,
    },
    /// Record a region to a GIF or MP4.
    Record(RecordArgs),
}

#[derive(Debug, clap::Args)]
pub struct CaptureArgs {
    #[command(subcommand)]
    pub target: TargetArg,
    /// Write the image to this path. Without it, the settings template is used.
    #[arg(long, global = true)]
    pub out: Option<PathBuf>,
    /// Also copy the image to the clipboard.
    #[arg(long, global = true)]
    pub clipboard: bool,
    /// Override the image format from settings.
    #[arg(long, global = true)]
    pub format: Option<FormatArg>,
    /// Seconds to wait before capturing.
    #[arg(long, global = true)]
    pub delay: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum TargetArg {
    /// Capture every monitor as one image.
    Full,
    /// Capture one monitor by id.
    Monitor { id: u32 },
    /// Capture one window by id.
    Window { id: u64 },
    /// Capture `x,y,width,height` in physical pixels.
    Region {
        /// `x,y,width,height`. `x` and `y` may be negative.
        #[arg(allow_hyphen_values = true)]
        spec: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ListWhat {
    Monitors,
    Windows,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FormatArg {
    Png,
    Jpeg,
    Webp,
}

impl From<FormatArg> for ImageFormat {
    fn from(format: FormatArg) -> Self {
        match format {
            FormatArg::Png => ImageFormat::Png,
            FormatArg::Jpeg => ImageFormat::Jpeg,
            FormatArg::Webp => ImageFormat::Webp,
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct RecordArgs {
    /// Region `x,y,width,height` in physical pixels.
    #[arg(long)]
    pub region: String,
    /// Seconds to record.
    #[arg(long)]
    pub duration: u64,
    /// Output path. `.gif` uses gifski, `.mp4` uses ffmpeg.
    #[arg(long)]
    pub out: PathBuf,
    /// Frames per second. Defaults to 15 for GIF and 30 for MP4.
    #[arg(long)]
    pub fps: Option<u32>,
}

/// Parse argv and run. `--help` exits from clap with status 0.
pub fn run_from_env() -> i32 {
    match Cli::try_parse() {
        Ok(cli) => execute(cli),
        Err(err) => err.exit(),
    }
}

pub fn execute(cli: Cli) -> i32 {
    let result = match cli.cmd {
        Cmd::Capture(args) => capture(args, cli.json),
        Cmd::List { what } => list(what, cli.json),
        Cmd::Edit { path } => edit(&path),
        Cmd::Ocr { target } => ocr(&target),
        Cmd::Record(args) => record(args, cli.json),
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(failure) => {
            eprintln!("{failure}");
            failure.code
        }
    }
}

struct Failure {
    code: i32,
    message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_USAGE,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_FAILED,
            message: message.into(),
        }
    }

    fn from_capture(err: CaptureError) -> Self {
        let code = exit_for_capture(&err);
        Self {
            code,
            message: err.to_string(),
        }
    }
}

/// Map a capture error onto the CLI exit codes.
pub fn exit_for_capture(err: &CaptureError) -> i32 {
    match err {
        CaptureError::PermissionDenied => EXIT_PERMISSION,
        _ => EXIT_FAILED,
    }
}

fn capture(args: CaptureArgs, json: bool) -> Result<(), Failure> {
    if let Some(delay) = args.delay {
        if delay > 0 {
            std::thread::sleep(Duration::from_secs(delay));
        }
    }
    let image = grab(&args.target)?;
    let settings = Settings::load().unwrap_or_else(|err| {
        eprintln!("settings failed, using defaults: {err}");
        Settings::default()
    });
    let mut output = settings.output.clone();
    if let Some(format) = args.format {
        output.format = format.into();
    }
    let path = match &args.out {
        Some(path) => path.clone(),
        None => templated_path(&image, &output)?,
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| {
                Failure::failed(format!("could not create {}: {err}", parent.display()))
            })?;
        }
    }
    write_image(&path, &image, output.format, output.jpeg_quality)
        .map_err(|err| Failure::failed(err.to_string()))?;
    if args.clipboard {
        copy_image(&image).map_err(Failure::failed)?;
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "path": path.display().to_string(),
                "width": image.width,
                "height": image.height,
            })
        );
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

fn grab(target: &TargetArg) -> Result<Image, Failure> {
    let capturer = default_capturer();
    let image = match target {
        TargetArg::Full => capturer.capture_all(),
        TargetArg::Monitor { id } => capturer.capture_monitor(*id),
        TargetArg::Window { id } => capturer.capture_window(*id),
        TargetArg::Region { spec } => {
            let rect = parse_region(spec).map_err(Failure::usage)?;
            let monitors = capturer.monitors().map_err(Failure::from_capture)?;
            let image = capturer.capture_all().map_err(Failure::from_capture)?;
            let (origin_x, origin_y) = desktop_origin(&monitors);
            return crop(
                &image,
                Rect {
                    x: rect.x.saturating_sub(origin_x),
                    y: rect.y.saturating_sub(origin_y),
                    width: rect.width,
                    height: rect.height,
                },
            )
            .map_err(Failure::from_capture);
        }
    };
    image.map_err(Failure::from_capture)
}

fn list(what: ListWhat, json: bool) -> Result<(), Failure> {
    let capturer = default_capturer();
    match what {
        ListWhat::Monitors => {
            let monitors = capturer.monitors().map_err(Failure::from_capture)?;
            if json {
                println!("{}", monitors_json(&monitors));
            } else if monitors.is_empty() {
                println!("no monitors");
            } else {
                for monitor in monitors {
                    println!(
                        "{}\t{}\t{},{}\t{}x{}\tscale {}\t{}",
                        monitor.id,
                        monitor.name,
                        monitor.rect.x,
                        monitor.rect.y,
                        monitor.rect.width,
                        monitor.rect.height,
                        monitor.scale_factor,
                        if monitor.is_primary { "primary" } else { "" }
                    );
                }
            }
        }
        ListWhat::Windows => {
            let windows = capturer.windows().map_err(Failure::from_capture)?;
            if json {
                println!("{}", windows_json(&windows));
            } else if windows.is_empty() {
                println!("no windows");
            } else {
                for window in windows {
                    println!(
                        "{}\t{}\t{}\t{}x{}",
                        window.id,
                        window.app_name,
                        window.title,
                        window.rect.width,
                        window.rect.height
                    );
                }
            }
        }
    }
    Ok(())
}

/// JSON array of monitors. Used by `list monitors --json` and by tests.
pub fn monitors_json(monitors: &[MonitorInfo]) -> String {
    let items: Vec<_> = monitors
        .iter()
        .map(|monitor| {
            serde_json::json!({
                "id": monitor.id,
                "name": monitor.name,
                "x": monitor.rect.x,
                "y": monitor.rect.y,
                "width": monitor.rect.width,
                "height": monitor.rect.height,
                "scale_factor": monitor.scale_factor,
                "primary": monitor.is_primary,
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_owned())
}

pub fn windows_json(windows: &[WindowInfo]) -> String {
    let items: Vec<_> = windows
        .iter()
        .map(|window| {
            serde_json::json!({
                "id": window.id,
                "title": window.title,
                "app_name": window.app_name,
                "x": window.rect.x,
                "y": window.rect.y,
                "width": window.rect.width,
                "height": window.rect.height,
                "minimised": window.is_minimised,
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_owned())
}

fn edit(path: &Path) -> Result<(), Failure> {
    if !path.is_file() {
        return Err(Failure::usage(format!(
            "image not found: {}",
            path.display()
        )));
    }
    let program = editor_program();
    let status = Command::new(&program)
        .arg("--open-editor")
        .arg(path)
        .status()
        .map_err(|err| {
            Failure::failed(format!(
                "could not launch {} --open-editor: {err}",
                program.display()
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::failed(format!("snapper-app exited with {status}")))
    }
}

/// `snapper-app` next to this executable, or `snapper-app` on `PATH`.
pub fn editor_program() -> PathBuf {
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            let sibling = dir.join(format!("snapper-app{}", std::env::consts::EXE_SUFFIX));
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from(format!("snapper-app{}", std::env::consts::EXE_SUFFIX))
}

fn ocr(target: &str) -> Result<(), Failure> {
    let image = if Path::new(target).is_file() {
        load_image(Path::new(target)).map_err(|err| Failure::failed(err.to_string()))?
    } else if target.contains(',') {
        grab(&TargetArg::Region {
            spec: target.to_owned(),
        })?
    } else {
        return Err(Failure::usage(format!("image not found: {target}")));
    };
    let engine = default_engine().map_err(|err| Failure::failed(err.to_string()))?;
    let words = engine
        .recognise(&image, "en")
        .map_err(|err| Failure::failed(err.to_string()))?;
    println!("{}", join_lines(&words));
    Ok(())
}

fn record(args: RecordArgs, json: bool) -> Result<(), Failure> {
    let rect = parse_region(&args.region).map_err(Failure::usage)?;
    let mut recorder = Recorder::new(rect, args.out.clone());
    if let Some(fps) = args.fps {
        recorder.fps = fps;
    }
    let handle = recorder.start().map_err(|err| {
        let message = err.to_string();
        if message.to_ascii_lowercase().contains("permission") {
            Failure {
                code: EXIT_PERMISSION,
                message,
            }
        } else {
            Failure::failed(message)
        }
    })?;
    if args.duration > 0 {
        std::thread::sleep(Duration::from_secs(args.duration));
    }
    let path = handle
        .stop()
        .map_err(|err| Failure::failed(err.to_string()))?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "path": path.display().to_string() })
        );
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

pub fn parse_region(spec: &str) -> Result<Rect, String> {
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 4 {
        return Err("region must be x,y,width,height".to_owned());
    }
    let x = parts[0]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("region x is not an integer: {}", parts[0].trim()))?;
    let y = parts[1]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("region y is not an integer: {}", parts[1].trim()))?;
    let width = parts[2]
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("region width is not an integer: {}", parts[2].trim()))?;
    let height = parts[3]
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("region height is not an integer: {}", parts[3].trim()))?;
    if width == 0 || height == 0 {
        return Err("region width and height must be greater than zero".to_owned());
    }
    Ok(Rect {
        x,
        y,
        width,
        height,
    })
}

fn desktop_origin(monitors: &[MonitorInfo]) -> (i32, i32) {
    (
        monitors
            .iter()
            .map(|monitor| monitor.rect.x)
            .min()
            .unwrap_or(0),
        monitors
            .iter()
            .map(|monitor| monitor.rect.y)
            .min()
            .unwrap_or(0),
    )
}

fn templated_path(
    image: &Image,
    output: &snapper_core::settings::OutputSettings,
) -> Result<PathBuf, Failure> {
    std::fs::create_dir_all(&output.directory).map_err(|err| {
        Failure::failed(format!(
            "could not create {}: {err}",
            output.directory.display()
        ))
    })?;
    let ctx = TemplateContext {
        now: chrono::Local::now(),
        app: None,
        title: None,
        width: image.width,
        height: image.height,
        counter: 1,
    };
    let rendered = template::render(&output.filename_template, &ctx);
    let stem = template::sanitise(&rendered);
    let stem = if stem.is_empty() {
        "Snapper".to_owned()
    } else {
        stem
    };
    let ext = match output.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
    };
    Ok(template::unique_path(&output.directory, &stem, ext))
}

fn write_image(path: &Path, image: &Image, format: ImageFormat, quality: u8) -> anyhow::Result<()> {
    let file = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    match format {
        ImageFormat::Png => PngEncoder::new(file)
            .write_image(
                &image.rgba,
                image.width,
                image.height,
                ExtendedColorType::Rgba8,
            )
            .context("encode png")?,
        ImageFormat::Jpeg => {
            let rgb = rgba_to_rgb(&image.rgba);
            JpegEncoder::new_with_quality(file, quality.clamp(1, 100))
                .write_image(&rgb, image.width, image.height, ExtendedColorType::Rgb8)
                .context("encode jpeg")?;
        }
        ImageFormat::Webp => WebPEncoder::new_lossless(file)
            .write_image(
                &image.rgba,
                image.width,
                image.height,
                ExtendedColorType::Rgba8,
            )
            .context("encode webp")?,
    }
    Ok(())
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    rgb
}

fn copy_image(image: &Image) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|err| err.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: std::borrow::Cow::Borrowed(&image.rgba),
        })
        .map_err(|err| err.to_string())
}

fn load_image(path: &Path) -> anyhow::Result<Image> {
    let image = image::open(path).with_context(|| anyhow!("open {}", path.display()))?;
    let rgba = image.to_rgba8();
    Ok(Image {
        width: rgba.width(),
        height: rgba.height(),
        scale_factor: 1.0,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_shape_parses_capture_list_and_edit() {
        let cli = Cli::try_parse_from([
            "snapper-cli",
            "--json",
            "capture",
            "full",
            "--out",
            "x.png",
            "--format",
            "png",
            "--delay",
            "1",
            "--clipboard",
        ])
        .expect("parse");
        assert!(cli.json);
        let Cmd::Capture(args) = cli.cmd else {
            panic!("capture");
        };
        assert!(matches!(args.target, TargetArg::Full));
        assert_eq!(args.out.unwrap(), PathBuf::from("x.png"));
        assert!(args.clipboard);
        assert_eq!(args.delay, Some(1));
    }

    #[test]
    fn region_and_list_arguments_parse() {
        let cli = Cli::try_parse_from(["snapper-cli", "capture", "region", "-10,20,30,40"])
            .expect("region");
        let Cmd::Capture(args) = cli.cmd else {
            panic!("capture");
        };
        let TargetArg::Region { spec } = args.target else {
            panic!("region");
        };
        assert_eq!(
            parse_region(&spec).expect("rect"),
            Rect {
                x: -10,
                y: 20,
                width: 30,
                height: 40,
            }
        );

        let cli = Cli::try_parse_from(["snapper-cli", "list", "monitors", "--json"]).expect("list");
        assert!(cli.json);
        assert!(matches!(
            cli.cmd,
            Cmd::List {
                what: ListWhat::Monitors
            }
        ));
        assert!(Cli::try_parse_from(["snapper-cli", "capture"]).is_err());
        assert!(parse_region("1,2,3").is_err());
        assert!(parse_region("1,2,0,4").is_err());
    }

    #[test]
    fn permission_denied_is_exit_3_and_other_failures_are_exit_4() {
        assert_eq!(
            exit_for_capture(&CaptureError::PermissionDenied),
            EXIT_PERMISSION
        );
        assert_eq!(
            exit_for_capture(&CaptureError::Backend("no".into())),
            EXIT_FAILED
        );
        assert_eq!(
            exit_for_capture(&CaptureError::Unsupported("nope")),
            EXIT_FAILED
        );
    }

    #[test]
    fn monitor_json_is_valid() {
        let monitors = vec![MonitorInfo {
            id: 1,
            name: "Built-in".to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            scale_factor: 2.0,
            is_primary: true,
        }];
        let text = monitors_json(&monitors);
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(parsed[0]["name"], "Built-in");
        assert_eq!(parsed[0]["primary"], true);
        assert_eq!(parsed[0]["width"], 800);
    }
}
