//! Editor actions that hand a flattened image to the output pipeline.
//!
//! Copy, save and upload themselves land in the output issue. This module
//! flattens the document and records the request.

use snapper_core::Image;
use snapper_editor::Document;

/// Bottom-bar action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRequest {
    Copy,
    Save,
    Upload,
}

/// Flatten `doc`, including its crop, for an output action.
pub fn flattened(doc: &Document) -> Image {
    snapper_editor::render::flatten(doc)
}

/// Record that the editor asked the output pipeline to run.
pub fn dispatch(request: OutputRequest, image: &Image) {
    tracing::info!(
        action = ?request,
        width = image.width,
        height = image.height,
        "editor output requested"
    );
}
