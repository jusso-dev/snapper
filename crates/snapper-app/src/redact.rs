//! Turn OCR words into pixelate items, then into a flattened image.
//!
//! Recognition stays on this machine. A failure leaves the original image
//! unchanged so a saved file is still written.

use snapper_core::{Image, Rect};
use snapper_editor::{Document, Edit, Item, RectF, Shape, Style};
use snapper_ocr::{detect, OcrWord};

/// Pixelate blocks inserted for one finding.
pub const REDACT_BLOCK: u32 = 12;

/// Build reviewable pixelate items. The caller inserts them into a document.
pub fn items_for(words: &[OcrWord], next_id: u64) -> Vec<Item> {
    detect(words)
        .into_iter()
        .enumerate()
        .map(|(index, (_, rect))| Item {
            id: next_id + index as u64,
            shape: Shape::Pixelate {
                rect: rect_f(rect),
                block: REDACT_BLOCK,
            },
            style: Style::default(),
            auto: true,
        })
        .collect()
}

/// Flatten `words` onto a copy of `image`.
pub fn apply_to_image(image: &Image, words: &[OcrWord]) -> Image {
    let items = items_for(words, 1);
    if items.is_empty() {
        return image.clone();
    }
    let mut doc = Document::new(image.clone());
    for item in items {
        doc.apply(Edit::Add(item));
    }
    snapper_editor::flatten(&doc)
}

/// Run the platform OCR engine and pixelate what it finds.
pub fn recognise_and_apply(image: &Image) -> Result<Image, String> {
    let engine = snapper_ocr::default_engine().map_err(|err| err.to_string())?;
    let words = engine
        .recognise(image, "en")
        .map_err(|err| err.to_string())?;
    Ok(apply_to_image(image, &words))
}

fn rect_f(rect: Rect) -> RectF {
    RectF {
        x: rect.x as f32,
        y: rect.y as f32,
        width: rect.width as f32,
        height: rect.height as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            scale_factor: 1.0,
            rgba: [20, 40, 60, 255].repeat((width * height) as usize),
        }
    }

    fn word(text: &str) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 16,
            },
            confidence: 0.9,
        }
    }

    #[test]
    fn an_aws_key_is_pixelated() {
        let mut image = solid(32, 16);
        // A flat colour averages to itself, so one pixel has to differ.
        image.rgba[0] = 255;
        image.rgba[1] = 0;
        image.rgba[2] = 0;
        let out = apply_to_image(&image, &[word("AKIAIOSFODNN7EXAMPLE")]);
        assert_eq!((out.width, out.height), (32, 16));
        assert_ne!(out.rgba, image.rgba);
    }

    #[test]
    fn ordinary_words_are_left_alone() {
        let image = solid(32, 16);
        let out = apply_to_image(&image, &[word("hello")]);
        assert_eq!(out.rgba, image.rgba);
        assert!(items_for(&[word("hello")], 1).is_empty());
    }

    #[test]
    fn auto_items_are_marked_for_review() {
        let items = items_for(&[word("user@example.com")], 4);
        assert_eq!(items.len(), 1);
        assert!(items[0].auto);
        assert_eq!(items[0].id, 4);
        assert!(matches!(
            items[0].shape,
            Shape::Pixelate {
                block: REDACT_BLOCK,
                ..
            }
        ));
    }
}
