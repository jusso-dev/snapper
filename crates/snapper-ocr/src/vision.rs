//! macOS Vision text recognition.
//!
//! `VNRecognizeTextRequest` runs at the accurate level. A missing Vision
//! result is a backend error; this module does not send the image anywhere.

use std::io::Write;

use objc2::rc::Retained;
use objc2::{AnyThread, ClassType};
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedTextObservation, VNRequest,
    VNRequestTextRecognitionLevel,
};
use snapper_core::Rect;

use crate::{encode_png, OcrEngine, OcrError, OcrWord};

pub struct VisionEngine;

impl OcrEngine for VisionEngine {
    fn recognise(&self, img: &snapper_core::Image, lang: &str) -> Result<Vec<OcrWord>, OcrError> {
        let png = encode_png(img)?;
        let path = write_temp_png(&png)?;
        let words = recognise_file(&path, img.width, img.height, lang);
        let _ = std::fs::remove_file(&path);
        words
    }
}

fn write_temp_png(bytes: &[u8]) -> Result<std::path::PathBuf, OcrError> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "snapper-ocr-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let mut file =
        std::fs::File::create(&path).map_err(|err| OcrError::Backend(err.to_string()))?;
    file.write_all(bytes)
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    Ok(path)
}

fn recognise_file(
    path: &std::path::Path,
    width: u32,
    height: u32,
    lang: &str,
) -> Result<Vec<OcrWord>, OcrError> {
    let text = path.to_string_lossy();
    unsafe { recognise_path(&text, width, height, lang) }
}

unsafe fn recognise_path(
    path: &str,
    width: u32,
    height: u32,
    lang: &str,
) -> Result<Vec<OcrWord>, OcrError> {
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    if !lang.is_empty() {
        let language = NSString::from_str(lang);
        let languages = NSArray::from_retained_slice(&[language]);
        request.setRecognitionLanguages(&languages);
    }
    request.setUsesLanguageCorrection(true);

    let ns_path = NSString::from_str(path);
    let url = NSURL::fileURLWithPath(&ns_path);
    let options: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> = NSDictionary::new();
    let handler =
        VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &options);
    let as_request: &VNRequest = request.as_super().as_super();
    let requests = NSArray::from_slice(&[as_request]);
    if let Err(err) = handler.performRequests_error(&requests) {
        return Err(OcrError::Backend(err.localizedDescription().to_string()));
    }

    let Some(results) = request.results() else {
        return Ok(Vec::new());
    };
    let mut words = Vec::new();
    for observation in results.iter() {
        let observation: Retained<VNRecognizedTextObservation> = observation;
        let candidates = observation.topCandidates(1);
        let Some(candidate) = candidates.iter().next() else {
            continue;
        };
        let text = candidate.string().to_string();
        if text.trim().is_empty() {
            continue;
        }
        let bounds = observation.boundingBox();
        // Vision boxes are normalised, origin bottom-left.
        let word_width = (bounds.size.width.max(0.0) * f64::from(width)).round() as u32;
        let word_height = (bounds.size.height.max(0.0) * f64::from(height)).round() as u32;
        let x = (bounds.origin.x * f64::from(width)).round() as i32;
        let y = ((1.0 - bounds.origin.y - bounds.size.height).max(0.0) * f64::from(height)).round()
            as i32;
        words.push(OcrWord {
            text,
            rect: Rect {
                x,
                y,
                width: word_width.max(1),
                height: word_height.max(1),
            },
            confidence: candidate.confidence(),
        });
    }
    Ok(words)
}
