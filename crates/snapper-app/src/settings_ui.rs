//! Settings window.
//!
//! [`show`] draws one frame and reports what changed. The window saves through
//! [`Settings::save`] after edits have been quiet for 500 ms, and again when
//! the window closes, so a change inside that window is not lost. Hotkeys are
//! re-registered after a save that touched them. `start_at_login` is applied
//! to the operating system at the same time.
//!
//! Upload targets, workflows, and the update check are edited here.

use std::time::{Duration, Instant};

use chrono::TimeZone;

use eframe::egui::{self, Color32, ViewportBuilder, ViewportCommand};
use snapper_core::settings::{HotkeySettings, ImageFormat};
use snapper_core::template::{self, TemplateContext};
use snapper_core::Settings;
use snapper_editor::Rgba;

use crate::editor::{format_colour, parse_colour};
use crate::hotkeys;
use crate::login;
use crate::shell;

const DEBOUNCE: Duration = Duration::from_millis(500);

/// Shown on the About tab. Australian spelling, matching the issue.
const LICENCE: &str =
    "Snapper is released under the MIT Licence. Copyright (c) 2026 Justin Middler.";

/// What [`show`] changed during this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsChange {
    /// No field changed.
    Unchanged,
    /// At least one field changed.
    ///
    /// `hotkeys` means the chords should be registered again after the save.
    /// `login` means the start-at-login entry should be applied after the save.
    Edited { hotkeys: bool, login: bool },
}

#[derive(Default)]
struct FieldChange {
    edited: bool,
    hotkeys: bool,
    login: bool,
}

impl FieldChange {
    fn finish(self) -> SettingsChange {
        if self.edited {
            SettingsChange::Edited {
                hotkeys: self.hotkeys,
                login: self.login,
            }
        } else {
            SettingsChange::Unchanged
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Hotkeys,
    Output,
    Editor,
    Uploads,
    Workflows,
    About,
}

impl Tab {
    const ALL: [Self; 7] = [
        Self::General,
        Self::Hotkeys,
        Self::Output,
        Self::Editor,
        Self::Uploads,
        Self::Workflows,
        Self::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Hotkeys => "Hotkeys",
            Self::Output => "Output",
            Self::Editor => "Editor",
            Self::Uploads => "Uploads",
            Self::Workflows => "Workflows",
            Self::About => "About",
        }
    }
}

/// One chord in [`HotkeySettings`]. Workflow chords are edited on their own tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyField {
    Region,
    Window,
    FullScreen,
    LastRegion,
    Record,
    Ocr,
    Scroll,
}

impl HotkeyField {
    const ALL: [Self; 7] = [
        Self::Region,
        Self::Window,
        Self::FullScreen,
        Self::LastRegion,
        Self::Record,
        Self::Ocr,
        Self::Scroll,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Region => "Capture region",
            Self::Window => "Capture window",
            Self::FullScreen => "Capture full screen",
            Self::LastRegion => "Capture last region",
            Self::Record => "Record",
            Self::Ocr => "OCR region",
            Self::Scroll => "Scrolling capture",
        }
    }

    fn get(self, hotkeys: &HotkeySettings) -> &str {
        match self {
            Self::Region => hotkeys.region.as_str(),
            Self::Window => hotkeys.window.as_str(),
            Self::FullScreen => hotkeys.full_screen.as_str(),
            Self::LastRegion => hotkeys.last_region.as_str(),
            Self::Record => hotkeys.record.as_str(),
            Self::Ocr => hotkeys.ocr.as_str(),
            Self::Scroll => hotkeys.scroll.as_str(),
        }
    }

    fn set(self, hotkeys: &mut HotkeySettings, chord: String) {
        *match self {
            Self::Region => &mut hotkeys.region,
            Self::Window => &mut hotkeys.window,
            Self::FullScreen => &mut hotkeys.full_screen,
            Self::LastRegion => &mut hotkeys.last_region,
            Self::Record => &mut hotkeys.record,
            Self::Ocr => &mut hotkeys.ocr,
            Self::Scroll => &mut hotkeys.scroll,
        } = chord;
    }
}

#[derive(Clone, Debug)]
struct UiState {
    tab: Tab,
    recording: Option<HotkeyField>,
    error_field: Option<HotkeyField>,
    hotkey_error: String,
    about_note: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            tab: Tab::General,
            recording: None,
            error_field: None,
            hotkey_error: String::new(),
            about_note: String::new(),
        }
    }
}

fn state_id() -> egui::Id {
    egui::Id::new("snapper-settings-ui")
}

fn load_state(ctx: &egui::Context) -> UiState {
    ctx.data(|data| data.get_temp(state_id()))
        .unwrap_or_default()
}

fn store_state(ctx: &egui::Context, state: &UiState) {
    ctx.data_mut(|data| data.insert_temp(state_id(), state.clone()));
}

/// Draw the settings tabs and edit `settings` in place.
///
/// Unknown top-level keys in `settings.extra` are left alone. The caller
/// debounces [`Settings::save`]. A returned [`SettingsChange::Edited`] means
/// something changed on this frame only.
pub fn show(ctx: &egui::Context, settings: &mut Settings) -> SettingsChange {
    let mut state = load_state(ctx);
    let mut change = FieldChange::default();
    if state.recording.is_some() {
        capture_chord(ctx, settings, &mut state, &mut change);
    }
    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    if ui.selectable_label(state.tab == tab, tab.label()).clicked() {
                        state.tab = tab;
                    }
                }
            });
            ui.separator();
            match state.tab {
                Tab::General => general_tab(ui, settings, &mut change),
                Tab::Hotkeys => hotkeys_tab(ui, settings, &mut state),
                Tab::Output => output_tab(ui, settings, &mut change),
                Tab::Editor => editor_tab(ui, settings, &mut change),
                Tab::Uploads => uploads_tab(ui, settings, &mut change),
                Tab::Workflows => workflows_tab(ui, settings, &mut change),
                Tab::About => about_tab(ui, &mut state),
            }
        });
    });
    store_state(ctx, &state);
    change.finish()
}

/// Open the settings window. Returns when the window closes.
///
/// Saves are debounced. Closing the window flushes a pending save first.
pub fn run(settings: Settings) {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Snapper settings")
            .with_inner_size([720.0, 520.0]),
        persist_window: false,
        run_and_return: true,
        ..Default::default()
    };
    let app = SettingsWindow {
        settings,
        pending: None,
        save_error: None,
    };
    if let Err(err) = eframe::run_native(
        "Snapper settings",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    ) {
        eprintln!("settings failed: {err}");
        std::process::exit(1);
    }
}

struct PendingSave {
    at: Instant,
    hotkeys: bool,
    login: bool,
}

struct SettingsWindow {
    settings: Settings,
    pending: Option<PendingSave>,
    save_error: Option<String>,
}

impl SettingsWindow {
    fn note(&mut self, change: SettingsChange) {
        let SettingsChange::Edited { hotkeys, login } = change else {
            return;
        };
        note_edit(&mut self.pending, hotkeys, login, Instant::now());
    }

    fn flush_if_due(&mut self, ctx: &egui::Context) {
        let due = self
            .pending
            .as_ref()
            .is_some_and(|pending| save_is_due(pending, Instant::now()));
        if due {
            let _ = self.flush();
            if self.pending.is_some() {
                ctx.request_repaint_after(DEBOUNCE);
            }
            return;
        }
        let Some(pending) = &self.pending else {
            return;
        };
        ctx.request_repaint_after(DEBOUNCE.saturating_sub(pending.at.elapsed()));
    }

    /// Write a pending edit. Returns false when the file itself could not be saved.
    fn flush(&mut self) -> bool {
        let Some(pending) = self.pending.take() else {
            return true;
        };
        if let Err(err) = self.settings.save() {
            tracing::warn!(error = %err, "could not save settings");
            self.save_error = Some(format!("Could not save settings: {err}"));
            self.pending = Some(PendingSave {
                at: Instant::now(),
                hotkeys: pending.hotkeys,
                login: pending.login,
            });
            return false;
        }
        self.save_error = None;
        if pending.hotkeys {
            if let Err(err) = shell::reregister(&self.settings) {
                tracing::warn!(error = %err, "could not register hotkeys");
                self.save_error = Some(format!("Could not register hotkeys: {err}"));
                self.pending = Some(PendingSave {
                    at: Instant::now(),
                    hotkeys: true,
                    login: pending.login,
                });
                return true;
            }
        }
        if pending.login {
            if let Err(err) = login::apply(self.settings.start_at_login) {
                tracing::warn!(error = %err, "could not update start at login");
                self.save_error = Some(format!("Could not update start at login: {err}"));
                self.pending = Some(PendingSave {
                    at: Instant::now(),
                    hotkeys: false,
                    login: true,
                });
            }
        }
        true
    }
}

impl eframe::App for SettingsWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(error) = &self.save_error {
            let error = error.clone();
            egui::TopBottomPanel::bottom("settings-save-error").show(ctx, |ui| {
                ui.colored_label(Color32::from_rgb(200, 70, 70), error);
            });
        }
        let change = show(ctx, &mut self.settings);
        self.note(change);
        if ctx.input(|input| input.viewport().close_requested()) {
            // Keep the window open only when the file write failed. A hotkey
            // or login failure is retried, and the next shell start registers
            // chords again when the manager is missing.
            if !self.flush() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        } else {
            self.flush_if_due(ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.flush();
    }
}

fn note_edit(pending: &mut Option<PendingSave>, hotkeys: bool, login: bool, now: Instant) {
    match pending {
        Some(pending) => {
            pending.at = now;
            pending.hotkeys |= hotkeys;
            pending.login |= login;
        }
        None => {
            *pending = Some(PendingSave {
                at: now,
                hotkeys,
                login,
            });
        }
    }
}

fn save_is_due(pending: &PendingSave, now: Instant) -> bool {
    now.saturating_duration_since(pending.at) >= DEBOUNCE
}

fn general_tab(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    ui.label("Changes are saved about half a second after you stop editing.");
    ui.add_space(8.0);
    if ui
        .checkbox(&mut settings.start_at_login, "Start Snapper at login")
        .changed()
    {
        change.edited = true;
        change.login = true;
    }
    if ui
        .checkbox(
            &mut settings.check_updates,
            "Check for updates (never installs them)",
        )
        .changed()
    {
        change.edited = true;
    }
    ui.horizontal(|ui| {
        ui.label("Scroll delay (ms)");
        if ui
            .add(egui::DragValue::new(&mut settings.scroll_delay_ms).speed(10.0))
            .changed()
        {
            change.edited = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Scroll max height");
        if ui
            .add(egui::DragValue::new(&mut settings.scroll_max_height).speed(100.0))
            .changed()
        {
            change.edited = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Settings version");
        if ui
            .add(egui::DragValue::new(&mut settings.version).speed(1.0))
            .changed()
        {
            change.edited = true;
        }
    });
}

fn hotkeys_tab(ui: &mut egui::Ui, settings: &Settings, state: &mut UiState) {
    ui.label("Click a chord, then press the next key. Escape cancels.");
    ui.label("Print Screen is not reported to this window. A Print Screen chord stays until you record a different key.");
    ui.add_space(8.0);
    for field in HotkeyField::ALL {
        hotkey_row(ui, settings, state, field);
    }
}

fn hotkey_row(ui: &mut egui::Ui, settings: &Settings, state: &mut UiState, field: HotkeyField) {
    ui.horizontal(|ui| {
        ui.label(field.label());
        let listening = state.recording == Some(field);
        let text = if listening {
            "Press a key…".to_owned()
        } else {
            display_chord(field.get(&settings.hotkeys))
        };
        if ui
            .add_sized([200.0, 24.0], egui::Button::new(text))
            .clicked()
        {
            state.hotkey_error.clear();
            state.error_field = None;
            state.recording = if listening { None } else { Some(field) };
        }
    });
    if state.error_field == Some(field) && !state.hotkey_error.is_empty() {
        ui.colored_label(Color32::from_rgb(200, 70, 70), &state.hotkey_error);
    }
}

fn output_tab(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    ui.horizontal(|ui| {
        ui.label("Save folder");
        let mut directory = settings.output.directory.to_string_lossy().into_owned();
        if ui
            .add(egui::TextEdit::singleline(&mut directory).desired_width(360.0))
            .changed()
        {
            settings.output.directory = directory.into();
            change.edited = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Filename");
        let mut template = settings.output.filename_template.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut template).desired_width(360.0))
            .changed()
        {
            settings.output.filename_template = template;
            change.edited = true;
        }
    });
    ui.label("Preview");
    ui.monospace(filename_preview(
        &settings.output.filename_template,
        settings.output.format,
    ));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Format");
        egui::ComboBox::from_id_salt("image-format")
            .selected_text(format_label(settings.output.format))
            .show_ui(ui, |ui| {
                for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Webp] {
                    if ui
                        .selectable_value(&mut settings.output.format, format, format_label(format))
                        .changed()
                    {
                        change.edited = true;
                    }
                }
            });
    });
    if ui
        .checkbox(
            &mut settings.output.copy_to_clipboard,
            "Copy to the clipboard",
        )
        .changed()
    {
        change.edited = true;
    }
    if ui
        .checkbox(
            &mut settings.output.open_editor,
            "Open the editor after a capture",
        )
        .changed()
    {
        change.edited = true;
    }
    if ui
        .checkbox(
            &mut settings.output.auto_redact,
            "Redact secrets, emails and card numbers",
        )
        .changed()
    {
        change.edited = true;
    }
    ui.horizontal(|ui| {
        ui.label("JPEG quality");
        if ui
            .add(egui::Slider::new(
                &mut settings.output.jpeg_quality,
                1..=100,
            ))
            .changed()
        {
            change.edited = true;
        }
    });
}

fn editor_tab(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    ui.horizontal(|ui| {
        ui.label("Stroke colour");
        let mut text = settings.editor.stroke_color.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut text).desired_width(140.0))
            .changed()
        {
            settings.editor.stroke_color = text;
            change.edited = true;
        }
        let mut colour = colour_to_color32(&settings.editor.stroke_color);
        if egui::widgets::color_picker::color_edit_button_srgba(
            ui,
            &mut colour,
            egui::widgets::color_picker::Alpha::OnlyBlend,
        )
        .changed()
        {
            let hex = color32_to_hex(colour);
            if hex != settings.editor.stroke_color {
                settings.editor.stroke_color = hex;
                change.edited = true;
            }
        }
    });
    if !is_hex_colour(&settings.editor.stroke_color) {
        ui.colored_label(Color32::from_rgb(200, 70, 70), "Use #RRGGBB or #RRGGBBAA.");
    }
    ui.horizontal(|ui| {
        ui.label("Stroke width");
        if ui
            .add(egui::Slider::new(
                &mut settings.editor.stroke_width,
                0.5..=64.0,
            ))
            .changed()
        {
            change.edited = true;
        }
    });
}

fn uploads_tab(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    ui.label("Secrets stay in the system keychain. They are not written to config.toml.");
    if settings.uploads.is_empty() {
        ui.label("No upload targets yet.");
    }
    let mut remove = None;
    for (index, upload) in settings.uploads.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{} ({})", upload.id(), upload_kind(upload)));
            if ui.button("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        settings.uploads.remove(index);
        change.edited = true;
    }
    ui.add_space(8.0);
    if ui.button("Add S3 or R2 target").clicked() {
        settings
            .uploads
            .push(snapper_core::settings::UploadConfig::S3 {
                id: format!("s3-{}", settings.uploads.len() + 1),
                endpoint: "https://example.r2.cloudflarestorage.com".to_owned(),
                bucket: "shots".to_owned(),
                region: "auto".to_owned(),
                key_prefix: "snap".to_owned(),
                public_base_url: String::new(),
                access_key_id: String::new(),
            });
        change.edited = true;
    }
    if ui.button("Add Imgur target").clicked() {
        settings
            .uploads
            .push(snapper_core::settings::UploadConfig::Imgur {
                id: format!("imgur-{}", settings.uploads.len() + 1),
                client_id: String::new(),
            });
        change.edited = true;
    }
    edit_upload_fields(ui, settings, change);
    ui.add_space(8.0);
    ui.label("Import a ShareX .sxcu file");
    let mut path = ui
        .ctx()
        .data(|data| data.get_temp::<String>(egui::Id::new("sxcu-path")))
        .unwrap_or_default();
    if ui
        .add(egui::TextEdit::singleline(&mut path).hint_text("/path/to/uploader.sxcu"))
        .changed()
    {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("sxcu-path"), path.clone()));
    } else {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("sxcu-path"), path.clone()));
    }
    if ui.button("Import .sxcu").clicked() {
        match snapper_upload::import_sxcu(std::path::Path::new(&path), &snapper_upload::Keychain) {
            Ok(config) => {
                settings.uploads.push(config);
                change.edited = true;
            }
            Err(err) => {
                ui.label(format!("Import failed: {err}"));
            }
        }
    }
    ui.add_space(8.0);
    ui.label("Discovered plugins");
    let plugin_id = egui::Id::new("snapper-discovered-plugins");
    let plugins = ui
        .ctx()
        .data(|data| data.get_temp::<Vec<snapper_upload::DiscoveredPlugin>>(plugin_id))
        .unwrap_or_else(|| {
            let found = snapper_upload::discover();
            ui.ctx()
                .data_mut(|data| data.insert_temp(plugin_id, found.clone()));
            found
        });
    if plugins.is_empty() {
        ui.label("No snapper-plugin-* programs were found on PATH or in the plugins folder.");
    }
    for plugin in plugins {
        ui.horizontal(|ui| {
            ui.label(&plugin.id);
            if ui.button("Add").clicked() {
                settings
                    .uploads
                    .push(snapper_core::settings::UploadConfig::External {
                        id: plugin.id.clone(),
                        command: plugin.path.display().to_string(),
                        config: toml::Table::new(),
                    });
                change.edited = true;
            }
        });
        if let Some(note) = plugin.schema.get("title").and_then(|item| item.as_str()) {
            ui.label(note);
        }
    }
}

fn upload_kind(upload: &snapper_core::settings::UploadConfig) -> &'static str {
    match upload {
        snapper_core::settings::UploadConfig::S3 { .. } => "S3",
        snapper_core::settings::UploadConfig::Imgur { .. } => "Imgur",
        snapper_core::settings::UploadConfig::Http { .. } => "HTTP",
        snapper_core::settings::UploadConfig::External { .. } => "Plugin",
    }
}

fn edit_upload_fields(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    for upload in &mut settings.uploads {
        ui.separator();
        match upload {
            snapper_core::settings::UploadConfig::S3 {
                id,
                endpoint,
                bucket,
                region,
                key_prefix,
                public_base_url,
                access_key_id,
            } => {
                text_row(ui, "Id", id, change);
                text_row(ui, "Endpoint", endpoint, change);
                text_row(ui, "Bucket", bucket, change);
                text_row(ui, "Region", region, change);
                text_row(ui, "Key prefix", key_prefix, change);
                text_row(ui, "Public URL", public_base_url, change);
                text_row(ui, "Access key id", access_key_id, change);
                ui.label("The secret access key is stored in the keychain as s3:<id>, not here.");
            }
            snapper_core::settings::UploadConfig::Imgur { id, client_id } => {
                text_row(ui, "Id", id, change);
                text_row(ui, "Imgur client id", client_id, change);
            }
            snapper_core::settings::UploadConfig::Http {
                id,
                url,
                method,
                file_field,
                url_json_pointer,
                ..
            } => {
                text_row(ui, "Id", id, change);
                text_row(ui, "URL", url, change);
                text_row(ui, "Method", method, change);
                text_row(ui, "File field", file_field, change);
                text_row(ui, "URL JSON pointer", url_json_pointer, change);
            }
            snapper_core::settings::UploadConfig::External { id, command, .. } => {
                text_row(ui, "Id", id, change);
                text_row(ui, "Command", command, change);
            }
        }
    }
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String, change: &mut FieldChange) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui
            .add(egui::TextEdit::singleline(value).desired_width(280.0))
            .changed()
        {
            change.edited = true;
        }
    });
}

fn workflows_tab(ui: &mut egui::Ui, settings: &mut Settings, change: &mut FieldChange) {
    ui.label("A workflow hotkey runs its steps after a capture. Leave the hotkey empty to start it from the tray only.");
    if ui.button("Add workflow").clicked() {
        settings.workflows.push(snapper_core::settings::Workflow {
            name: format!("Workflow {}", settings.workflows.len() + 1),
            hotkey: String::new(),
            capture: snapper_core::settings::CaptureKind::Region,
            steps: vec![
                snapper_core::settings::PipelineStep::Save,
                snapper_core::settings::PipelineStep::Notify,
            ],
            link_format: snapper_core::settings::LinkFormat::Markdown,
        });
        change.edited = true;
        change.hotkeys = true;
    }
    let mut remove = None;
    for (index, workflow) in settings.workflows.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui
                .add(egui::TextEdit::singleline(&mut workflow.name).desired_width(200.0))
                .changed()
            {
                change.edited = true;
            }
            if ui.button("Remove").clicked() {
                remove = Some(index);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Hotkey");
            if ui
                .add(egui::TextEdit::singleline(&mut workflow.hotkey).desired_width(200.0))
                .changed()
            {
                change.edited = true;
                change.hotkeys = true;
            }
        });
        ui.label(format!("Capture: {}", capture_label(&workflow.capture)));
        ui.horizontal(|ui| {
            if ui.button("Region").clicked() {
                workflow.capture = snapper_core::settings::CaptureKind::Region;
                change.edited = true;
            }
            if ui.button("Window").clicked() {
                workflow.capture = snapper_core::settings::CaptureKind::Window;
                change.edited = true;
            }
            if ui.button("Full screen").clicked() {
                workflow.capture = snapper_core::settings::CaptureKind::FullScreen;
                change.edited = true;
            }
        });
        ui.label("Steps, one per line: redact, annotate, clipboard, save, upload <id>, copy_link, notify");
        let mut text = steps_to_text(&workflow.steps);
        if ui
            .add(
                egui::TextEdit::multiline(&mut text)
                    .desired_width(360.0)
                    .desired_rows(4),
            )
            .changed()
        {
            workflow.steps = text_to_steps(&text);
            change.edited = true;
        }
        ui.horizontal(|ui| {
            ui.label("Link");
            if ui.button("Plain").clicked() {
                workflow.link_format = snapper_core::settings::LinkFormat::Plain;
                change.edited = true;
            }
            if ui.button("Markdown").clicked() {
                workflow.link_format = snapper_core::settings::LinkFormat::Markdown;
                change.edited = true;
            }
            if ui.button("HTML").clicked() {
                workflow.link_format = snapper_core::settings::LinkFormat::Html;
                change.edited = true;
            }
        });
    }
    if let Some(index) = remove {
        settings.workflows.remove(index);
        change.edited = true;
        change.hotkeys = true;
    }
}

fn capture_label(kind: &snapper_core::settings::CaptureKind) -> &'static str {
    match kind {
        snapper_core::settings::CaptureKind::Region => "region",
        snapper_core::settings::CaptureKind::Window => "window",
        snapper_core::settings::CaptureKind::FullScreen => "full screen",
        snapper_core::settings::CaptureKind::Monitor { .. } => "monitor",
    }
}

fn steps_to_text(steps: &[snapper_core::settings::PipelineStep]) -> String {
    steps
        .iter()
        .map(|step| match step {
            snapper_core::settings::PipelineStep::Redact => "redact".to_owned(),
            snapper_core::settings::PipelineStep::Annotate => "annotate".to_owned(),
            snapper_core::settings::PipelineStep::Clipboard => "clipboard".to_owned(),
            snapper_core::settings::PipelineStep::Save => "save".to_owned(),
            snapper_core::settings::PipelineStep::Upload { target } => format!("upload {target}"),
            snapper_core::settings::PipelineStep::CopyLink => "copy_link".to_owned(),
            snapper_core::settings::PipelineStep::Notify => "notify".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_to_steps(text: &str) -> Vec<snapper_core::settings::PipelineStep> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let step = if line == "redact" {
                snapper_core::settings::PipelineStep::Redact
            } else if line == "annotate" {
                snapper_core::settings::PipelineStep::Annotate
            } else if line == "clipboard" {
                snapper_core::settings::PipelineStep::Clipboard
            } else if line == "save" {
                snapper_core::settings::PipelineStep::Save
            } else if let Some(target) = line.strip_prefix("upload ") {
                snapper_core::settings::PipelineStep::Upload {
                    target: target.trim().to_owned(),
                }
            } else if line == "copy_link" {
                snapper_core::settings::PipelineStep::CopyLink
            } else if line == "notify" {
                snapper_core::settings::PipelineStep::Notify
            } else {
                return None;
            };
            Some(step)
        })
        .collect()
}

fn about_tab(ui: &mut egui::Ui, state: &mut UiState) {
    ui.label(format!("Snapper {}", snapper_core::version()));
    if ui.button("Open log folder").clicked() {
        let dir = snapper_core::logging::log_dir();
        state.about_note = match std::fs::create_dir_all(&dir)
            .and_then(|()| opener::open(&dir).map_err(|err| std::io::Error::other(err.to_string())))
        {
            Ok(()) => String::new(),
            Err(err) => format!("Could not open the log folder: {err}"),
        };
    }
    if !state.about_note.is_empty() {
        ui.colored_label(Color32::from_rgb(200, 70, 70), &state.about_note);
    }
    ui.add_space(12.0);
    ui.label("Licence");
    ui.label(LICENCE);
}

fn display_chord(chord: &str) -> String {
    match hotkeys::parse(chord) {
        Ok(hotkey) => hotkeys::format(&hotkey),
        Err(_) => chord.to_owned(),
    }
}

/// Accept `chord` for `field`, or a message the window can show.
///
/// The stored value is the canonical form from [`hotkeys::format`]. A chord
/// another action already uses is rejected. The same chord on `field` is kept.
fn accept_hotkey(
    hotkeys: &HotkeySettings,
    field: HotkeyField,
    chord: &str,
) -> Result<String, String> {
    let hotkey = hotkeys::parse(chord).map_err(|err| err.to_string())?;
    for other in HotkeyField::ALL {
        if other == field {
            continue;
        }
        let Ok(existing) = hotkeys::parse(other.get(hotkeys)) else {
            continue;
        };
        if existing.id() == hotkey.id() {
            return Err(format!(
                "{} is already used by {}",
                hotkeys::format(&hotkey),
                other.label()
            ));
        }
    }
    Ok(hotkeys::format(&hotkey))
}

#[derive(Debug, PartialEq, Eq)]
enum RecordDecision {
    Waiting,
    Cancel,
    Chord(String),
    Unsupported,
}

fn capture_chord(
    ctx: &egui::Context,
    settings: &mut Settings,
    state: &mut UiState,
    change: &mut FieldChange,
) {
    let Some(field) = state.recording else {
        return;
    };
    let decision = ctx.input(|input| {
        let mut decision = RecordDecision::Waiting;
        for event in &input.events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            decision = interpret_key(*key, *modifiers);
            if decision != RecordDecision::Waiting {
                break;
            }
        }
        decision
    });
    match decision {
        RecordDecision::Waiting => {}
        RecordDecision::Cancel => {
            state.recording = None;
            state.hotkey_error.clear();
            state.error_field = None;
        }
        RecordDecision::Unsupported => {
            state.recording = None;
            state.error_field = Some(field);
            state.hotkey_error = "That key cannot be used as a hotkey.".to_owned();
        }
        RecordDecision::Chord(chord) => match accept_hotkey(&settings.hotkeys, field, &chord) {
            Ok(canonical) => {
                field.set(&mut settings.hotkeys, canonical);
                state.recording = None;
                state.hotkey_error.clear();
                state.error_field = None;
                change.edited = true;
                change.hotkeys = true;
            }
            Err(message) => {
                state.recording = None;
                state.error_field = Some(field);
                state.hotkey_error = message;
            }
        },
    }
}

fn interpret_key(key: egui::Key, mods: egui::Modifiers) -> RecordDecision {
    let bare = !mods.alt && !mods.ctrl && !mods.shift && !mods.mac_cmd && !mods.command;
    if key == egui::Key::Escape && bare {
        return RecordDecision::Cancel;
    }
    match chord_from_key(key, mods) {
        Some(chord) => RecordDecision::Chord(chord),
        None => RecordDecision::Unsupported,
    }
}

fn chord_from_key(key: egui::Key, mods: egui::Modifiers) -> Option<String> {
    let token = key_token(key)?;
    let mut parts = Vec::new();
    // `command` mirrors Ctrl on Windows and Linux, and Command on macOS.
    let control = mods.ctrl || (mods.command && !cfg!(target_os = "macos"));
    let super_key = mods.mac_cmd || (cfg!(target_os = "macos") && mods.command);
    if control {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if super_key {
        parts.push("Super");
    }
    parts.push(token);
    Some(parts.join("+"))
}

fn key_token(key: egui::Key) -> Option<&'static str> {
    Some(match key {
        egui::Key::A => "A",
        egui::Key::B => "B",
        egui::Key::C => "C",
        egui::Key::D => "D",
        egui::Key::E => "E",
        egui::Key::F => "F",
        egui::Key::G => "G",
        egui::Key::H => "H",
        egui::Key::I => "I",
        egui::Key::J => "J",
        egui::Key::K => "K",
        egui::Key::L => "L",
        egui::Key::M => "M",
        egui::Key::N => "N",
        egui::Key::O => "O",
        egui::Key::P => "P",
        egui::Key::Q => "Q",
        egui::Key::R => "R",
        egui::Key::S => "S",
        egui::Key::T => "T",
        egui::Key::U => "U",
        egui::Key::V => "V",
        egui::Key::W => "W",
        egui::Key::X => "X",
        egui::Key::Y => "Y",
        egui::Key::Z => "Z",
        egui::Key::Num0 => "0",
        egui::Key::Num1 => "1",
        egui::Key::Num2 => "2",
        egui::Key::Num3 => "3",
        egui::Key::Num4 => "4",
        egui::Key::Num5 => "5",
        egui::Key::Num6 => "6",
        egui::Key::Num7 => "7",
        egui::Key::Num8 => "8",
        egui::Key::Num9 => "9",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        egui::Key::Escape => "Escape",
        egui::Key::Space => "Space",
        egui::Key::Tab => "Tab",
        egui::Key::Enter => "Enter",
        egui::Key::Backspace => "Backspace",
        egui::Key::Delete => "Delete",
        egui::Key::Insert => "Insert",
        egui::Key::ArrowUp => "Up",
        egui::Key::ArrowDown => "Down",
        egui::Key::ArrowLeft => "Left",
        egui::Key::ArrowRight => "Right",
        egui::Key::PageUp => "PageUp",
        egui::Key::PageDown => "PageDown",
        egui::Key::Home => "Home",
        egui::Key::End => "End",
        _ => return None,
    })
}

/// Filename the output tab shows while the template is edited.
///
/// The clock, title and size are fixed so the preview is stable. `{random}`
/// still changes, because [`template::render`] draws it that way.
fn filename_preview(template: &str, format: ImageFormat) -> String {
    let stem = template::render(template, &sample_context());
    format!("{stem}.{}", extension(format))
}

fn sample_context() -> TemplateContext {
    let now = chrono::Local
        .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
        .single()
        .unwrap_or_else(chrono::Local::now);
    TemplateContext {
        now,
        app: Some("Snapper".to_owned()),
        title: Some("Sample".to_owned()),
        width: 1920,
        height: 1080,
        counter: 7,
    }
}

fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
    }
}

fn format_label(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Webp => "WebP",
    }
}

fn is_hex_colour(text: &str) -> bool {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6 || hex.len() == 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn colour_to_color32(text: &str) -> Color32 {
    let rgba = parse_colour(text);
    Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a)
}

fn color32_to_hex(colour: Color32) -> String {
    let [r, g, b, a] = colour.to_srgba_unmultiplied();
    format_colour(Rgba { r, g, b, a })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_hotkeys_are_rejected_with_a_message() {
        let hotkeys = HotkeySettings::default();
        let err = accept_hotkey(&hotkeys, HotkeyField::Record, "Nope").unwrap_err();
        assert!(err.contains("Nope"), "{err}");
        assert!(accept_hotkey(&hotkeys, HotkeyField::Region, "").is_err());
        assert!(accept_hotkey(&hotkeys, HotkeyField::Region, "Ctrl+Shift").is_err());
    }

    #[test]
    fn a_duplicate_hotkey_names_the_other_action() {
        let hotkeys = HotkeySettings::default();
        let err = accept_hotkey(&hotkeys, HotkeyField::Window, "printscreen").unwrap_err();
        assert!(err.contains("PrintScreen"), "{err}");
        assert!(err.contains("Capture region"), "{err}");
        let same = accept_hotkey(&hotkeys, HotkeyField::Region, "PrintScreen").unwrap();
        assert_eq!(same, "PrintScreen");
    }

    #[test]
    fn an_accepted_hotkey_is_canonical_and_persists() {
        let dir = std::env::temp_dir().join(format!(
            "snapper-settings-ui-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let mut settings = Settings::default();
        let before_extra = settings.extra.clone();
        settings.hotkeys.record =
            accept_hotkey(&settings.hotkeys, HotkeyField::Record, "ctrl+shift+p").unwrap();
        settings.output.filename_template = "bug_{date}".to_owned();
        settings.output.format = ImageFormat::Jpeg;
        settings.output.jpeg_quality = 75;
        settings.output.copy_to_clipboard = false;
        settings.output.open_editor = true;
        settings.editor.stroke_color = "#112233FF".to_owned();
        settings.editor.stroke_width = 2.5;
        settings.start_at_login = true;
        settings.save_to(&dir).unwrap();
        let loaded = Settings::load_from(&dir).unwrap();
        let _ = std::fs::remove_file(&dir);
        assert_eq!(loaded.hotkeys.record, "Ctrl+Shift+P");
        assert_eq!(loaded.output.filename_template, "bug_{date}");
        assert_eq!(loaded.output.format, ImageFormat::Jpeg);
        assert_eq!(loaded.output.jpeg_quality, 75);
        assert!(!loaded.output.copy_to_clipboard);
        assert!(loaded.output.open_editor);
        assert_eq!(loaded.editor.stroke_color, "#112233FF");
        assert_eq!(loaded.editor.stroke_width, 2.5);
        assert!(loaded.start_at_login);
        assert_eq!(loaded.extra, before_extra);
        assert_eq!(loaded, settings);
    }

    #[test]
    fn template_preview_updates_as_the_template_changes() {
        let dated = filename_preview("Snapper_{date}_{time}", ImageFormat::Png);
        let sized = filename_preview("Bug_{width}x{height}", ImageFormat::Jpeg);
        let named = filename_preview("{app}_{title}_{counter}", ImageFormat::Webp);
        assert_eq!(dated, "Snapper_2026-01-02_03-04-05.png");
        assert_eq!(sized, "Bug_1920x1080.jpg");
        assert_eq!(named, "Snapper_Sample_7.webp");
        assert_ne!(dated, filename_preview("Other_{date}", ImageFormat::Png));
    }

    #[test]
    fn recorder_maps_chords_and_rejects_unknown_keys() {
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..egui::Modifiers::default()
        };
        // On macOS, egui's command modifier is Command and is recorded as Super.
        let expected = if cfg!(target_os = "macos") {
            "Ctrl+Shift+Super+R"
        } else {
            "Ctrl+Shift+R"
        };
        assert_eq!(
            interpret_key(egui::Key::R, ctrl_shift),
            RecordDecision::Chord(expected.to_owned())
        );
        assert_eq!(
            interpret_key(egui::Key::Escape, egui::Modifiers::default()),
            RecordDecision::Cancel
        );
        assert_eq!(
            interpret_key(egui::Key::Copy, egui::Modifiers::default()),
            RecordDecision::Unsupported
        );
    }

    #[test]
    fn save_is_debounced_for_half_a_second() {
        let start = Instant::now();
        let pending = PendingSave {
            at: start,
            hotkeys: false,
            login: false,
        };
        assert!(!save_is_due(&pending, start + Duration::from_millis(499)));
        assert!(save_is_due(&pending, start + Duration::from_millis(500)));
    }

    #[test]
    fn later_edits_reset_the_debounce_and_keep_flags() {
        let start = Instant::now();
        let mut pending = None;
        note_edit(&mut pending, true, false, start);
        let later = start + Duration::from_millis(400);
        note_edit(&mut pending, false, true, later);
        let pending = pending.expect("pending save");
        assert!(pending.hotkeys);
        assert!(pending.login);
        assert!(!save_is_due(&pending, later + Duration::from_millis(499)));
        assert!(save_is_due(&pending, later + Duration::from_millis(500)));
    }

    #[test]
    fn show_without_input_does_not_edit_or_drop_fields() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        settings.output.filename_template = "kept_{date}".to_owned();
        let before = settings.clone();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut change = SettingsChange::Edited {
            hotkeys: true,
            login: true,
        };
        let _ = ctx.run(input, |ctx| {
            change = show(ctx, &mut settings);
        });
        assert_eq!(change, SettingsChange::Unchanged);
        assert_eq!(settings, before);
    }

    #[test]
    fn about_names_the_mit_licence() {
        assert!(LICENCE.contains("MIT"));
        assert!(LICENCE.contains("Licence"));
        assert!(!snapper_core::version().is_empty());
    }
}
