//! Clipboard, save, and notification steps.
//!
//! Upload targets are a later issue. The [`OutputStep::Upload`] variant is
//! recognised so a configured name can be queued, then reported as unavailable.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use snapper_core::settings::{ImageFormat, OutputSettings};
use snapper_core::template::{self, TemplateContext};
use snapper_core::{Image, Settings};

/// One stage of the output pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputStep {
    Clipboard,
    Save,
    Upload(String),
    Notify,
}

/// What the pipeline managed to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputResult {
    pub saved: Option<PathBuf>,
    pub url: Option<String>,
}

/// Ordered steps selected from [`OutputSettings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputPipeline {
    steps: Vec<OutputStep>,
}

impl OutputPipeline {
    /// Clipboard (when enabled), then save, then an optional upload, then notify.
    ///
    /// `upload` is the target name. Performing the upload is a later issue;
    /// until then the step reports that the target is not available.
    pub fn from_settings(settings: &OutputSettings, upload: Option<String>) -> Self {
        let mut steps = Vec::new();
        if settings.copy_to_clipboard {
            steps.push(OutputStep::Clipboard);
        }
        steps.push(OutputStep::Save);
        if let Some(target) = upload {
            steps.push(OutputStep::Upload(target));
        }
        steps.push(OutputStep::Notify);
        Self { steps }
    }

    pub fn steps(&self) -> &[OutputStep] {
        &self.steps
    }
}

/// Template values for a capture whose app and title are not known yet.
pub fn context_for(image: &Image) -> TemplateContext {
    TemplateContext {
        now: chrono::Local::now(),
        app: None,
        title: None,
        width: image.width,
        height: image.height,
        counter: 1,
    }
}

/// Copy, save, and notify, as `settings` configures. A failed step is logged
/// and reported, and the later steps still run.
pub fn run(image: &Image, ctx: &TemplateContext, settings: &Settings) -> OutputResult {
    let pipeline = OutputPipeline::from_settings(&settings.output, None);
    run_steps(pipeline.steps(), image, ctx, settings, &LiveEffects)
}

/// Copy `image` to the clipboard.
///
/// On Linux the clipboard is owned by the process, so a background thread
/// keeps it alive for 60 seconds or until a newer copy replaces it.
pub fn copy_image(image: &Image) -> Result<(), String> {
    image_ok(image)?;
    #[cfg(target_os = "linux")]
    {
        linux_clipboard(image)
    }
    #[cfg(not(target_os = "linux"))]
    {
        set_clipboard_image(
            image.width as usize,
            image.height as usize,
            Cow::Borrowed(&image.rgba),
        )
    }
}

trait Effects {
    fn copy(&self, image: &Image) -> Result<(), String>;
    fn notify_saved(&self, path: &Path) -> Result<(), String>;
    fn notify_error(&self, message: &str);
}

struct LiveEffects;

impl Effects for LiveEffects {
    fn copy(&self, image: &Image) -> Result<(), String> {
        copy_image(image)
    }

    fn notify_saved(&self, path: &Path) -> Result<(), String> {
        let body = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("screenshot");
        let folder = path.parent().unwrap_or(path).to_path_buf();
        spawn_notification("Screenshot saved", body, Some(folder))
    }

    fn notify_error(&self, message: &str) {
        if let Err(err) = spawn_notification("Snapper", message, None) {
            tracing::warn!(error = %err, "could not show an output error");
        }
    }
}

struct Job<'a> {
    image: &'a Image,
    ctx: &'a TemplateContext,
    settings: &'a Settings,
    effects: &'a dyn Effects,
}

fn run_steps(
    steps: &[OutputStep],
    image: &Image,
    ctx: &TemplateContext,
    settings: &Settings,
    effects: &dyn Effects,
) -> OutputResult {
    let job = Job {
        image,
        ctx,
        settings,
        effects,
    };
    let mut result = OutputResult {
        saved: None,
        url: None,
    };
    for step in steps {
        if let Err(err) = apply_step(step, &job, &mut result) {
            tracing::error!(step = ?step, error = %err, "output step failed");
            effects.notify_error(&err);
        }
    }
    result
}

fn apply_step(step: &OutputStep, job: &Job<'_>, result: &mut OutputResult) -> Result<(), String> {
    match step {
        OutputStep::Clipboard => job.effects.copy(job.image),
        OutputStep::Save => {
            let path = save_image(job.image, job.ctx, &job.settings.output)?;
            result.saved = Some(path);
            Ok(())
        }
        OutputStep::Upload(target) => Err(format!("upload target {target} is not available yet")),
        OutputStep::Notify => {
            if let Some(path) = &result.saved {
                job.effects.notify_saved(path)?;
            }
            Ok(())
        }
    }
}

/// Render the filename template, sanitise it, and write a unique file.
pub fn save_image(
    image: &Image,
    ctx: &TemplateContext,
    settings: &OutputSettings,
) -> Result<PathBuf, String> {
    image_ok(image)?;
    std::fs::create_dir_all(&settings.directory)
        .map_err(|err| format!("could not create {}: {err}", settings.directory.display()))?;
    let stem = file_stem(&settings.filename_template, ctx);
    let path = template::unique_path(&settings.directory, &stem, extension(settings.format));
    if let Err(err) = encode_file(&path, image, settings) {
        let _ = std::fs::remove_file(&path);
        return Err(err);
    }
    Ok(path)
}

fn file_stem(template: &str, ctx: &TemplateContext) -> String {
    let rendered = template::render(template, ctx);
    let cleaned = template::sanitise(&rendered);
    if cleaned.is_empty() {
        "Snapper".to_owned()
    } else {
        cleaned
    }
}

fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
    }
}

fn encode_file(path: &Path, image: &Image, settings: &OutputSettings) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|err| err.to_string())?;
    match settings.format {
        ImageFormat::Png => {
            let encoder =
                PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Adaptive);
            encoder
                .write_image(
                    &image.rgba,
                    image.width,
                    image.height,
                    ExtendedColorType::Rgba8,
                )
                .map_err(|err| err.to_string())
        }
        ImageFormat::Jpeg => {
            let rgb = rgba_to_rgb(&image.rgba);
            let encoder = JpegEncoder::new_with_quality(file, jpeg_quality(settings.jpeg_quality));
            encoder
                .write_image(&rgb, image.width, image.height, ExtendedColorType::Rgb8)
                .map_err(|err| err.to_string())
        }
        ImageFormat::Webp => {
            let encoder = WebPEncoder::new_lossless(file);
            encoder
                .write_image(
                    &image.rgba,
                    image.width,
                    image.height,
                    ExtendedColorType::Rgba8,
                )
                .map_err(|err| err.to_string())
        }
    }
}

fn jpeg_quality(value: u8) -> u8 {
    value.clamp(1, 100)
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.as_chunks::<4>().0 {
        rgb.extend_from_slice(&pixel[..3]);
    }
    rgb
}

fn image_ok(image: &Image) -> Result<(), String> {
    let expected = (image.width as usize)
        .saturating_mul(image.height as usize)
        .saturating_mul(4);
    if image.width == 0 || image.height == 0 || image.rgba.len() != expected {
        Err("image has no pixels".to_owned())
    } else {
        Ok(())
    }
}

fn set_clipboard_image(width: usize, height: usize, bytes: Cow<'_, [u8]>) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|err| err.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width,
            height,
            bytes,
        })
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "linux")]
fn linux_clipboard(image: &Image) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static GENERATION: AtomicU64 = AtomicU64::new(0);
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst).saturating_add(1);
    let width = image.width as usize;
    let height = image.height as usize;
    let bytes = image.rgba.clone();
    std::thread::spawn(move || {
        if let Err(err) = hold_linux_clipboard(width, height, bytes, generation, &GENERATION) {
            tracing::error!(error = %err, "linux clipboard owner failed");
        }
    });
    Ok(())
}

#[cfg(target_os = "linux")]
fn hold_linux_clipboard(
    width: usize,
    height: usize,
    bytes: Vec<u8>,
    generation: u64,
    current: &std::sync::atomic::AtomicU64,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    set_clipboard_image(width, height, Cow::Owned(bytes))?;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(60)
        && current.load(Ordering::SeqCst) == generation
    {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(())
}

fn spawn_notification(title: &str, body: &str, open_folder: Option<PathBuf>) -> Result<(), String> {
    let title = title.to_owned();
    let body = body.to_owned();
    std::thread::Builder::new()
        .name("snapper-notify".to_owned())
        .spawn(move || show_notification(&title, &body, open_folder))
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn show_notification(title: &str, body: &str, open_folder: Option<PathBuf>) {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    if open_folder.is_some() {
        notification.action("default", "Open folder");
    }
    match notification.show() {
        Ok(handle) => {
            let Some(folder) = open_folder else {
                return;
            };
            handle.wait_for_action(|action| {
                if action == "default" {
                    if let Err(err) = opener::open(&folder) {
                        tracing::warn!(error = %err, "could not open the screenshot folder");
                    }
                }
            });
        }
        Err(err) => tracing::warn!(error = %err, "notification failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Record {
        copied: Mutex<bool>,
        notes: Mutex<Vec<PathBuf>>,
        errors: Mutex<Vec<String>>,
    }

    impl Record {
        fn new() -> Self {
            Self {
                copied: Mutex::new(false),
                notes: Mutex::new(Vec::new()),
                errors: Mutex::new(Vec::new()),
            }
        }
    }

    impl Effects for Record {
        fn copy(&self, image: &Image) -> Result<(), String> {
            image_ok(image)?;
            *self.copied.lock().expect("copied") = true;
            Ok(())
        }

        fn notify_saved(&self, path: &Path) -> Result<(), String> {
            self.notes.lock().expect("notes").push(path.to_path_buf());
            Ok(())
        }

        fn notify_error(&self, message: &str) {
            self.errors.lock().expect("errors").push(message.to_owned());
        }
    }

    fn pixel() -> Image {
        Image {
            width: 2,
            height: 1,
            scale_factor: 1.0,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        }
    }

    fn fixed_ctx(image: &Image) -> TemplateContext {
        TemplateContext {
            now: chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 1, 2, 3, 4, 5).unwrap(),
            app: Some("Demo".to_owned()),
            title: Some("Title".to_owned()),
            width: image.width,
            height: image.height,
            counter: 7,
        }
    }

    fn settings(dir: PathBuf, format: ImageFormat, copy: bool) -> Settings {
        Settings {
            output: OutputSettings {
                directory: dir,
                filename_template: "Shot".to_owned(),
                format,
                copy_to_clipboard: copy,
                jpeg_quality: 80,
                ..OutputSettings::default()
            },
            ..Settings::default()
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("snapper-output-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn step_list_follows_the_clipboard_setting() {
        assert_eq!(
            OutputPipeline::from_settings(&OutputSettings::default(), None).steps(),
            &[OutputStep::Clipboard, OutputStep::Save, OutputStep::Notify]
        );
        let output = OutputSettings {
            copy_to_clipboard: false,
            ..OutputSettings::default()
        };
        assert_eq!(
            OutputPipeline::from_settings(&output, None).steps(),
            &[OutputStep::Save, OutputStep::Notify]
        );
        assert_eq!(
            OutputPipeline::from_settings(&output, Some("s3".to_owned())).steps(),
            &[
                OutputStep::Save,
                OutputStep::Upload("s3".to_owned()),
                OutputStep::Notify
            ]
        );
    }

    #[test]
    fn save_uses_the_template_and_does_not_overwrite() {
        let dir = scratch("save");
        let image = pixel();
        let ctx = fixed_ctx(&image);
        let mut prefs = settings(dir.clone(), ImageFormat::Png, false);
        prefs.output.filename_template = "Shot_{width}x{height}".to_owned();
        let effects = Record::new();
        let first = run_steps(
            OutputPipeline::from_settings(&prefs.output, None).steps(),
            &image,
            &ctx,
            &prefs,
            &effects,
        );
        let saved = first.saved.expect("saved");
        assert_eq!(
            saved.file_name().and_then(|name| name.to_str()),
            Some("Shot_2x1.png")
        );
        let loaded = image::open(&saved).expect("png").to_rgba8();
        assert_eq!(loaded.into_raw(), image.rgba);

        let second = run_steps(
            OutputPipeline::from_settings(&prefs.output, None).steps(),
            &image,
            &ctx,
            &prefs,
            &effects,
        );
        let again = second.saved.expect("second");
        assert_eq!(
            again.file_name().and_then(|name| name.to_str()),
            Some("Shot_2x1 (1).png")
        );
        assert!(!*effects.copied.lock().expect("copied"));
        assert_eq!(effects.notes.lock().expect("notes").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jpeg_and_webp_files_keep_the_image_size() {
        let dir = scratch("formats");
        let image = pixel();
        let ctx = fixed_ctx(&image);
        for format in [ImageFormat::Jpeg, ImageFormat::Webp] {
            let prefs = settings(dir.clone(), format, false);
            let path = save_image(&image, &ctx, &prefs.output).expect("encode");
            let loaded = image::open(&path).expect("reload");
            assert_eq!((loaded.width(), loaded.height()), (2, 1));
            if format == ImageFormat::Webp {
                assert_eq!(loaded.to_rgba8().into_raw(), image.rgba);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_save_still_copies() {
        let dir = scratch("fail");
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, b"file").expect("blocker");
        let image = pixel();
        let ctx = fixed_ctx(&image);
        let prefs = settings(blocker, ImageFormat::Png, true);
        let effects = Record::new();
        let result = run_steps(
            OutputPipeline::from_settings(&prefs.output, None).steps(),
            &image,
            &ctx,
            &prefs,
            &effects,
        );
        assert!(result.saved.is_none());
        assert!(result.url.is_none());
        assert!(*effects.copied.lock().expect("copied"));
        assert!(!effects.errors.lock().expect("errors").is_empty());
        assert!(effects.notes.lock().expect("notes").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_upload_failure_still_notifies() {
        let dir = scratch("upload");
        let image = pixel();
        let ctx = fixed_ctx(&image);
        let prefs = settings(dir.clone(), ImageFormat::Png, false);
        let steps = OutputPipeline::from_settings(&prefs.output, Some("s3".to_owned()))
            .steps()
            .to_vec();
        let effects = Record::new();
        let result = run_steps(&steps, &image, &ctx, &prefs, &effects);
        assert!(result.saved.is_some());
        assert_eq!(result.url, None);
        let errors = effects.errors.lock().expect("errors");
        assert!(errors.iter().any(|err| err.contains("s3")));
        assert_eq!(effects.notes.lock().expect("notes").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reserved_windows_names_are_prefixed() {
        let image = pixel();
        let mut ctx = fixed_ctx(&image);
        ctx.app = None;
        ctx.title = None;
        assert_eq!(file_stem("CON", &ctx), "_CON");
        assert_eq!(file_stem("   ", &ctx), "Snapper");
    }
}
