//! Optical character recognition.
//!
//! Windows uses `Windows.Media.Ocr`, macOS uses Vision, and Linux uses
//! Tesseract when Snapper is built with `--features tesseract`. Recognition
//! stays on the machine. Redaction in [`redact`] only sees the words returned
//! here.

mod lines;
mod redact;

#[cfg(all(target_os = "linux", feature = "tesseract"))]
mod tesseract;
#[cfg(target_os = "macos")]
mod vision;
#[cfg(target_os = "windows")]
mod winocr;

pub use lines::join_lines;
pub use redact::{detect, shannon_entropy, Finding};

use image::ImageEncoder;
use snapper_core::{Image, Rect};

/// One recognised word and where it sits in the image.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
    pub confidence: f32,
}

/// Failure from an OCR engine.
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// The platform engine is not installed or this build left it out.
    #[error("{0}")]
    Unavailable(String),
    /// The engine ran and failed.
    #[error("ocr failed: {0}")]
    Backend(String),
    /// The image had no pixels to read.
    #[error("image has no pixels")]
    Empty,
}

/// A platform text recogniser.
pub trait OcrEngine {
    fn recognise(&self, img: &Image, lang: &str) -> Result<Vec<OcrWord>, OcrError>;
}

/// Message shown when Tesseract is not built in or cannot be loaded.
pub fn tesseract_install_hint() -> &'static str {
    "Tesseract is not available. On Linux, install it (for example `sudo apt install tesseract-ocr`) and rebuild Snapper with `--features tesseract`."
}

/// The engine for this operating system.
///
/// On Linux without the `tesseract` feature this returns [`OcrError::Unavailable`]
/// with [`tesseract_install_hint`].
pub fn default_engine() -> Result<Box<dyn OcrEngine>, OcrError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(vision::VisionEngine))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(winocr::WindowsEngine))
    }
    #[cfg(all(target_os = "linux", feature = "tesseract"))]
    {
        Ok(Box::new(tesseract::TesseractEngine))
    }
    #[cfg(all(target_os = "linux", not(feature = "tesseract")))]
    {
        Err(OcrError::Unavailable(tesseract_install_hint().to_owned()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(OcrError::Unavailable(
            "OCR is not implemented on this operating system".to_owned(),
        ))
    }
}

pub(crate) fn image_ok(image: &Image) -> Result<(), OcrError> {
    let expected = (image.width as usize)
        .saturating_mul(image.height as usize)
        .saturating_mul(4);
    if image.width == 0 || image.height == 0 || image.rgba.len() != expected {
        Err(OcrError::Empty)
    } else {
        Ok(())
    }
}

pub(crate) fn encode_png(image: &Image) -> Result<Vec<u8>, OcrError> {
    image_ok(image)?;
    let mut bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    image::codecs::png::PngEncoder::new(&mut cursor)
        .write_image(
            &image.rgba,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tesseract_hint_names_the_package() {
        let hint = tesseract_install_hint();
        assert!(hint.contains("tesseract"));
        assert!(hint.contains("tesseract-ocr"));
        assert!(hint.contains("--features tesseract"));
    }

    #[test]
    fn empty_image_is_rejected() {
        let image = Image {
            width: 0,
            height: 0,
            scale_factor: 1.0,
            rgba: Vec::new(),
        };
        assert!(matches!(image_ok(&image), Err(OcrError::Empty)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_without_tesseract_explains_how_to_install_it() {
        if cfg!(feature = "tesseract") {
            return;
        }
        let err = default_engine().expect_err("tesseract feature is off");
        let message = err.to_string();
        assert!(message.contains("tesseract-ocr"), "{message}");
    }
}
