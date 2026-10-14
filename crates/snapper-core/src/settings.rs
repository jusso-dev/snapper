//! User settings stored as TOML in the OS config directory.
//!
//! Unknown top-level keys are kept in [`Settings::extra`] so a newer or
//! hand-edited file is not stripped on save.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current settings document version written by this crate.
pub const SETTINGS_VERSION: u32 = 1;

/// Failure while reading or writing the settings file.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Filesystem error creating, reading, or replacing the file.
    #[error("settings io error: {0}")]
    Io(#[from] std::io::Error),
    /// The file was present but was not valid TOML for [`Settings`].
    #[error("settings parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// The in-memory settings could not be encoded as TOML.
    #[error("settings serialise error: {0}")]
    Serialise(#[from] toml::ser::Error),
}

/// Image file format used when a capture is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    /// Lossless PNG. This is the default.
    #[default]
    Png,
    /// JPEG at [`OutputSettings::jpeg_quality`].
    Jpeg,
    /// WebP.
    Webp,
}

/// Global hotkey chords. The strings are parsed by the app later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    /// Capture a region.
    pub region: String,
    /// Capture the window under the cursor.
    pub window: String,
    /// Capture the full screen.
    pub full_screen: String,
    /// Repeat the last region.
    pub last_region: String,
    /// Start or stop a recording.
    pub record: String,
    /// Recognise text in a region.
    pub ocr: String,
    /// Capture a scrolling region.
    pub scroll: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            region: "PrintScreen".to_owned(),
            window: "Alt+PrintScreen".to_owned(),
            full_screen: "Ctrl+PrintScreen".to_owned(),
            last_region: "Shift+PrintScreen".to_owned(),
            record: "Ctrl+Shift+R".to_owned(),
            ocr: "Ctrl+Shift+O".to_owned(),
            scroll: "Ctrl+Shift+S".to_owned(),
        }
    }
}

/// Where captures go after the editor, if one is opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSettings {
    /// Directory created on demand when a capture is saved.
    pub directory: PathBuf,
    /// Filename stem. Placeholders are expanded by the template engine.
    pub filename_template: String,
    /// Container written by the output pipeline.
    pub format: ImageFormat,
    /// Copy the image to the clipboard.
    pub copy_to_clipboard: bool,
    /// Open the annotation editor before output.
    pub open_editor: bool,
    /// JPEG quality from 1 to 100. Ignored for other formats.
    pub jpeg_quality: u8,
    /// Run OCR and pixelate secrets before the image is copied or saved.
    pub auto_redact: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            directory: default_output_directory(),
            filename_template: "Snapper_{date}_{time}".to_owned(),
            format: ImageFormat::Png,
            copy_to_clipboard: true,
            open_editor: false,
            jpeg_quality: 90,
            auto_redact: false,
        }
    }
}

/// How a workflow takes the screenshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    /// Drag a region.
    Region,
    /// The window under the pointer.
    Window,
    /// The primary monitor, or the virtual desktop when there is no primary.
    FullScreen,
    /// One monitor id from `snapper-cli list monitors`.
    Monitor { id: u32 },
}

/// How an uploaded URL is written to the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LinkFormat {
    /// The URL on its own.
    Plain,
    /// `![name](url)`.
    #[default]
    Markdown,
    /// An `<img>` tag.
    Html,
}

/// One stage a workflow runs after capture.
///
/// `upload.target` is the id of an entry in [`Settings::uploads`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum PipelineStep {
    /// OCR the image and pixelate detected secrets.
    Redact,
    /// Open the annotation editor before the remaining steps.
    Annotate,
    /// Copy the image to the clipboard.
    Clipboard,
    /// Save using the output directory and filename template.
    Save,
    /// Upload to a configured target.
    Upload { target: String },
    /// Copy a link in [`Workflow::link_format`].
    CopyLink,
    /// Show a notification.
    Notify,
}

/// A named after-capture sequence bound to a hotkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    /// Empty means the workflow is only started from the tray or CLI.
    pub hotkey: String,
    pub capture: CaptureKind,
    pub steps: Vec<PipelineStep>,
    pub link_format: LinkFormat,
}

/// An upload destination stored in `config.toml`.
///
/// Secret keys, tokens, and `Authorization` header values are not fields of
/// this enum. They live in the OS keychain under the service name `snapper`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadConfig {
    /// S3-compatible storage, including AWS S3 and Cloudflare R2.
    S3 {
        id: String,
        endpoint: String,
        bucket: String,
        region: String,
        #[serde(default)]
        key_prefix: String,
        public_base_url: String,
        access_key_id: String,
    },
    /// Anonymous Imgur upload. The Client-ID is an application identifier.
    Imgur { id: String, client_id: String },
    /// Multipart HTTP upload, including targets imported from `.sxcu` files.
    Http {
        id: String,
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default = "default_file_field")]
        file_field: String,
        /// Header values may be `keyring:<account>` so the secret stays out of this file.
        #[serde(default)]
        headers: Vec<HeaderSetting>,
        /// JSON pointer, such as `/data/link`, to the public URL in the response.
        #[serde(default)]
        url_json_pointer: String,
        #[serde(default)]
        delete_url_json_pointer: String,
    },
    /// External `snapper-plugin-<id>` executable.
    External {
        id: String,
        /// Empty uses `snapper-plugin-<id>` on `PATH` or in the plugins directory.
        #[serde(default)]
        command: String,
        #[serde(default)]
        config: toml::Table,
    },
}

/// One HTTP header. `value` is either a literal or `keyring:<account>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderSetting {
    pub name: String,
    pub value: String,
}

fn default_http_method() -> String {
    "POST".to_owned()
}

fn default_file_field() -> String {
    "file".to_owned()
}

impl UploadConfig {
    /// Stable id used by workflows and the keychain account name.
    pub fn id(&self) -> &str {
        match self {
            Self::S3 { id, .. }
            | Self::Imgur { id, .. }
            | Self::Http { id, .. }
            | Self::External { id, .. } => id,
        }
    }
}

/// Last editor choices that should survive a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// Stroke colour as `#RRGGBBAA`.
    pub stroke_color: String,
    /// Stroke width in physical pixels.
    pub stroke_width: f32,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            stroke_color: "#E23D3DFF".to_owned(),
            stroke_width: 4.0,
        }
    }
}

/// All user preferences.
///
/// `extra` holds top-level keys this version does not know about. They are
/// written back unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Schema version. [`SETTINGS_VERSION`] is written by [`Settings::default`].
    pub version: u32,
    /// Hotkey chords.
    pub hotkeys: HotkeySettings,
    /// Save and clipboard behaviour.
    pub output: OutputSettings,
    /// Annotation editor preferences.
    pub editor: EditorSettings,
    /// Start Snapper when the user signs in.
    ///
    /// The settings window applies this when the change is saved. The tray
    /// app also applies it on launch, so a hand-edited file takes effect then.
    pub start_at_login: bool,
    /// Look for a signed GitHub release. Snapper never installs it unattended.
    pub check_updates: bool,
    /// Milliseconds between frames while scrolling a capture.
    pub scroll_delay_ms: u64,
    /// Tallest scrolling capture, in physical pixels.
    pub scroll_max_height: u32,
    /// Configured upload targets. Secrets are not stored here.
    pub uploads: Vec<UploadConfig>,
    /// Named workflows.
    pub workflows: Vec<Workflow>,
    /// Unknown top-level keys, preserved across save.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            hotkeys: HotkeySettings::default(),
            output: OutputSettings::default(),
            editor: EditorSettings::default(),
            start_at_login: false,
            check_updates: true,
            scroll_delay_ms: 250,
            scroll_max_height: 30_000,
            uploads: Vec::new(),
            workflows: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// Directory for history, plugins, and other data that is not configuration.
///
/// Uses `ProjectDirs` with qualifier `au`, organisation `jusso-dev` and
/// application `snapper`.
pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("au", "jusso-dev", "snapper")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Absolute path of `config.toml` for this OS user.
///
/// Falls back to a relative `config.toml` when the home directory cannot be resolved.
pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("au", "jusso-dev", "snapper")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

fn default_output_directory() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.picture_dir().map(|pictures| pictures.join("Snapper")))
        .unwrap_or_else(|| PathBuf::from("Snapper"))
}

impl Settings {
    /// Load settings from `path`.
    ///
    /// A missing file returns [`Settings::default`] and does not create it.
    /// Any other IO error is returned.
    pub fn load_from(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Load settings from [`config_path`].
    pub fn load() -> Result<Self, SettingsError> {
        Self::load_from(&config_path())
    }

    /// Write settings to [`config_path`], creating parent directories.
    pub fn save(&self) -> Result<(), SettingsError> {
        self.save_to(&config_path())
    }

    /// Write settings to `path` atomically.
    ///
    /// The document is written to a sibling `*.tmp` file, flushed, then
    /// renamed over `path`. A crash during the write leaves the previous
    /// file in place. On Windows, `rename` cannot replace an existing file,
    /// so an `AlreadyExists` error removes the destination and retries.
    pub fn save_to(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let text = toml::to_string_pretty(self)?;
        let tmp = tmp_sibling(path);
        write_synced(&tmp, text.as_bytes())?;
        if let Err(err) = replace_file(&tmp, path) {
            // Drop the temp file only when the previous settings are still in
            // place. On Windows the replace path may already have removed
            // `path`, and deleting `tmp` then would throw away the new copy.
            if path.exists() {
                let _ = std::fs::remove_file(&tmp);
            }
            return Err(err.into());
        }
        Ok(())
    }
}

fn tmp_sibling(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn replace_file(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    match std::fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(dest)?;
            std::fs::rename(tmp, dest)
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[test]
    fn default_serialises_every_section() {
        let text = toml::to_string_pretty(&Settings::default()).expect("serialise");
        let parsed: toml::Value = toml::from_str(&text).expect("default toml is valid");
        assert!(parsed.get("version").is_some());
        assert!(parsed.get("hotkeys").is_some(), "{text}");
        assert!(parsed.get("output").is_some(), "{text}");
        assert!(parsed.get("editor").is_some(), "{text}");

        let round: Settings = toml::from_str(&text).expect("parse default");
        assert_eq!(round, Settings::default());
        assert_eq!(round.hotkeys.region, "PrintScreen");
        assert_eq!(round.hotkeys.window, "Alt+PrintScreen");
        assert_eq!(round.hotkeys.full_screen, "Ctrl+PrintScreen");
        assert_eq!(round.hotkeys.last_region, "Shift+PrintScreen");
        assert_eq!(round.hotkeys.record, "Ctrl+Shift+R");
        assert_eq!(round.hotkeys.ocr, "Ctrl+Shift+O");
        assert_eq!(round.hotkeys.scroll, "Ctrl+Shift+S");
        assert!(round.check_updates);
        assert_eq!(round.scroll_delay_ms, 250);
        assert_eq!(round.scroll_max_height, 30_000);
        assert!(round.uploads.is_empty());
        assert!(round.workflows.is_empty());
        assert!(!round.output.auto_redact);
        assert!(round.output.directory.ends_with("Snapper"));
        assert_eq!(round.output.filename_template, "Snapper_{date}_{time}");
        assert_eq!(round.output.format, ImageFormat::Png);
        assert!(round.output.copy_to_clipboard);
        assert!(!round.output.open_editor);
        assert_eq!(round.output.jpeg_quality, 90);
        assert!(!round.start_at_login);
    }

    #[test]
    fn missing_file_returns_defaults_and_creates_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        let loaded = Settings::load_from(&path).expect("load missing");
        assert_eq!(loaded, Settings::default());
        assert!(!path.exists());
        assert!(!tmp_sibling(&path).exists());
    }

    #[test]
    fn round_trip_preserves_edits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        let mut settings = Settings::default();
        settings.output.filename_template = "bug_{date}".to_owned();
        settings.output.format = ImageFormat::Jpeg;
        settings.output.jpeg_quality = 80;
        settings.output.copy_to_clipboard = false;
        settings.editor.stroke_width = 2.5;
        settings.save_to(&path).expect("save");

        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(loaded, settings);
    }

    #[test]
    fn partial_sections_keep_other_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        std::fs::write(
            &path,
            "[hotkeys]\nregion = \"F1\"\n\n[output]\nfilename_template = \"only\"\n",
        )
        .expect("seed");

        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(loaded.hotkeys.region, "F1");
        assert_eq!(loaded.hotkeys.window, HotkeySettings::default().window);
        assert_eq!(loaded.hotkeys.record, HotkeySettings::default().record);
        assert_eq!(loaded.output.filename_template, "only");
        assert!(loaded.output.copy_to_clipboard);
        assert_eq!(loaded.output.jpeg_quality, 90);
        assert_eq!(loaded.output.format, ImageFormat::Png);
        assert_eq!(loaded.editor, EditorSettings::default());
        assert!(!loaded.start_at_login);
        assert_eq!(loaded.version, SETTINGS_VERSION);
    }

    #[test]
    fn unknown_top_level_key_survives_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        std::fs::write(&path, "foo = 1\n").expect("seed");

        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(
            loaded.extra.get("foo").and_then(toml::Value::as_integer),
            Some(1)
        );
        assert_eq!(loaded.version, SETTINGS_VERSION);
        assert_eq!(loaded.hotkeys, HotkeySettings::default());

        loaded.save_to(&path).expect("save");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.contains("foo"), "{text}");
        assert!(!text.contains("foo = 1\nfoo"), "{text}");

        let again = Settings::load_from(&path).expect("reload");
        assert_eq!(
            again.extra.get("foo").and_then(toml::Value::as_integer),
            Some(1)
        );
        assert_eq!(again.hotkeys.region, HotkeySettings::default().region);
    }

    #[test]
    fn save_is_atomic_tmp_then_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        std::fs::write(&path, "version = 1\nstale = true\n").expect("seed");

        let settings = Settings::default();
        settings.save_to(&path).expect("first save");
        settings.save_to(&path).expect("replace existing");

        let tmp = tmp_sibling(&path);
        assert!(!tmp.exists(), "tmp file left behind at {}", tmp.display());
        let text = std::fs::read_to_string(&path).expect("read");
        let parsed: Settings = toml::from_str(&text).expect("complete document");
        assert_eq!(parsed, settings);
        assert!(!text.contains("stale"), "{text}");
    }

    #[test]
    fn upload_config_round_trip_has_no_secret_field() {
        let mut settings = Settings::default();
        settings.uploads.push(UploadConfig::S3 {
            id: "r2".to_owned(),
            endpoint: "https://example.r2.cloudflarestorage.com".to_owned(),
            bucket: "shots".to_owned(),
            region: "auto".to_owned(),
            key_prefix: "snap".to_owned(),
            public_base_url: "https://cdn.example".to_owned(),
            access_key_id: "AKIAEXAMPLE".to_owned(),
        });
        settings.workflows.push(Workflow {
            name: "Bug report".to_owned(),
            hotkey: "Ctrl+Shift+B".to_owned(),
            capture: CaptureKind::Region,
            steps: vec![
                PipelineStep::Redact,
                PipelineStep::Annotate,
                PipelineStep::Upload {
                    target: "r2".to_owned(),
                },
                PipelineStep::CopyLink,
            ],
            link_format: LinkFormat::Markdown,
        });
        let text = toml::to_string_pretty(&settings).expect("serialise");
        assert!(!text.to_ascii_lowercase().contains("secret"), "{text}");
        assert!(text.contains("AKIAEXAMPLE"), "{text}");
        let loaded: Settings = toml::from_str(&text).expect("parse");
        assert_eq!(loaded.uploads, settings.uploads);
        assert_eq!(loaded.workflows, settings.workflows);
    }

    #[test]
    fn config_path_ends_with_config_toml() {
        let path = config_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("config.toml")
        );
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = temp_config(&dir, "config.toml");
        std::fs::write(&path, "version = [\n").expect("seed");
        let err = Settings::load_from(&path).expect_err("parse");
        assert!(matches!(err, SettingsError::Parse(_)));
    }
}
