//! Annotation document model.
//!
//! A [`Document`] is a base image plus ordered items. [`flatten`](render::flatten)
//! paints that model to RGBA with tiny-skia and the bundled Inter font, so the
//! same input produces the same pixels on every platform.
//!
//! Inter is stored in `assets/` under the SIL Open Font License (`assets/OFL.txt`).

pub mod render;

pub use render::flatten;

use serde::{Deserialize, Serialize};
use snapper_core::{Image, Rect};

/// A point in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// An axis-aligned rectangle in image pixels. Width and height may be negative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectF {
    /// Origin at the top-left, with positive width and height.
    pub fn normalised(self) -> Self {
        let x = self.x.min(self.x + self.width);
        let y = self.y.min(self.y + self.height);
        Self {
            x,
            y,
            width: self.width.abs(),
            height: self.height.abs(),
        }
    }
}

/// Straight RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// How a shape is stroked and filled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub stroke: Rgba,
    pub fill: Option<Rgba>,
    pub width: f32,
    pub shadow: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Rgba {
                r: 0xE2,
                g: 0x3D,
                b: 0x3D,
                a: 0xFF,
            },
            fill: None,
            width: 4.0,
            shadow: false,
        }
    }
}

/// One annotation. Coordinates are image pixels, origin top-left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Arrow { from: Point, to: Point },
    Line { from: Point, to: Point },
    Rect { rect: RectF },
    Ellipse { rect: RectF },
    Highlight { rect: RectF },
    Text { at: Point, text: String, size: f32 },
    Step { at: Point, n: u32 },
    Blur { rect: RectF, radius: f32 },
    Pixelate { rect: RectF, block: u32 },
}

/// A shape with an identity and a style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub shape: Shape,
    pub style: Style,
}

/// One undoable change. `apply` performs it; `undo` performs the inverse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    Add(Item),
    Remove(Item),
    Modify {
        before: Item,
        after: Item,
    },
    Crop {
        #[serde(with = "rect_serde")]
        before: Option<Rect>,
        #[serde(with = "rect_serde")]
        after: Option<Rect>,
    },
}

/// Base image, annotations, optional crop, and the undo stacks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(with = "image_serde")]
    pub base: Image,
    pub items: Vec<Item>,
    #[serde(with = "rect_serde")]
    pub crop: Option<Rect>,
    #[serde(skip, default)]
    pub undo: Vec<Edit>,
    #[serde(skip, default)]
    pub redo: Vec<Edit>,
}

impl Document {
    pub fn new(base: Image) -> Self {
        Self {
            base,
            items: Vec::new(),
            crop: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Perform `edit`, push it onto the undo stack, and drop the redo stack.
    pub fn apply(&mut self, edit: Edit) {
        self.perform(&edit);
        self.undo.push(edit);
        self.redo.clear();
    }

    /// Reverse the latest edit. Returns false when the undo stack is empty.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        let inverse = invert(edit.clone());
        self.perform(&inverse);
        self.redo.push(edit);
        true
    }

    /// Re-apply the latest undone edit. Returns false when the redo stack is empty.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        self.perform(&edit);
        self.undo.push(edit);
        true
    }

    /// One more than the highest step number, or 1 when there are none.
    pub fn next_step_number(&self) -> u32 {
        self.items
            .iter()
            .filter_map(|item| match item.shape {
                Shape::Step { n, .. } => Some(n),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Top-most item under `point`, if any.
    pub fn hit_test(&self, point: Point) -> Option<u64> {
        let scale = positive_scale(self.base.scale_factor);
        self.items
            .iter()
            .rev()
            .find(|item| hit_item(item, point, scale))
            .map(|item| item.id)
    }

    fn perform(&mut self, edit: &Edit) {
        match edit {
            Edit::Add(item) => self.items.push(item.clone()),
            Edit::Remove(item) => self.items.retain(|existing| existing.id != item.id),
            Edit::Modify { before, after } => {
                if let Some(slot) = self.items.iter_mut().find(|item| item.id == before.id) {
                    *slot = after.clone();
                } else {
                    self.items.push(after.clone());
                }
            }
            Edit::Crop { after, .. } => self.crop = *after,
        }
    }
}

fn invert(edit: Edit) -> Edit {
    match edit {
        Edit::Add(item) => Edit::Remove(item),
        Edit::Remove(item) => Edit::Add(item),
        Edit::Modify { before, after } => Edit::Modify {
            before: after,
            after: before,
        },
        Edit::Crop { before, after } => Edit::Crop {
            before: after,
            after: before,
        },
    }
}

fn positive_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn hit_item(item: &Item, point: Point, scale: f32) -> bool {
    let pad = (item.style.width.abs() * 0.5).max(0.0);
    match &item.shape {
        Shape::Arrow { from, to } | Shape::Line { from, to } => {
            let head = match &item.shape {
                Shape::Arrow { .. } => render::arrow_head_length(item.style.width) * 0.5,
                _ => 0.0,
            };
            distance_to_segment(point, *from, *to) <= pad.max(3.0).max(head)
        }
        Shape::Rect { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => contains_rect(*rect, point, pad),
        Shape::Ellipse { rect } => contains_ellipse(*rect, point, pad),
        Shape::Text { at, text, size } => {
            let width = text.chars().count() as f32 * size.abs() * 0.6;
            let height = size.abs() * text.lines().count().max(1) as f32;
            contains_rect(
                RectF {
                    x: at.x,
                    y: at.y,
                    width,
                    height,
                },
                point,
                0.0,
            )
        }
        Shape::Step { at, .. } => {
            let radius = 14.0 * scale;
            let dx = point.x - at.x;
            let dy = point.y - at.y;
            dx * dx + dy * dy <= radius * radius
        }
    }
}

fn contains_rect(rect: RectF, point: Point, pad: f32) -> bool {
    let rect = rect.normalised();
    point.x >= rect.x - pad
        && point.y >= rect.y - pad
        && point.x <= rect.x + rect.width + pad
        && point.y <= rect.y + rect.height + pad
}

fn contains_ellipse(rect: RectF, point: Point, pad: f32) -> bool {
    let rect = rect.normalised();
    let rx = rect.width * 0.5 + pad;
    let ry = rect.height * 0.5 + pad;
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let dx = point.x - (rect.x + rect.width * 0.5);
    let dy = point.y - (rect.y + rect.height * 0.5);
    (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry) <= 1.0
}

fn distance_to_segment(point: Point, from: Point, to: Point) -> f32 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return (point.x - from.x).hypot(point.y - from.y);
    }
    let t = (((point.x - from.x) * dx + (point.y - from.y) * dy) / len2).clamp(0.0, 1.0);
    let x = from.x + t * dx;
    let y = from.y + t * dy;
    (point.x - x).hypot(point.y - y)
}

mod image_serde {
    use super::Image;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct ImageDto {
        width: u32,
        height: u32,
        scale_factor: f32,
        rgba: Vec<u8>,
    }

    pub fn serialize<S>(image: &Image, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ImageDto {
            width: image.width,
            height: image.height,
            scale_factor: image.scale_factor,
            rgba: image.rgba.clone(),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Image, D::Error>
    where
        D: Deserializer<'de>,
    {
        let image = ImageDto::deserialize(deserializer)?;
        Ok(Image {
            width: image.width,
            height: image.height,
            scale_factor: image.scale_factor,
            rgba: image.rgba,
        })
    }
}

mod rect_serde {
    use super::Rect;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct RectDto {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    pub fn serialize<S>(value: &Option<Rect>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value
            .map(|rect| RectDto {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            })
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Rect>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<RectDto>::deserialize(deserializer)?;
        Ok(value.map(|rect| Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Image {
        Image {
            width: 8,
            height: 8,
            scale_factor: 1.0,
            rgba: vec![255; 8 * 8 * 4],
        }
    }

    fn line(id: u64) -> Item {
        Item {
            id,
            shape: Shape::Line {
                from: Point { x: 0.0, y: 0.0 },
                to: Point { x: 4.0, y: 0.0 },
            },
            style: Style::default(),
        }
    }

    #[test]
    fn undo_and_redo_cover_every_edit() {
        let mut doc = Document::new(blank());
        let added = line(1);
        doc.apply(Edit::Add(added.clone()));
        assert_eq!(doc.items, vec![added.clone()]);
        assert!(doc.undo());
        assert!(doc.items.is_empty());
        assert!(doc.redo());
        assert_eq!(doc.items, vec![added.clone()]);
        assert!(!doc.redo());

        let after = Item {
            shape: Shape::Rect {
                rect: RectF {
                    x: 1.0,
                    y: 1.0,
                    width: 2.0,
                    height: 2.0,
                },
            },
            ..added.clone()
        };
        doc.apply(Edit::Modify {
            before: added.clone(),
            after: after.clone(),
        });
        assert_eq!(doc.items[0], after);
        assert!(doc.undo());
        assert_eq!(doc.items[0], added);
        assert!(doc.redo());
        assert_eq!(doc.items[0], after);

        doc.apply(Edit::Remove(after.clone()));
        assert!(doc.items.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.items[0].id, after.id);

        let crop = Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        doc.apply(Edit::Crop {
            before: None,
            after: Some(crop),
        });
        assert_eq!(doc.crop, Some(crop));
        assert!(doc.undo());
        assert_eq!(doc.crop, None);
        assert!(doc.redo());
        assert_eq!(doc.crop, Some(crop));

        assert!(doc.undo());
        doc.apply(Edit::Add(line(9)));
        assert!(doc.redo.is_empty(), "a new edit drops the redo stack");
        for _ in 0..8 {
            if !doc.undo() {
                break;
            }
        }
        assert!(!doc.undo());
    }

    #[test]
    fn step_numbers_increment_from_the_highest() {
        let mut doc = Document::new(blank());
        assert_eq!(doc.next_step_number(), 1);
        doc.apply(Edit::Add(Item {
            id: 1,
            shape: Shape::Step {
                at: Point { x: 1.0, y: 1.0 },
                n: 1,
            },
            style: Style::default(),
        }));
        doc.apply(Edit::Add(Item {
            id: 2,
            shape: Shape::Step {
                at: Point { x: 3.0, y: 3.0 },
                n: 3,
            },
            style: Style::default(),
        }));
        doc.apply(Edit::Add(line(4)));
        assert_eq!(doc.next_step_number(), 4);
    }

    #[test]
    fn hit_test_picks_the_top_most_item() {
        let mut doc = Document::new(blank());
        doc.items.push(Item {
            id: 1,
            shape: Shape::Rect {
                rect: RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 6.0,
                    height: 6.0,
                },
            },
            style: Style::default(),
        });
        doc.items.push(Item {
            id: 2,
            shape: Shape::Ellipse {
                rect: RectF {
                    x: 2.0,
                    y: 2.0,
                    width: 4.0,
                    height: 4.0,
                },
            },
            style: Style::default(),
        });
        doc.items.push(Item {
            id: 3,
            shape: Shape::Text {
                at: Point { x: 0.0, y: 0.0 },
                text: "A".into(),
                size: 4.0,
            },
            style: Style::default(),
        });
        assert_eq!(doc.hit_test(Point { x: 1.0, y: 1.0 }), Some(3));
        assert_eq!(doc.hit_test(Point { x: 5.0, y: 5.0 }), Some(2));
        assert_eq!(doc.hit_test(Point { x: 20.0, y: 20.0 }), None);
        doc.base.scale_factor = 2.0;
        doc.items.push(Item {
            id: 4,
            shape: Shape::Step {
                at: Point { x: 1.0, y: 1.0 },
                n: 1,
            },
            style: Style::default(),
        });
        assert_eq!(doc.hit_test(Point { x: 1.0, y: 1.0 }), Some(4));
    }

    #[test]
    fn document_json_round_trip_keeps_items_and_crop() {
        let mut doc = Document::new(blank());
        doc.apply(Edit::Add(line(1)));
        doc.crop = Some(Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        });
        let json = serde_json::to_string(&doc).expect("serialise");
        let loaded: Document = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(loaded.items, doc.items);
        assert_eq!(loaded.crop, doc.crop);
        assert_eq!(loaded.base.rgba, doc.base.rgba);
        assert!(loaded.undo.is_empty());
    }
}
