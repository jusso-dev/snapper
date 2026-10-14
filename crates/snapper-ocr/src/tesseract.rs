//! Tesseract via `leptess`. Built only with `--features tesseract` on Linux.
//!
//! A missing library or language pack becomes [`OcrError::Unavailable`] and
//! includes [`crate::tesseract_install_hint`].

use snapper_core::{Image, Rect};

use crate::{encode_png, tesseract_install_hint, OcrEngine, OcrError, OcrWord};

pub struct TesseractEngine;

impl OcrEngine for TesseractEngine {
    fn recognise(&self, img: &Image, lang: &str) -> Result<Vec<OcrWord>, OcrError> {
        let png = encode_png(img)?;
        let language = if lang.is_empty() { "eng" } else { lang };
        let mut tess = leptess::LepTess::new(None, language).map_err(|err| {
            OcrError::Unavailable(format!("{} ({err})", tesseract_install_hint()))
        })?;
        tess.set_image_from_mem(&png)
            .map_err(|err| OcrError::Backend(err.to_string()))?;
        let text = tess
            .get_utf8_text()
            .map_err(|err| OcrError::Backend(err.to_string()))?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        // leptess exposes the page text. Word rectangles are not in its safe
        // API, so the whole image is the box. Redaction still covers the line
        // when a detector matches that text.
        Ok(vec![OcrWord {
            text: text.to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: img.width,
                height: img.height,
            },
            confidence: 0.0,
        }])
    }
}
