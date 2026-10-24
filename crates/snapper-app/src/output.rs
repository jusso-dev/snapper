//! Clipboard, save, upload, redaction, and notification steps.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use snapper_core::settings::{ImageFormat, LinkFormat, OutputSettings};
use snapper_core::template::{self, TemplateContext};
use snapper_core::{Image, Settings};

/// One stage of the output pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputStep {
    /// OCR the image and pixelate detected secrets before later steps.
    Redact,
    Clipboard,
    Save,
    Upload(String),
    /// Copy the upload URL. The image file is left in place.
    CopyLink(LinkFormat),
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
    /// Redact (when enabled), clipboard (when enabled), save, an optional upload, then notify.
    ///
    /// `upload` is the target id. A target that is not configured fails that
    /// step and leaves a file saved by an earlier step on disk.
    pub fn from_settings(settings: &OutputSettings, upload: Option<String>) -> Self {
        let mut steps = Vec::new();
        if settings.auto_redact {
            steps.push(OutputStep::Redact);
        }
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
    run_with(pipeline.steps(), image, ctx, settings)
}

/// Run `steps` in order. A failed step is logged and notified. Later steps still run.
pub fn run_with(
    steps: &[OutputStep],
    image: &Image,
    ctx: &TemplateContext,
    settings: &Settings,
) -> OutputResult {
    run_steps(steps, image, ctx, settings, &LiveEffects)
}

/// Plain, Markdown, or HTML for an uploaded image.
pub fn format_link(format: LinkFormat, name: &str, url: &str) -> String {
    match format {
        LinkFormat::Plain => url.to_owned(),
        LinkFormat::Markdown => format!("![{}]({url})", markdown_label(name)),
        LinkFormat::Html => format!(
            "<img src=\"{}\" alt=\"{}\">",
            escape_attr(url),
            escape_attr(name)
        ),
    }
}

fn markdown_label(name: &str) -> String {
    name.replace(['[', ']'], "")
}

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    fn redact(&self, image: &Image) -> Result<Image, String> {
        Ok(image.clone())
    }
    fn upload(
        &self,
        target: &str,
        _bytes: &[u8],
        _name: &str,
        _mime: &str,
        _settings: &Settings,
    ) -> Result<snapper_upload::UploadResult, String> {
        Err(format!("upload target {target} is not available yet"))
    }
    fn copy_text(&self, text: &str) -> Result<(), String> {
        copy_text(text)
    }
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

    fn redact(&self, image: &Image) -> Result<Image, String> {
        crate::redact::recognise_and_apply(image)
    }

    fn upload(
        &self,
        target: &str,
        bytes: &[u8],
        name: &str,
        mime: &str,
        settings: &Settings,
    ) -> Result<snapper_upload::UploadResult, String> {
        let config = settings
            .uploads
            .iter()
            .find(|config| config.id() == target)
            .ok_or_else(|| format!("upload target {target} is not available yet"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("upload target {target} failed: {err}"))?;
        runtime
            .block_on(snapper_upload::upload_with(
                config,
                name,
                bytes.to_vec(),
                mime,
                &snapper_upload::Keychain,
            ))
            .map_err(|err| format!("upload target {target} failed: {err}"))
    }
}

struct Job<'a> {
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
        ctx,
        settings,
        effects,
    };
    let mut working = image.clone();
    let mut result = OutputResult {
        saved: None,
        url: None,
    };
    let mut redaction_failed = false;
    for step in steps {
        // After a failed redaction the image may still hold secrets, so it is
        // only saved locally, never copied or uploaded.
        if redaction_failed
            && matches!(
                step,
                OutputStep::Clipboard | OutputStep::Upload(_) | OutputStep::CopyLink(_)
            )
        {
            continue;
        }
        if let Err(err) = apply_step(step, &job, &mut working, &mut result) {
            tracing::error!(step = ?step, error = %err, "output step failed");
            if matches!(step, OutputStep::Redact) {
                redaction_failed = true;
                effects.notify_error(&format!(
                    "redaction failed, so the image was not copied or uploaded: {err}"
                ));
            } else {
                effects.notify_error(&err);
            }
        }
    }
    result
}

fn apply_step(
    step: &OutputStep,
    job: &Job<'_>,
    image: &mut Image,
    result: &mut OutputResult,
) -> Result<(), String> {
    match step {
        OutputStep::Redact => {
            *image = job.effects.redact(image)?;
            Ok(())
        }
        OutputStep::Clipboard => job.effects.copy(image),
        OutputStep::Save => {
            let path = save_image(image, job.ctx, &job.settings.output)?;
            result.saved = Some(path);
            Ok(())
        }
        OutputStep::Upload(target) => {
            let bytes = encode_memory(image, &job.settings.output)?;
            let name = file_name(result, &job.settings.output);
            let uploaded = job.effects.upload(
                target,
                &bytes,
                &name,
                mime(job.settings.output.format),
                job.settings,
            )?;
            if let Err(err) = remember_upload(result, &name, &uploaded.url) {
                tracing::warn!(error = %err, "could not append upload history");
            }
            result.url = Some(uploaded.url);
            Ok(())
        }
        OutputStep::CopyLink(format) => {
            let url = result
                .url
                .clone()
                .ok_or_else(|| "there is no link to copy".to_owned())?;
            let name = file_name(result, &job.settings.output);
            job.effects.copy_text(&format_link(*format, &name, &url))
        }
        OutputStep::Notify => {
            if let Some(path) = &result.saved {
                job.effects.notify_saved(path)?;
            }
            Ok(())
        }
    }
}

fn file_name(result: &OutputResult, settings: &OutputSettings) -> String {
    result
        .saved
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("snap.{}", extension(settings.format)))
}

fn mime(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Webp => "image/webp",
    }
}

fn remember_upload(result: &OutputResult, name: &str, url: &str) -> Result<(), String> {
    let path = snapper_core::settings::data_dir().join("history.jsonl");
    let file = result
        .saved
        .as_ref()
        .map(|saved| saved.display().to_string())
        .unwrap_or_else(|| name.to_owned());
    snapper_upload::append_history(
        &path,
        &snapper_upload::HistoryEntry {
            time: chrono::Local::now().to_rfc3339(),
            file,
            url: url.to_owned(),
        },
    )
    .map_err(|err| err.to_string())
}

fn encode_memory(image: &Image, settings: &OutputSettings) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut bytes);
        match settings.format {
            ImageFormat::Png => {
                let encoder = PngEncoder::new_with_quality(
                    cursor,
                    CompressionType::Fast,
                    FilterType::Adaptive,
                );
                encoder
                    .write_image(
                        &image.rgba,
                        image.width,
                        image.height,
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|err| err.to_string())?;
            }
            ImageFormat::Jpeg => {
                let rgb = rgba_to_rgb(&image.rgba);
                let encoder =
                    JpegEncoder::new_with_quality(cursor, jpeg_quality(settings.jpeg_quality));
                encoder
                    .write_image(&rgb, image.width, image.height, ExtendedColorType::Rgb8)
                    .map_err(|err| err.to_string())?;
            }
            ImageFormat::Webp => {
                let encoder = WebPEncoder::new_lossless(cursor);
                encoder
                    .write_image(
                        &image.rgba,
                        image.width,
                        image.height,
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|err| err.to_string())?;
            }
        }
    }
    Ok(bytes)
}

fn copy_text(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|err| err.to_string())?;
    clipboard.set_text(text).map_err(|err| err.to_string())
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
    // `create_new` claims the name, so two saves in the same second cannot
    // both pick it.
    let (path, file) = loop {
        let path = template::unique_path(&settings.directory, &stem, extension(settings.format));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("could not create {}: {err}", path.display())),
        }
    };
    if let Err(err) = encode_file(file, image, settings) {
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

fn encode_file(
    file: std::fs::File,
    image: &Image,
    settings: &OutputSettings,
) -> Result<(), String> {
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
    for pixel in rgba.chunks_exact(4) {
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

/// Show a notification. A failure is logged and does not stop the caller.
pub fn notify(title: &str, body: &str) {
    if let Err(err) = spawn_notification(title, body, None) {
        tracing::warn!(error = %err, "notification failed");
    }
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

    struct FailingRedact(Record);

    impl Effects for FailingRedact {
        fn copy(&self, image: &Image) -> Result<(), String> {
            self.0.copy(image)
        }

        fn notify_saved(&self, path: &Path) -> Result<(), String> {
            self.0.notify_saved(path)
        }

        fn notify_error(&self, message: &str) {
            self.0.notify_error(message);
        }

        fn redact(&self, _image: &Image) -> Result<Image, String> {
            Err("no OCR engine".to_owned())
        }

        fn upload(
            &self,
            _target: &str,
            _bytes: &[u8],
            _name: &str,
            _mime: &str,
            _settings: &Settings,
        ) -> Result<snapper_upload::UploadResult, String> {
            panic!("an unredacted image was uploaded");
        }
    }

    #[test]
    fn a_failed_redaction_saves_but_does_not_copy_or_upload() {
        let dir = scratch("redact");
        let image = pixel();
        let ctx = fixed_ctx(&image);
        let mut prefs = settings(dir.clone(), ImageFormat::Png, true);
        prefs.output.auto_redact = true;
        let steps = OutputPipeline::from_settings(&prefs.output, Some("s3".to_owned()))
            .steps()
            .to_vec();
        let effects = FailingRedact(Record::new());
        let result = run_steps(&steps, &image, &ctx, &prefs, &effects);
        assert!(result.saved.is_some());
        assert_eq!(result.url, None);
        assert!(!*effects.0.copied.lock().expect("copied"));
        let errors = effects.0.errors.lock().expect("errors");
        assert!(errors.iter().any(|err| err.contains("redaction failed")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_formats_escape_the_name_and_url() {
        assert_eq!(
            format_link(LinkFormat::Plain, "shot", "https://cdn.example/a.png"),
            "https://cdn.example/a.png"
        );
        assert_eq!(
            format_link(LinkFormat::Markdown, "a[b]", "https://cdn.example/a.png"),
            "![ab](https://cdn.example/a.png)"
        );
        assert_eq!(
            format_link(
                LinkFormat::Html,
                "a<b>",
                "https://cdn.example/a.png?q=\"1\""
            ),
            "<img src=\"https://cdn.example/a.png?q=&quot;1&quot;\" alt=\"a&lt;b&gt;\">"
        );
    }

    #[test]
    fn auto_redact_is_the_first_step_only_when_enabled() {
        let mut output = OutputSettings::default();
        assert!(!output.auto_redact);
        assert!(!OutputPipeline::from_settings(&output, None)
            .steps()
            .contains(&OutputStep::Redact));
        output.auto_redact = true;
        assert_eq!(
            OutputPipeline::from_settings(&output, None).steps()[0],
            OutputStep::Redact
        );
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
