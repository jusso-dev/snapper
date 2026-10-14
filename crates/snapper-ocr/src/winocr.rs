//! Windows.Media.Ocr.
//!
//! The bitmap is BGRA8, which is what `SoftwareBitmap` expects. This module
//! is compiled only on Windows.

use snapper_core::{Image, Rect};
use windows::core::HSTRING;
use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinOcr;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

use crate::{encode_png, OcrEngine, OcrError, OcrWord};

pub struct WindowsEngine;

impl OcrEngine for WindowsEngine {
    fn recognise(&self, img: &Image, _lang: &str) -> Result<Vec<OcrWord>, OcrError> {
        let png = encode_png(img)?;
        let bitmap = software_bitmap(&png)?;
        let engine = WinOcr::TryCreateFromUserProfileLanguages().map_err(|err| {
            OcrError::Unavailable(format!(
                "Windows OCR is not available ({err}). Install a language pack in Settings, Time & language, Language & region."
            ))
        })?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| OcrError::Backend(err.to_string()))?
            .get()
            .map_err(|err| OcrError::Backend(err.to_string()))?;
        let mut words = Vec::new();
        let lines = result
            .Lines()
            .map_err(|err| OcrError::Backend(err.to_string()))?;
        for line in lines {
            let line_words = line
                .Words()
                .map_err(|err| OcrError::Backend(err.to_string()))?;
            for word in line_words {
                let text = word
                    .Text()
                    .map_err(|err| OcrError::Backend(err.to_string()))?
                    .to_string();
                let bounds = word
                    .BoundingRect()
                    .map_err(|err| OcrError::Backend(err.to_string()))?;
                if text.trim().is_empty() {
                    continue;
                }
                words.push(OcrWord {
                    text,
                    rect: Rect {
                        x: bounds.X as i32,
                        y: bounds.Y as i32,
                        width: bounds.Width.max(0.0) as u32,
                        height: bounds.Height.max(0.0) as u32,
                    },
                    confidence: 1.0,
                });
            }
        }
        let _ = HSTRING::new();
        Ok(words)
    }
}

fn software_bitmap(png: &[u8]) -> Result<SoftwareBitmap, OcrError> {
    let stream =
        InMemoryRandomAccessStream::new().map_err(|err| OcrError::Backend(err.to_string()))?;
    let writer =
        DataWriter::CreateDataWriter(&stream).map_err(|err| OcrError::Backend(err.to_string()))?;
    writer
        .WriteBytes(png)
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    writer
        .StoreAsync()
        .map_err(|err| OcrError::Backend(err.to_string()))?
        .get()
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    writer
        .DetachStream()
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    stream
        .Seek(0)
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|err| OcrError::Backend(err.to_string()))?
        .get()
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|err| OcrError::Backend(err.to_string()))?
        .get()
        .map_err(|err| OcrError::Backend(err.to_string()))?;
    SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8)
        .map_err(|err| OcrError::Backend(err.to_string()))
}
