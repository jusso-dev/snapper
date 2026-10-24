//! Named after-capture workflows.
//!
//! A workflow that annotates runs the steps before Annotate on the capture,
//! then opens the editor with the remaining steps. Those steps run when the
//! user saves. Redaction before the editor is also inserted as reviewable
//! pixelate items.

use snapper_capture::default_capturer;
use snapper_core::settings::{CaptureKind, LinkFormat, PipelineStep, Workflow};
use snapper_core::{Image, Rect, Settings};
use snapper_editor::Document;

use crate::output::{self, OutputStep};
use crate::overlay::{self, OverlayResult};
use crate::redact;

/// Map workflow steps onto output steps. Annotate is not an output step.
pub fn to_output_steps(workflow: &Workflow) -> Vec<OutputStep> {
    workflow
        .steps
        .iter()
        .filter_map(|step| step_to_output(step, workflow.link_format))
        .collect()
}

fn step_to_output(step: &PipelineStep, link: LinkFormat) -> Option<OutputStep> {
    Some(match step {
        PipelineStep::Redact => OutputStep::Redact,
        PipelineStep::Annotate => return None,
        PipelineStep::Clipboard => OutputStep::Clipboard,
        PipelineStep::Save => OutputStep::Save,
        PipelineStep::Upload { target } => OutputStep::Upload(target.clone()),
        PipelineStep::CopyLink => OutputStep::CopyLink(link),
        PipelineStep::Notify => OutputStep::Notify,
    })
}

/// Capture and run `workflow`.
pub fn run(workflow: &Workflow, settings: &Settings, last_region: &mut Option<Rect>) {
    let Some(image) = capture(workflow, last_region) else {
        return;
    };
    if let Some(index) = workflow
        .steps
        .iter()
        .position(|step| matches!(step, PipelineStep::Annotate))
    {
        open_editor(workflow, settings, image, index);
        return;
    }
    let steps = to_output_steps(workflow);
    let ctx = output::context_for(&image);
    output::run_with(&steps, &image, &ctx, settings);
}

fn open_editor(workflow: &Workflow, settings: &Settings, image: Image, annotate_at: usize) {
    let before = &workflow.steps[..annotate_at];
    // Steps such as Save or Clipboard before Annotate run on the capture now.
    // A Redact among them is baked in for those steps and also shown in the
    // editor as reviewable items.
    if before
        .iter()
        .any(|step| !matches!(step, PipelineStep::Redact))
    {
        let steps = before
            .iter()
            .filter_map(|step| step_to_output(step, workflow.link_format))
            .collect::<Vec<_>>();
        let ctx = output::context_for(&image);
        output::run_with(&steps, &image, &ctx, settings);
    }
    let mut doc = Document::new(image);
    if before
        .iter()
        .any(|step| matches!(step, PipelineStep::Redact))
    {
        match snapper_ocr::default_engine().and_then(|engine| engine.recognise(&doc.base, "en")) {
            Ok(words) => {
                for item in redact::items_for(&words, 1) {
                    doc.apply(snapper_editor::Edit::Add(item));
                }
            }
            Err(err) => output::notify("Snapper", &format!("redaction skipped: {err}")),
        }
    }
    let after = workflow.steps[annotate_at + 1..]
        .iter()
        .filter_map(|step| step_to_output(step, workflow.link_format))
        .collect();
    crate::editor::run_editor_after(doc, settings.clone(), after);
}

fn capture(workflow: &Workflow, last_region: &mut Option<Rect>) -> Option<Image> {
    match &workflow.capture {
        CaptureKind::FullScreen => capture_full(),
        CaptureKind::Monitor { id } => match default_capturer().capture_monitor(*id) {
            Ok(image) => Some(image),
            Err(err) => {
                output::notify("Snapper", &format!("capture failed: {err}"));
                None
            }
        },
        CaptureKind::Window | CaptureKind::Region => capture_overlay(last_region),
    }
}

fn capture_full() -> Option<Image> {
    let capturer = default_capturer();
    let image = match capturer.monitors() {
        Ok(monitors) => {
            let primary = monitors
                .iter()
                .find(|monitor| monitor.is_primary)
                .or(monitors.first());
            match primary {
                Some(monitor) => capturer.capture_monitor(monitor.id),
                None => capturer.capture_all(),
            }
        }
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return None;
        }
    };
    match image {
        Ok(image) => Some(image),
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            None
        }
    }
}

fn capture_overlay(last_region: &mut Option<Rect>) -> Option<Image> {
    let capturer = default_capturer();
    let monitors = match capturer.monitors() {
        Ok(monitors) if !monitors.is_empty() => monitors,
        Ok(_) => {
            output::notify("Snapper", "capture failed: no monitors found");
            return None;
        }
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return None;
        }
    };
    let windows = match capturer.windows() {
        Ok(windows) => windows,
        Err(snapper_capture::CaptureError::Unsupported(_)) => Vec::new(),
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return None;
        }
    };
    let frozen = match capturer.capture_all() {
        Ok(image) => image,
        Err(err) => {
            output::notify("Snapper", &format!("capture failed: {err}"));
            return None;
        }
    };
    let source = Image {
        width: frozen.width,
        height: frozen.height,
        scale_factor: frozen.scale_factor,
        rgba: frozen.rgba.clone(),
    };
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
    match overlay::run_overlay(frozen, monitors, windows) {
        OverlayResult::Region(rect) => {
            *last_region = Some(rect);
            match snapper_capture::crop(
                &source,
                Rect {
                    x: rect.x.saturating_sub(origin_x),
                    y: rect.y.saturating_sub(origin_y),
                    width: rect.width,
                    height: rect.height,
                },
            ) {
                Ok(image) => Some(image),
                Err(err) => {
                    output::notify("Snapper", &err.to_string());
                    None
                }
            }
        }
        OverlayResult::Window(id) => match capturer.capture_window(id) {
            Ok(image) => Some(image),
            Err(err) => {
                output::notify("Snapper", &format!("capture failed: {err}"));
                None
            }
        },
        OverlayResult::Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_keep_order_and_the_link_format() {
        let workflow = Workflow {
            name: "Bug report".to_owned(),
            hotkey: String::new(),
            capture: CaptureKind::Region,
            steps: vec![
                PipelineStep::Redact,
                PipelineStep::Annotate,
                PipelineStep::Save,
                PipelineStep::Upload {
                    target: "r2".to_owned(),
                },
                PipelineStep::CopyLink,
                PipelineStep::Notify,
            ],
            link_format: LinkFormat::Html,
        };
        assert_eq!(
            to_output_steps(&workflow),
            vec![
                OutputStep::Redact,
                OutputStep::Save,
                OutputStep::Upload("r2".to_owned()),
                OutputStep::CopyLink(LinkFormat::Html),
                OutputStep::Notify,
            ]
        );
    }
}
