//! Editor actions that hand a flattened image to the output pipeline.

use snapper_core::{Image, Settings};
use snapper_editor::Document;

use crate::output::{self, OutputResult};

/// Bottom-bar action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRequest {
    Copy,
    Save,
    Upload,
}

/// What the editor should show after an output action.
pub struct EditorOutput {
    pub note: String,
    /// True when a file was written, so the editor can clear its dirty flag.
    pub saved: bool,
}

/// Flatten `doc`, including its crop, for an output action.
pub fn flattened(doc: &Document) -> Image {
    snapper_editor::render::flatten(doc)
}

/// Copy, save, or note that upload is not available yet.
pub fn perform(request: OutputRequest, doc: &Document, settings: &Settings) -> EditorOutput {
    let image = flattened(doc);
    match request {
        OutputRequest::Copy => match output::copy_image(&image) {
            Ok(()) => EditorOutput {
                note: format!("Copied {}x{}", image.width, image.height),
                saved: false,
            },
            Err(err) => {
                tracing::error!(error = %err, "copy failed");
                EditorOutput {
                    note: format!("Copy failed: {err}"),
                    saved: false,
                }
            }
        },
        OutputRequest::Save => {
            let ctx = output::context_for(&image);
            let result = output::run_with(&save_steps(settings), &image, &ctx, settings);
            saved_note(&result)
        }
        OutputRequest::Upload => {
            let Some(target) = settings.uploads.first() else {
                return EditorOutput {
                    note: "No upload target is configured".to_owned(),
                    saved: false,
                };
            };
            let steps = vec![
                crate::output::OutputStep::Save,
                crate::output::OutputStep::Upload(target.id().to_owned()),
                crate::output::OutputStep::CopyLink(snapper_core::settings::LinkFormat::Plain),
                crate::output::OutputStep::Notify,
            ];
            let ctx = output::context_for(&image);
            let result = output::run_with(&steps, &image, &ctx, settings);
            if let Some(url) = &result.url {
                EditorOutput {
                    note: format!("Uploaded {url}"),
                    saved: result.saved.is_some(),
                }
            } else {
                saved_note(&result)
            }
        }
    }
}

/// The default output steps without automatic redaction. The user has already
/// reviewed the document, so redaction boxes they removed must stay removed.
fn save_steps(settings: &Settings) -> Vec<output::OutputStep> {
    output::OutputPipeline::from_settings(&settings.output, None)
        .steps()
        .iter()
        .filter(|step| !matches!(step, output::OutputStep::Redact))
        .cloned()
        .collect()
}

fn saved_note(result: &OutputResult) -> EditorOutput {
    match &result.saved {
        Some(path) => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("screenshot");
            EditorOutput {
                note: format!("Saved {name}"),
                saved: true,
            }
        }
        None => EditorOutput {
            note: "Not saved".to_owned(),
            saved: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_does_not_redact_again() {
        let mut settings = Settings::default();
        settings.output.auto_redact = true;
        let steps = save_steps(&settings);
        assert!(!steps.contains(&output::OutputStep::Redact));
        assert!(steps.contains(&output::OutputStep::Save));
    }
}
