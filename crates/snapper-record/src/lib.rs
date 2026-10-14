//! Screen recording.
//!
//! GIF frames are encoded in-process with gifski. MP4 frames are piped to a
//! system `ffmpeg` as raw RGBA. Audio is not captured.
//!
//! The frame queue holds at most [`FRAME_BOUND`] frames. A full queue drops
//! the new frame and counts it, so a long recording does not grow without a
//! limit. macOS ScreenCaptureKit (`scap`) is not linked: each frame comes from
//! [`snapper_capture::Capturer`], cropped to the region. Wayland ScreenCast is
//! not implemented; [`Recorder::start`] returns an error on the portal backend.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use snapper_capture::{crop, default_capturer};
use snapper_core::{Image, Rect};

/// Frames kept in memory waiting for the encoder. Further frames are dropped.
pub const FRAME_BOUND: usize = 60;

/// Default GIF frame rate.
pub const GIF_FPS: u32 = 15;
/// Default MP4 frame rate.
pub const MP4_FPS: u32 = 30;

/// What the recording is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFormat {
    Gif,
    Mp4,
}

impl RecordFormat {
    pub fn default_fps(self) -> u32 {
        match self {
            Self::Gif => GIF_FPS,
            Self::Mp4 => MP4_FPS,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("mp4") => Self::Mp4,
            _ => Self::Gif,
        }
    }
}

/// A recording that has not started.
#[derive(Debug, Clone)]
pub struct Recorder {
    pub region: Rect,
    pub fps: u32,
    pub format: RecordFormat,
    pub out: PathBuf,
}

/// A running recording.
pub struct RecordingHandle {
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    capture: Option<JoinHandle<()>>,
    encode: Option<JoinHandle<Result<(), RecordError>>>,
    out: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("{0}")]
    Unsupported(String),
    #[error("ffmpeg was not found. Install ffmpeg and try again, or record a GIF instead.")]
    MissingFfmpeg,
    #[error("recording failed: {0}")]
    Backend(String),
    #[error("the recording had no frames")]
    Empty,
}

impl Recorder {
    pub fn new(region: Rect, out: PathBuf) -> Self {
        let format = RecordFormat::from_path(&out);
        Self {
            region,
            fps: format.default_fps(),
            format,
            out,
        }
    }

    /// Capture `region` until [`RecordingHandle::stop`].
    ///
    /// On Wayland this returns an error. The portal ScreenCast session is not
    /// implemented, and a screenshot per frame would prompt on every frame.
    pub fn start(self) -> Result<RecordingHandle, RecordError> {
        let capturer = default_capturer();
        if capturer.name() == "wayland" {
            return Err(RecordError::Unsupported(
                "screen recording on Wayland is not available yet. The XDG ScreenCast portal is a later step, and synthetic capture would ask for permission on every frame.".to_owned(),
            ));
        }
        let region = self.region;
        self.start_with(move || capture_region(capturer.as_ref(), region))
    }

    /// Record frames from `capture` instead of the screen.
    pub fn start_with<F>(self, mut capture: F) -> Result<RecordingHandle, RecordError>
    where
        F: FnMut() -> Result<Image, RecordError> + Send + 'static,
    {
        if self.fps == 0 {
            return Err(RecordError::Backend("fps must be at least 1".to_owned()));
        }
        if self.region.width == 0 || self.region.height == 0 {
            return Err(RecordError::Backend("the region is empty".to_owned()));
        }
        if let Some(parent) = self.out.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| RecordError::Backend(err.to_string()))?;
            }
        }

        let (tx, rx) = crossbeam_channel::bounded::<Image>(FRAME_BOUND);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let stop_capture = Arc::clone(&stop);
        let dropped_capture = Arc::clone(&dropped);
        let fps = self.fps;
        let capture_thread = std::thread::Builder::new()
            .name("snapper-record-capture".to_owned())
            .spawn(move || {
                let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
                while !stop_capture.load(Ordering::Relaxed) {
                    let started = std::time::Instant::now();
                    match capture() {
                        Ok(frame) => {
                            offer_frame(&tx, frame, &dropped_capture);
                        }
                        Err(_) => break,
                    }
                    let elapsed = started.elapsed();
                    if elapsed < interval {
                        std::thread::sleep(interval - elapsed);
                    }
                }
            })
            .map_err(|err| RecordError::Backend(err.to_string()))?;

        let format = self.format;
        let out = self.out.clone();
        let encode_fps = self.fps;
        let encode_thread = std::thread::Builder::new()
            .name("snapper-record-encode".to_owned())
            .spawn(move || encode_stream(rx, format, encode_fps, &out))
            .map_err(|err| RecordError::Backend(err.to_string()))?;

        Ok(RecordingHandle {
            stop,
            dropped,
            capture: Some(capture_thread),
            encode: Some(encode_thread),
            out: self.out,
        })
    }
}

impl RecordingHandle {
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Stop capturing and finish the file.
    pub fn stop(mut self) -> Result<PathBuf, RecordError> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(capture) = self.capture.take() {
            let _ = capture.join();
        }
        self.encode
            .take()
            .map(|handle| handle.join())
            .transpose()
            .map_err(|_| RecordError::Backend("the encoder thread panicked".to_owned()))?
            .unwrap_or(Ok(()))?;
        if !self.out.exists() {
            return Err(RecordError::Empty);
        }
        Ok(self.out)
    }
}

fn capture_region(
    capturer: &dyn snapper_capture::Capturer,
    region: Rect,
) -> Result<Image, RecordError> {
    let monitors = capturer
        .monitors()
        .map_err(|err| RecordError::Backend(err.to_string()))?;
    let image = capturer
        .capture_all()
        .map_err(|err| RecordError::Backend(err.to_string()))?;
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
    let local = Rect {
        x: region.x.saturating_sub(origin_x),
        y: region.y.saturating_sub(origin_y),
        width: region.width,
        height: region.height,
    };
    crop(&image, local).map_err(|err| RecordError::Backend(err.to_string()))
}

/// Push `frame` unless the queue is full or disconnected.
///
/// A full queue increments `dropped` and discards `frame`.
pub fn offer_frame(
    tx: &crossbeam_channel::Sender<Image>,
    frame: Image,
    dropped: &AtomicU64,
) -> bool {
    match tx.try_send(frame) {
        Ok(()) => true,
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => false,
    }
}

fn encode_stream(
    rx: crossbeam_channel::Receiver<Image>,
    format: RecordFormat,
    fps: u32,
    out: &Path,
) -> Result<(), RecordError> {
    match format {
        RecordFormat::Gif => encode_gif_stream(rx, fps, out),
        RecordFormat::Mp4 => {
            let ffmpeg = which::which("ffmpeg").map_err(|_| RecordError::MissingFfmpeg)?;
            encode_mp4_stream(rx, fps, out, &ffmpeg)
        }
    }
}

fn encode_gif_stream(
    rx: crossbeam_channel::Receiver<Image>,
    fps: u32,
    out: &Path,
) -> Result<(), RecordError> {
    let Ok(first) = rx.recv() else {
        return Err(RecordError::Empty);
    };
    let width = first.width;
    let height = first.height;
    let (collector, writer) = gifski::new(gifski::Settings {
        width: Some(width),
        height: Some(height),
        quality: 90,
        fast: true,
        repeat: gifski::Repeat::Infinite,
    })
    .map_err(|err| RecordError::Backend(err.to_string()))?;
    let step = 1.0 / f64::from(fps.max(1));
    let mut index = 0usize;
    add_gif_frame(&collector, index, &first, step)?;
    while let Ok(frame) = rx.recv() {
        index += 1;
        add_gif_frame(&collector, index, &frame, step)?;
    }
    drop(collector);
    let file = std::fs::File::create(out).map_err(|err| RecordError::Backend(err.to_string()))?;
    writer
        .write(file, &mut KeepGoing)
        .map_err(|err| RecordError::Backend(err.to_string()))?;
    Ok(())
}

fn add_gif_frame(
    collector: &gifski::Collector,
    index: usize,
    frame: &Image,
    step: f64,
) -> Result<(), RecordError> {
    let pixels = rgba_pixels(&frame.rgba);
    let image = imgref::Img::new(pixels, frame.width as usize, frame.height as usize);
    collector
        .add_frame_rgba(index, image, index as f64 * step)
        .map_err(|err| RecordError::Backend(err.to_string()))
}

fn encode_mp4_stream(
    rx: crossbeam_channel::Receiver<Image>,
    fps: u32,
    out: &Path,
    ffmpeg_path: &Path,
) -> Result<(), RecordError> {
    let Ok(first) = rx.recv() else {
        return Err(RecordError::Empty);
    };
    let (width, height, _) = pad_even(&first);
    let mut child = std::process::Command::new(ffmpeg_path)
        .args(ffmpeg_args(width, height, fps, out))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                RecordError::MissingFfmpeg
            } else {
                RecordError::Backend(err.to_string())
            }
        })?;
    let write_frame = |frame: &Image, stdin: &mut std::process::ChildStdin| {
        let owned;
        let raw = if frame.width == width && frame.height == height {
            frame.rgba.as_slice()
        } else {
            owned = pad_even(frame).2;
            owned.as_slice()
        };
        std::io::Write::write_all(stdin, raw).map_err(|err| RecordError::Backend(err.to_string()))
    };
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| RecordError::Backend("ffmpeg stdin was closed".to_owned()))?;
        write_frame(&first, stdin)?;
        while let Ok(frame) = rx.recv() {
            write_frame(&frame, stdin)?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| RecordError::Backend(err.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RecordError::Backend(format!(
            "ffmpeg exited with {}: {stderr}",
            output.status
        )));
    }
    Ok(())
}

/// Encode `frames` as a GIF with gifski quality 90.
pub fn encode_gif(frames: &[Image], fps: u32, out: &Path) -> Result<(), RecordError> {
    if frames.is_empty() {
        return Err(RecordError::Empty);
    }
    let width = frames[0].width;
    let height = frames[0].height;
    let (collector, writer) = gifski::new(gifski::Settings {
        width: Some(width),
        height: Some(height),
        quality: 90,
        fast: true,
        repeat: gifski::Repeat::Infinite,
    })
    .map_err(|err| RecordError::Backend(err.to_string()))?;

    let step = 1.0 / f64::from(fps.max(1));
    for (index, frame) in frames.iter().enumerate() {
        if frame.width != width || frame.height != height {
            return Err(RecordError::Backend(
                "frames in one recording must share a size".to_owned(),
            ));
        }
        let pixels = rgba_pixels(&frame.rgba);
        let image = imgref::Img::new(pixels, width as usize, height as usize);
        collector
            .add_frame_rgba(index, image, index as f64 * step)
            .map_err(|err| RecordError::Backend(err.to_string()))?;
    }
    drop(collector);
    let file = std::fs::File::create(out).map_err(|err| RecordError::Backend(err.to_string()))?;
    writer
        .write(file, &mut KeepGoing)
        .map_err(|err| RecordError::Backend(err.to_string()))?;
    Ok(())
}

struct KeepGoing;

impl gifski::progress::ProgressReporter for KeepGoing {
    fn increase(&mut self) -> bool {
        true
    }
}

fn rgba_pixels(bytes: &[u8]) -> Vec<rgb::RGBA8> {
    bytes
        .chunks_exact(4)
        .map(|pixel| rgb::RGBA8 {
            r: pixel[0],
            g: pixel[1],
            b: pixel[2],
            a: pixel[3],
        })
        .collect()
}

/// Arguments for `ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i - -c:v libx264 -pix_fmt yuv420p -crf 23`.
///
/// `-y` is added so a tray app does not wait on an overwrite prompt.
pub fn ffmpeg_args(width: u32, height: u32, fps: u32, out: &Path) -> Vec<String> {
    vec![
        "-y".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-s".to_owned(),
        format!("{width}x{height}"),
        "-r".to_owned(),
        fps.to_string(),
        "-i".to_owned(),
        "-".to_owned(),
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-crf".to_owned(),
        "23".to_owned(),
        out.display().to_string(),
    ]
}

/// Encode MP4 by piping raw frames to `ffmpeg` at `ffmpeg_path`.
pub fn encode_mp4(
    frames: &[Image],
    fps: u32,
    out: &Path,
    ffmpeg_path: &Path,
) -> Result<(), RecordError> {
    if frames.is_empty() {
        return Err(RecordError::Empty);
    }
    if !ffmpeg_path.exists() && which::which(ffmpeg_path).is_err() {
        return Err(RecordError::MissingFfmpeg);
    }
    let (width, height, _) = pad_even(&frames[0]);
    let mut child = std::process::Command::new(ffmpeg_path)
        .args(ffmpeg_args(width, height, fps, out))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                RecordError::MissingFfmpeg
            } else {
                RecordError::Backend(err.to_string())
            }
        })?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| RecordError::Backend("ffmpeg stdin was closed".to_owned()))?;
        for frame in frames {
            let owned;
            let raw = if frame.width == width && frame.height == height {
                frame.rgba.as_slice()
            } else {
                owned = pad_even(frame).2;
                owned.as_slice()
            };
            std::io::Write::write_all(stdin, raw)
                .map_err(|err| RecordError::Backend(err.to_string()))?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| RecordError::Backend(err.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RecordError::Backend(format!(
            "ffmpeg exited with {}: {stderr}",
            output.status
        )));
    }
    Ok(())
}

/// yuv420p wants even dimensions. Extra rows and columns are transparent black.
fn pad_even(image: &Image) -> (u32, u32, Vec<u8>) {
    let width = image.width + (image.width % 2);
    let height = image.height + (image.height % 2);
    if width == image.width && height == image.height {
        return (width, height, image.rgba.clone());
    }
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..image.height {
        let src = (y * image.width * 4) as usize;
        let dst = (y * width * 4) as usize;
        let len = (image.width * 4) as usize;
        rgba[dst..dst + len].copy_from_slice(&image.rgba[src..src + len]);
    }
    (width, height, rgba)
}

/// Draw a cursor dot, and a ring when `click` is set.
///
/// Coordinates are pixels inside `image`. A point outside the image is ignored.
pub fn stamp_cursor(image: &mut Image, x: i32, y: i32, click: bool) {
    if x < 0 || y < 0 || x >= image.width as i32 || y >= image.height as i32 {
        return;
    }
    fill_circle(image, x, y, 5, [255, 40, 40, 220]);
    if click {
        ring(image, x, y, 12, [255, 200, 40, 230]);
    }
}

fn fill_circle(image: &mut Image, cx: i32, cy: i32, radius: i32, colour: [u8; 4]) {
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= radius * radius {
                put(image, x, y, colour);
            }
        }
    }
}

fn ring(image: &mut Image, cx: i32, cy: i32, radius: i32, colour: [u8; 4]) {
    let inner = radius - 2;
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            let distance = dx * dx + dy * dy;
            if distance <= radius * radius && distance >= inner * inner {
                put(image, x, y, colour);
            }
        }
    }
}

fn put(image: &mut Image, x: i32, y: i32, colour: [u8; 4]) {
    if x < 0 || y < 0 || x >= image.width as i32 || y >= image.height as i32 {
        return;
    }
    let index = ((y as u32 * image.width + x as u32) * 4) as usize;
    image.rgba[index..index + 4].copy_from_slice(&colour);
}

/// Upper bound on the queue for a 1920×1080 recording, in bytes.
pub fn queue_bytes(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) * 4 * FRAME_BOUND as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        Image {
            width,
            height,
            scale_factor: 1.0,
            rgba,
        }
    }

    #[test]
    fn a_full_queue_drops_frames() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let dropped = AtomicU64::new(0);
        assert!(offer_frame(&tx, frame(2, 2, [1, 0, 0, 255]), &dropped));
        assert!(!offer_frame(&tx, frame(2, 2, [2, 0, 0, 255]), &dropped));
        assert!(!offer_frame(&tx, frame(2, 2, [3, 0, 0, 255]), &dropped));
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn the_queue_stays_under_half_a_gigabyte_at_1080p() {
        let bytes = queue_bytes(1920, 1080);
        assert!(bytes < 500 * 1024 * 1024, "{bytes} bytes");
        assert!(queue_bytes(800, 600) < 500 * 1024 * 1024);
    }

    #[test]
    fn ffmpeg_args_match_the_encoder_command() {
        let args = ffmpeg_args(640, 480, 30, Path::new("out.mp4"));
        let text = args.join(" ");
        assert!(text.contains("-f rawvideo"));
        assert!(text.contains("-pix_fmt rgba"));
        assert!(text.contains("-s 640x480"));
        assert!(text.contains("-r 30"));
        assert!(text.contains("-i -"));
        assert!(text.contains("-c:v libx264"));
        assert!(text.contains("-pix_fmt yuv420p"));
        assert!(text.contains("-crf 23"));
        assert!(text.ends_with("out.mp4"));
    }

    #[test]
    fn a_missing_ffmpeg_binary_is_reported() {
        let err = encode_mp4(
            &[frame(2, 2, [0, 0, 0, 255])],
            30,
            Path::new("out.mp4"),
            Path::new("snapper-ffmpeg-missing"),
        )
        .expect_err("missing");
        assert!(matches!(err, RecordError::MissingFfmpeg), "{err}");
        assert!(err.to_string().contains("ffmpeg"));
    }

    #[test]
    fn gif_round_trip_writes_a_file() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("tiny.gif");
        let frames = [
            frame(8, 8, [200, 10, 10, 255]),
            frame(8, 8, [10, 10, 200, 255]),
            frame(8, 8, [10, 200, 10, 255]),
        ];
        encode_gif(&frames, 15, &path).expect("gif");
        let meta = std::fs::metadata(&path).expect("meta");
        assert!(meta.len() > 16, "gif was {}", meta.len());
        let header = std::fs::read(&path).expect("read");
        assert!(header.starts_with(b"GIF"), "header {:?}", &header[..6]);
    }

    #[test]
    fn mp4_round_trip_when_ffmpeg_is_installed() {
        let Ok(ffmpeg) = which::which("ffmpeg") else {
            eprintln!("ffmpeg is not installed; the missing-ffmpeg test covers that path");
            return;
        };
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("tiny.mp4");
        let frames = [
            frame(16, 16, [255, 0, 0, 255]),
            frame(16, 16, [0, 255, 0, 255]),
        ];
        encode_mp4(&frames, 30, &path, &ffmpeg).expect("mp4");
        let bytes = std::fs::read(&path).expect("read");
        assert!(bytes.len() > 32, "{}", bytes.len());
        assert!(
            bytes.windows(4).any(|chunk| chunk == b"ftyp"),
            "mp4 was missing an ftyp box"
        );
    }

    #[test]
    fn cursor_stamp_paints_inside_the_image_only() {
        let mut image = frame(20, 20, [0, 0, 0, 255]);
        stamp_cursor(&mut image, 10, 10, true);
        let centre = ((10 * 20 + 10) * 4) as usize;
        assert_eq!(&image.rgba[centre..centre + 3], &[255, 40, 40]);
        stamp_cursor(&mut image, -4, -4, true);
        stamp_cursor(&mut image, 100, 100, false);
        assert_eq!(image.rgba.len(), 20 * 20 * 4);
    }

    #[test]
    fn odd_frames_are_padded_to_even_dimensions() {
        let (width, height, rgba) = pad_even(&frame(3, 3, [1, 2, 3, 255]));
        assert_eq!((width, height), (4, 4));
        assert_eq!(rgba.len(), 4 * 4 * 4);
    }

    #[test]
    fn format_follows_the_extension() {
        assert_eq!(
            RecordFormat::from_path(Path::new("a.gif")),
            RecordFormat::Gif
        );
        assert_eq!(
            RecordFormat::from_path(Path::new("a.mp4")),
            RecordFormat::Mp4
        );
        assert_eq!(RecordFormat::Gif.default_fps(), 15);
        assert_eq!(RecordFormat::Mp4.default_fps(), 30);
    }
}
