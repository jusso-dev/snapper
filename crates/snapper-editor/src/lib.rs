//! Annotation model. See docs/ARCHITECTURE.md.

/// A single annotation layer item.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Arrow { from: (f32, f32), to: (f32, f32) },
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Text { x: f32, y: f32, text: String },
    Step { x: f32, y: f32, n: u32 },
}
