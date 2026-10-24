//! Pointer tools for the annotation editor.
//!
//! The session turns pointer and keyboard input into [`Edit`]s. The egui view
//! applies those edits; it does not invent geometry of its own.

use snapper_core::Rect;
use snapper_editor::{Document, Edit, Item, Point, RectF, Rgba, Shape, Style};

/// Toolbar tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Arrow,
    Rect,
    Ellipse,
    Highlight,
    Text,
    Step,
    Blur,
    Pixelate,
    Crop,
}

/// A key the editor cares about, independent of egui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKey {
    A,
    R,
    E,
    H,
    T,
    N,
    B,
    P,
    C,
    Z,
    Y,
    S,
    Delete,
    Backspace,
    Enter,
    Escape,
}

/// What a shortcut asked the editor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Tool(Tool),
    Undo,
    Redo,
    Copy,
    Save,
    Delete,
    CommitText,
    Cancel,
}

/// Map a key chord to an editor action.
pub fn interpret(ctrl: bool, key: CommandKey) -> Option<Action> {
    match (ctrl, key) {
        (true, CommandKey::Z) => Some(Action::Undo),
        (true, CommandKey::Y) => Some(Action::Redo),
        (true, CommandKey::C) => Some(Action::Copy),
        (true, CommandKey::S) => Some(Action::Save),
        (false, CommandKey::S) => Some(Action::Tool(Tool::Select)),
        (true, CommandKey::Enter) => Some(Action::CommitText),
        (false, CommandKey::A) => Some(Action::Tool(Tool::Arrow)),
        (false, CommandKey::R) => Some(Action::Tool(Tool::Rect)),
        (false, CommandKey::E) => Some(Action::Tool(Tool::Ellipse)),
        (false, CommandKey::H) => Some(Action::Tool(Tool::Highlight)),
        (false, CommandKey::T) => Some(Action::Tool(Tool::Text)),
        (false, CommandKey::N) => Some(Action::Tool(Tool::Step)),
        (false, CommandKey::B) => Some(Action::Tool(Tool::Blur)),
        (false, CommandKey::P) => Some(Action::Tool(Tool::Pixelate)),
        (false, CommandKey::C) => Some(Action::Tool(Tool::Crop)),
        (false, CommandKey::Delete | CommandKey::Backspace) => Some(Action::Delete),
        (false, CommandKey::Enter) => Some(Action::CommitText),
        (false, CommandKey::Escape) => Some(Action::Cancel),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Pressed,
    Moved,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub point: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle {
    Start,
    End,
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

#[derive(Debug, Clone)]
enum Gesture {
    Draw {
        from: Point,
    },
    Move {
        id: u64,
        start: Point,
        original: Item,
    },
    Resize {
        id: u64,
        handle: Handle,
        original: Item,
    },
}

/// Inline text that has not been committed yet.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDraft {
    pub at: Point,
    pub text: String,
    /// False until the text field has been shown once, so the first frame
    /// does not count as clicking away.
    pub armed: bool,
}

/// Tool state machine. `on_pointer` returns an edit when a gesture finishes.
pub struct ToolSession {
    pub tool: Tool,
    pub style: Style,
    pub zoom: f32,
    pub selected: Option<u64>,
    pub text: Option<TextDraft>,
    items: Vec<Item>,
    crop: Option<Rect>,
    /// Base image size, once synced, so crops stay inside the image.
    image_size: Option<(u32, u32)>,
    scale: f32,
    next_id: u64,
    gesture: Option<Gesture>,
    live: Option<Item>,
    live_crop: Option<RectF>,
}

impl ToolSession {
    pub fn new(style: Style) -> Self {
        Self {
            tool: Tool::Select,
            style,
            zoom: 1.0,
            selected: None,
            text: None,
            items: Vec::new(),
            crop: None,
            image_size: None,
            scale: 1.0,
            next_id: 1,
            gesture: None,
            live: None,
            live_crop: None,
        }
    }

    pub fn sync(&mut self, doc: &Document) {
        if self.gesture.is_some() {
            return;
        }
        self.items = doc.items.clone();
        self.crop = doc.crop;
        self.image_size = Some((doc.base.width, doc.base.height));
        self.scale = if doc.base.scale_factor.is_finite() && doc.base.scale_factor > 0.0 {
            doc.base.scale_factor
        } else {
            1.0
        };
        let max_id = self.items.iter().map(|item| item.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max_id.saturating_add(1));
    }

    /// Crop rectangle being dragged, before the edit is committed.
    pub fn preview_crop(&self) -> Option<RectF> {
        self.live_crop
    }

    /// Items plus the shape currently being drawn or transformed.
    pub fn display_items(&self) -> Vec<Item> {
        let mut items = self.items.clone();
        if let Some(live) = &self.live {
            if let Some(slot) = items.iter_mut().find(|item| item.id == live.id) {
                *slot = live.clone();
            } else {
                items.push(live.clone());
            }
        }
        items
    }

    pub fn on_pointer(&mut self, event: PointerEvent) -> Option<Edit> {
        if self.text.is_some() && event.kind == PointerKind::Pressed {
            return self.commit_text();
        }
        match event.kind {
            PointerKind::Pressed => self.press(event.point),
            PointerKind::Moved => {
                self.drag(event.point);
                None
            }
            PointerKind::Released => self.release(event.point),
        }
    }

    pub fn commit_text(&mut self) -> Option<Edit> {
        let draft = self.text.take()?;
        let text = draft.text.trim();
        if text.is_empty() {
            return None;
        }
        let item = self.item(Shape::Text {
            at: draft.at,
            text: text.to_string(),
            // 18 logical points, so text reads the same size on a Retina capture.
            size: 18.0 * self.scale,
        });
        self.selected = Some(item.id);
        Some(Edit::Add(item))
    }

    pub fn delete_selected(&mut self) -> Option<Edit> {
        let id = self.selected?;
        let item = self.items.iter().find(|item| item.id == id)?.clone();
        self.selected = None;
        self.gesture = None;
        self.live = None;
        Some(Edit::remove(item))
    }

    pub fn cancel(&mut self) {
        self.cancel_gesture();
        self.text = None;
    }

    /// Drop an in-progress drag so its release cannot emit a stale edit,
    /// e.g. before undo or redo replaces the items it was editing.
    pub fn cancel_gesture(&mut self) {
        self.gesture = None;
        self.live = None;
        self.live_crop = None;
    }

    fn press(&mut self, point: Point) -> Option<Edit> {
        match self.tool {
            Tool::Select => {
                if let Some(id) = self.selected {
                    if let Some(item) = self.items.iter().find(|item| item.id == id).cloned() {
                        if let Some(handle) = hit_handle(&item, point, self.tolerance()) {
                            self.gesture = Some(Gesture::Resize {
                                id,
                                handle,
                                original: item,
                            });
                            return None;
                        }
                    }
                }
                let hit = self.hit(point);
                self.selected = hit;
                if let Some(id) = hit {
                    if let Some(item) = self.items.iter().find(|item| item.id == id).cloned() {
                        self.gesture = Some(Gesture::Move {
                            id,
                            start: point,
                            original: item,
                        });
                    }
                }
                None
            }
            Tool::Text => {
                self.text = Some(TextDraft {
                    at: point,
                    text: String::new(),
                    armed: false,
                });
                None
            }
            Tool::Step => None,
            Tool::Arrow
            | Tool::Rect
            | Tool::Ellipse
            | Tool::Highlight
            | Tool::Blur
            | Tool::Pixelate
            | Tool::Crop => {
                self.gesture = Some(Gesture::Draw { from: point });
                None
            }
        }
    }

    fn drag(&mut self, point: Point) {
        let Some(gesture) = self.gesture.clone() else {
            return;
        };
        match gesture {
            Gesture::Draw { from } => {
                self.gesture = Some(Gesture::Draw { from });
                self.live = self.draft_item(from, point);
                self.live_crop = if self.tool == Tool::Crop {
                    rect_f(from, point)
                } else {
                    None
                };
            }
            Gesture::Move {
                id,
                start,
                original,
            } => {
                let moved = translate_item(&original, point.x - start.x, point.y - start.y);
                self.live = Some(moved);
                self.gesture = Some(Gesture::Move {
                    id,
                    start,
                    original,
                });
            }
            Gesture::Resize {
                id,
                handle,
                original,
            } => {
                self.live = Some(resize_item(&original, handle, point));
                self.gesture = Some(Gesture::Resize {
                    id,
                    handle,
                    original,
                });
            }
        }
    }

    fn release(&mut self, point: Point) -> Option<Edit> {
        if self.tool == Tool::Step && self.gesture.is_none() {
            let n = next_step(&self.items);
            let item = self.item(Shape::Step { at: point, n });
            self.selected = Some(item.id);
            return Some(Edit::Add(item));
        }
        let gesture = self.gesture.take()?;
        self.live = None;
        self.live_crop = None;
        match gesture {
            Gesture::Draw { from, .. } => self.finish_draw(from, point),
            Gesture::Move {
                start, original, ..
            } => {
                let dx = point.x - start.x;
                let dy = point.y - start.y;
                if dx.abs() < 0.5 && dy.abs() < 0.5 {
                    return None;
                }
                let after = translate_item(&original, dx, dy);
                self.selected = Some(after.id);
                Some(Edit::Modify {
                    before: original,
                    after,
                })
            }
            Gesture::Resize {
                handle, original, ..
            } => {
                let after = resize_item(&original, handle, point);
                if after == original {
                    return None;
                }
                self.selected = Some(after.id);
                Some(Edit::Modify {
                    before: original,
                    after,
                })
            }
        }
    }

    fn finish_draw(&mut self, from: Point, to: Point) -> Option<Edit> {
        if self.tool == Tool::Crop {
            let mut rect = core_rect(from, to)?;
            if let Some((width, height)) = self.image_size {
                rect = clamp_to_image(rect, width, height)?;
            }
            let before = self.crop;
            self.crop = Some(rect);
            return Some(Edit::Crop {
                before,
                after: Some(rect),
            });
        }
        let item = self.draft_item(from, to)?;
        if !has_extent(&item) {
            return None;
        }
        let item = self.take_id(item);
        self.selected = Some(item.id);
        Some(Edit::Add(item))
    }

    fn draft_item(&self, from: Point, to: Point) -> Option<Item> {
        let shape = match self.tool {
            Tool::Arrow => Shape::Arrow { from, to },
            Tool::Rect => Shape::Rect {
                rect: rect_f(from, to)?,
            },
            Tool::Ellipse => Shape::Ellipse {
                rect: rect_f(from, to)?,
            },
            Tool::Highlight => Shape::Highlight {
                rect: rect_f(from, to)?,
            },
            Tool::Blur => Shape::Blur {
                rect: rect_f(from, to)?,
                radius: 4.0,
            },
            Tool::Pixelate => Shape::Pixelate {
                rect: rect_f(from, to)?,
                block: 8,
            },
            _ => return None,
        };
        Some(Item {
            id: self.next_id,
            shape,
            style: self.item_style(),
            auto: false,
        })
    }

    fn item(&mut self, shape: Shape) -> Item {
        let item = Item {
            id: self.next_id,
            shape,
            style: self.item_style(),
            auto: false,
        };
        self.next_id = self.next_id.saturating_add(1);
        item
    }

    /// The chosen style with its width, picked in logical points, converted
    /// to image pixels so a stroke looks the same on a Retina capture.
    fn item_style(&self) -> Style {
        Style {
            width: self.style.width * self.scale,
            ..self.style
        }
    }

    fn take_id(&mut self, mut item: Item) -> Item {
        item.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        item
    }

    fn hit(&self, point: Point) -> Option<u64> {
        let mut probe = Document::new(snapper_core::Image {
            width: 1,
            height: 1,
            scale_factor: self.scale,
            rgba: vec![0, 0, 0, 255],
        });
        probe.items = self.items.clone();
        probe.hit_test(point)
    }

    fn tolerance(&self) -> f32 {
        8.0 / self.zoom.max(0.05)
    }
}

fn next_step(items: &[Item]) -> u32 {
    items
        .iter()
        .filter_map(|item| match item.shape {
            Shape::Step { n, .. } => Some(n),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn rect_f(from: Point, to: Point) -> Option<RectF> {
    let rect = RectF {
        x: from.x.min(to.x),
        y: from.y.min(to.y),
        width: (from.x - to.x).abs(),
        height: (from.y - to.y).abs(),
    };
    if rect.width < 1.0 || rect.height < 1.0 {
        None
    } else {
        Some(rect)
    }
}

fn core_rect(from: Point, to: Point) -> Option<Rect> {
    let rect = rect_f(from, to)?;
    Some(Rect {
        x: rect.x.round() as i32,
        y: rect.y.round() as i32,
        width: rect.width.round().max(1.0) as u32,
        height: rect.height.round().max(1.0) as u32,
    })
}

/// Intersect `rect` with a `width` x `height` image. `None` when they do not overlap.
fn clamp_to_image(rect: Rect, width: u32, height: u32) -> Option<Rect> {
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    let x0 = i64::from(rect.x).clamp(0, i64::from(width));
    let y0 = i64::from(rect.y).clamp(0, i64::from(height));
    let x1 = right.clamp(0, i64::from(width));
    let y1 = bottom.clamp(0, i64::from(height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect {
        x: x0 as i32,
        y: y0 as i32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

fn has_extent(item: &Item) -> bool {
    match &item.shape {
        Shape::Arrow { from, to } => from.x != to.x || from.y != to.y,
        Shape::Rect { rect }
        | Shape::Ellipse { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => rect.width >= 1.0 && rect.height >= 1.0,
        _ => true,
    }
}

fn translate_item(item: &Item, dx: f32, dy: f32) -> Item {
    let mut after = item.clone();
    let shift = |point: Point| Point {
        x: point.x + dx,
        y: point.y + dy,
    };
    let shift_rect = |rect: RectF| RectF {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    };
    after.shape = match item.shape.clone() {
        Shape::Arrow { from, to } => Shape::Arrow {
            from: shift(from),
            to: shift(to),
        },
        Shape::Line { from, to } => Shape::Line {
            from: shift(from),
            to: shift(to),
        },
        Shape::Rect { rect } => Shape::Rect {
            rect: shift_rect(rect),
        },
        Shape::Ellipse { rect } => Shape::Ellipse {
            rect: shift_rect(rect),
        },
        Shape::Highlight { rect } => Shape::Highlight {
            rect: shift_rect(rect),
        },
        Shape::Blur { rect, radius } => Shape::Blur {
            rect: shift_rect(rect),
            radius,
        },
        Shape::Pixelate { rect, block } => Shape::Pixelate {
            rect: shift_rect(rect),
            block,
        },
        Shape::Text { at, text, size } => Shape::Text {
            at: shift(at),
            text,
            size,
        },
        Shape::Step { at, n } => Shape::Step { at: shift(at), n },
    };
    after
}

fn resize_item(item: &Item, handle: Handle, point: Point) -> Item {
    let mut after = item.clone();
    after.shape = match &item.shape {
        Shape::Arrow { from, to } | Shape::Line { from, to } => {
            let (from, to) = match handle {
                Handle::Start => (point, *to),
                _ => (*from, point),
            };
            match &item.shape {
                Shape::Arrow { .. } => Shape::Arrow { from, to },
                _ => Shape::Line { from, to },
            }
        }
        Shape::Rect { rect }
        | Shape::Ellipse { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => {
            let rect = resize_rect(*rect, handle, point);
            match &item.shape {
                Shape::Rect { .. } => Shape::Rect { rect },
                Shape::Ellipse { .. } => Shape::Ellipse { rect },
                Shape::Highlight { .. } => Shape::Highlight { rect },
                Shape::Blur { radius, .. } => Shape::Blur {
                    rect,
                    radius: *radius,
                },
                Shape::Pixelate { block, .. } => Shape::Pixelate {
                    rect,
                    block: *block,
                },
                _ => item.shape.clone(),
            }
        }
        Shape::Text { at, text, .. } => {
            // Pick the size whose box reaches the pointer, so the handle stays under it.
            let per_size = snapper_editor::text_bounds(*at, text, 1.0).width;
            let size = ((point.x - at.x).abs() / per_size).max(8.0);
            Shape::Text {
                at: *at,
                text: text.clone(),
                size,
            }
        }
        other => other.clone(),
    };
    after
}

fn resize_rect(rect: RectF, handle: Handle, point: Point) -> RectF {
    let rect = rect.normalised();
    let (left, top, right, bottom) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    let (left, top, right, bottom) = match handle {
        Handle::NorthWest => (point.x, point.y, right, bottom),
        Handle::NorthEast => (left, point.y, point.x, bottom),
        Handle::SouthWest => (point.x, top, right, point.y),
        Handle::SouthEast | Handle::End | Handle::Start => (left, top, point.x, point.y),
    };
    RectF {
        x: left.min(right),
        y: top.min(bottom),
        width: (right - left).abs().max(1.0),
        height: (bottom - top).abs().max(1.0),
    }
}

fn hit_handle(item: &Item, point: Point, tolerance: f32) -> Option<Handle> {
    let near = |anchor: Point| {
        (point.x - anchor.x).abs() <= tolerance && (point.y - anchor.y).abs() <= tolerance
    };
    match &item.shape {
        Shape::Arrow { from, to } | Shape::Line { from, to } => {
            if near(*from) {
                Some(Handle::Start)
            } else if near(*to) {
                Some(Handle::End)
            } else {
                None
            }
        }
        Shape::Rect { .. }
        | Shape::Ellipse { .. }
        | Shape::Highlight { .. }
        | Shape::Blur { .. }
        | Shape::Pixelate { .. }
        | Shape::Text { .. } => {
            let rect = item_bounds(item)?.normalised();
            let corners = [
                (
                    Handle::NorthWest,
                    Point {
                        x: rect.x,
                        y: rect.y,
                    },
                ),
                (
                    Handle::NorthEast,
                    Point {
                        x: rect.x + rect.width,
                        y: rect.y,
                    },
                ),
                (
                    Handle::SouthWest,
                    Point {
                        x: rect.x,
                        y: rect.y + rect.height,
                    },
                ),
                (
                    Handle::SouthEast,
                    Point {
                        x: rect.x + rect.width,
                        y: rect.y + rect.height,
                    },
                ),
            ];
            corners
                .into_iter()
                .find(|(_, anchor)| near(*anchor))
                .map(|(handle, _)| handle)
        }
        Shape::Step { .. } => None,
    }
}

fn item_bounds(item: &Item) -> Option<RectF> {
    match &item.shape {
        Shape::Rect { rect }
        | Shape::Ellipse { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => Some(*rect),
        Shape::Text { at, text, size } => Some(snapper_editor::text_bounds(*at, text, *size)),
        _ => None,
    }
}

/// Zoom that fits `image` inside `view`.
pub fn zoom_to_fit(view_w: f32, view_h: f32, image_w: f32, image_h: f32) -> f32 {
    if view_w <= 0.0 || view_h <= 0.0 || image_w <= 0.0 || image_h <= 0.0 {
        return 1.0;
    }
    (view_w / image_w).min(view_h / image_h).clamp(0.05, 32.0)
}

/// Ctrl+wheel zoom. `steps` is positive when the wheel rolls away from the user.
pub fn zoom_wheel(zoom: f32, steps: f32) -> f32 {
    (zoom * 1.1_f32.powf(steps)).clamp(0.05, 32.0)
}

/// Screen-space placement of the image inside the editor.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub origin_x: f32,
    pub origin_y: f32,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

pub fn screen_to_image(screen_x: f32, screen_y: f32, view: View) -> Point {
    let zoom = if view.zoom.is_finite() && view.zoom > 0.0 {
        view.zoom
    } else {
        1.0
    };
    Point {
        x: (screen_x - view.origin_x - view.pan_x) / zoom,
        y: (screen_y - view.origin_y - view.pan_y) / zoom,
    }
}

pub fn image_to_screen(point: Point, view: View) -> (f32, f32) {
    (
        view.origin_x + view.pan_x + point.x * view.zoom,
        view.origin_y + view.pan_y + point.y * view.zoom,
    )
}

/// `#RRGGBBAA`.
pub fn format_colour(colour: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        colour.r, colour.g, colour.b, colour.a
    )
}

const DEFAULT_COLOUR: Rgba = Rgba {
    r: 0xE2,
    g: 0x3D,
    b: 0x3D,
    a: 0xFF,
};

/// Parse `#RRGGBB` or `#RRGGBBAA`. Anything else is the default red.
pub fn parse_colour(text: &str) -> Rgba {
    let hex = text.trim().trim_start_matches('#');
    if !hex.is_ascii() {
        return DEFAULT_COLOUR;
    }
    let byte = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    if hex.len() == 6 || hex.len() == 8 {
        if let (Some(r), Some(g), Some(b)) = (byte(0), byte(2), byte(4)) {
            let a = if hex.len() == 8 {
                byte(6).unwrap_or(255)
            } else {
                255
            };
            return Rgba { r, g, b, a };
        }
    }
    DEFAULT_COLOUR
}

pub fn style_from_settings(colour: &str, width: f32) -> Style {
    Style {
        stroke: parse_colour(colour),
        fill: None,
        width: if width.is_finite() && width > 0.0 {
            width
        } else {
            4.0
        },
        shadow: false,
    }
}

/// Remember the stroke the user just picked.
pub fn remember_style(colour: &mut String, width: &mut f32, style: &Style) {
    *colour = format_colour(style.stroke);
    *width = style.width;
}

/// Unsaved work should ask before the window disappears.
pub fn close_needs_confirm(dirty: bool) -> bool {
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapper_core::Image;

    fn session() -> ToolSession {
        ToolSession::new(Style::default())
    }

    fn blank() -> Document {
        Document::new(Image {
            width: 80,
            height: 60,
            scale_factor: 1.0,
            rgba: vec![255; 80 * 60 * 4],
        })
    }

    fn drag(session: &mut ToolSession, from: Point, to: Point) -> Option<Edit> {
        session.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: from,
        });
        session.on_pointer(PointerEvent {
            kind: PointerKind::Moved,
            point: to,
        });
        session.on_pointer(PointerEvent {
            kind: PointerKind::Released,
            point: to,
        })
    }

    fn shape_name(shape: &Shape) -> &'static str {
        match shape {
            Shape::Arrow { .. } => "arrow",
            Shape::Line { .. } => "line",
            Shape::Rect { .. } => "rect",
            Shape::Ellipse { .. } => "ellipse",
            Shape::Highlight { .. } => "highlight",
            Shape::Text { .. } => "text",
            Shape::Step { .. } => "step",
            Shape::Blur { .. } => "blur",
            Shape::Pixelate { .. } => "pixelate",
        }
    }

    #[test]
    fn shortcuts_match_the_issue() {
        assert_eq!(
            interpret(false, CommandKey::A),
            Some(Action::Tool(Tool::Arrow))
        );
        assert_eq!(
            interpret(false, CommandKey::R),
            Some(Action::Tool(Tool::Rect))
        );
        assert_eq!(
            interpret(false, CommandKey::E),
            Some(Action::Tool(Tool::Ellipse))
        );
        assert_eq!(
            interpret(false, CommandKey::H),
            Some(Action::Tool(Tool::Highlight))
        );
        assert_eq!(
            interpret(false, CommandKey::T),
            Some(Action::Tool(Tool::Text))
        );
        assert_eq!(
            interpret(false, CommandKey::N),
            Some(Action::Tool(Tool::Step))
        );
        assert_eq!(
            interpret(false, CommandKey::B),
            Some(Action::Tool(Tool::Blur))
        );
        assert_eq!(
            interpret(false, CommandKey::P),
            Some(Action::Tool(Tool::Pixelate))
        );
        assert_eq!(
            interpret(false, CommandKey::C),
            Some(Action::Tool(Tool::Crop))
        );
        assert_eq!(interpret(true, CommandKey::Z), Some(Action::Undo));
        assert_eq!(interpret(true, CommandKey::Y), Some(Action::Redo));
        assert_eq!(interpret(true, CommandKey::C), Some(Action::Copy));
        assert_eq!(interpret(true, CommandKey::S), Some(Action::Save));
        assert_eq!(
            interpret(false, CommandKey::S),
            Some(Action::Tool(Tool::Select))
        );
        assert_eq!(interpret(false, CommandKey::Delete), Some(Action::Delete));
        assert_eq!(interpret(true, CommandKey::Enter), Some(Action::CommitText));
    }

    #[test]
    fn every_tool_creates_an_edit_that_undoes() {
        let cases = [
            (Tool::Arrow, "arrow"),
            (Tool::Rect, "rect"),
            (Tool::Ellipse, "ellipse"),
            (Tool::Highlight, "highlight"),
            (Tool::Blur, "blur"),
            (Tool::Pixelate, "pixelate"),
        ];
        for (tool, name) in cases {
            let mut tools = session();
            tools.tool = tool;
            let edit = drag(
                &mut tools,
                Point { x: 4.0, y: 6.0 },
                Point { x: 30.0, y: 24.0 },
            )
            .unwrap_or_else(|| panic!("{name} produced no edit"));
            let Edit::Add(item) = edit.clone() else {
                panic!("{name} was not an add");
            };
            assert_eq!(shape_name(&item.shape), name);
            let mut doc = blank();
            doc.apply(edit);
            assert_eq!(doc.items.len(), 1);
            assert!(doc.undo());
            assert!(doc.items.is_empty(), "{name} did not undo");
        }

        let mut tools = session();
        tools.tool = Tool::Step;
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: Point { x: 8.0, y: 8.0 },
        });
        let edit = tools
            .on_pointer(PointerEvent {
                kind: PointerKind::Released,
                point: Point { x: 8.0, y: 8.0 },
            })
            .expect("step");
        let Edit::Add(item) = &edit else {
            panic!("step add")
        };
        assert!(matches!(item.shape, Shape::Step { n: 1, .. }));
        let mut doc = blank();
        doc.apply(edit);
        assert!(doc.undo());
        assert!(doc.items.is_empty());

        let mut tools = session();
        tools.tool = Tool::Text;
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: Point { x: 12.0, y: 14.0 },
        });
        tools.text.as_mut().unwrap().text = "Hi".into();
        let edit = tools.commit_text().expect("text");
        let Edit::Add(item) = &edit else {
            panic!("text add")
        };
        assert!(matches!(item.shape, Shape::Text { .. }));
        let mut doc = blank();
        doc.apply(edit);
        assert!(doc.undo());

        let mut tools = session();
        tools.tool = Tool::Crop;
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: Point { x: 2.0, y: 3.0 },
        });
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Moved,
            point: Point { x: 20.0, y: 18.0 },
        });
        assert!(tools.preview_crop().is_some());
        let edit = tools
            .on_pointer(PointerEvent {
                kind: PointerKind::Released,
                point: Point { x: 20.0, y: 18.0 },
            })
            .expect("crop");
        assert!(tools.preview_crop().is_none());
        let Edit::Crop { before, after } = edit.clone() else {
            panic!("crop edit");
        };
        assert_eq!(before, None);
        assert!(after.is_some());
        let mut doc = blank();
        doc.apply(edit);
        assert!(doc.undo());
        assert_eq!(doc.crop, None);
    }

    #[test]
    fn select_moves_resizes_and_deletes() {
        let mut doc = blank();
        let original = Item {
            id: 3,
            shape: Shape::Rect {
                rect: RectF {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 16.0,
                },
            },
            style: Style::default(),
            auto: false,
        };
        doc.apply(Edit::Add(original.clone()));
        let mut tools = session();
        tools.sync(&doc);
        tools.tool = Tool::Select;
        let moved = drag(
            &mut tools,
            Point { x: 12.0, y: 12.0 },
            Point { x: 18.0, y: 15.0 },
        )
        .expect("move");
        let Edit::Modify { after, .. } = moved.clone() else {
            panic!("modify");
        };
        let Shape::Rect { rect } = after.shape else {
            panic!("rect")
        };
        assert_eq!(rect.x, 16.0);
        assert_eq!(rect.y, 13.0);
        doc.apply(moved);
        tools.sync(&doc);

        tools.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: Point { x: 36.0, y: 29.0 },
        });
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Moved,
            point: Point { x: 40.0, y: 36.0 },
        });
        let resized = tools
            .on_pointer(PointerEvent {
                kind: PointerKind::Released,
                point: Point { x: 40.0, y: 36.0 },
            })
            .expect("resize");
        let Edit::Modify { after, .. } = resized.clone() else {
            panic!("resize modify");
        };
        let Shape::Rect { rect } = after.shape else {
            panic!("rect")
        };
        assert_eq!((rect.width, rect.height), (24.0, 23.0));
        doc.apply(resized);
        tools.sync(&doc);
        let removed = tools.delete_selected().expect("delete");
        doc.apply(removed);
        assert!(doc.items.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.items.len(), 1);
    }

    #[test]
    fn zoom_fit_hundred_and_wheel() {
        assert_eq!(zoom_to_fit(200.0, 100.0, 100.0, 100.0), 1.0);
        assert_eq!(zoom_to_fit(50.0, 40.0, 100.0, 100.0), 0.4);
        let zoom = zoom_wheel(1.0, 1.0);
        assert!((zoom - 1.1).abs() < 0.0001);
        let view = View {
            origin_x: 10.0,
            origin_y: 10.0,
            zoom: 2.0,
            pan_x: 4.0,
            pan_y: 6.0,
        };
        let point = screen_to_image(30.0, 40.0, view);
        assert_eq!(point, Point { x: 8.0, y: 12.0 });
        assert_eq!(image_to_screen(point, view), (30.0, 40.0));
    }

    #[test]
    fn colour_and_width_round_trip_into_settings_fields() {
        let style = style_from_settings("#E23D3DFF", 4.0);
        assert_eq!(
            style.stroke,
            Rgba {
                r: 0xE2,
                g: 0x3D,
                b: 0x3D,
                a: 0xFF,
            }
        );
        let mut colour = String::new();
        let mut width = 0.0;
        let picked = Style {
            stroke: Rgba {
                r: 1,
                g: 2,
                b: 3,
                a: 4,
            },
            width: 6.5,
            ..style
        };
        remember_style(&mut colour, &mut width, &picked);
        assert_eq!(colour, "#01020304");
        assert_eq!(width, 6.5);
        assert_eq!(parse_colour("nope").r, 0xE2);
        assert_eq!(parse_colour("aééb"), parse_colour("nope"));
        assert_eq!(parse_colour("#ééééé"), parse_colour("nope"));
        assert!(close_needs_confirm(true));
        assert!(!close_needs_confirm(false));
    }

    #[test]
    fn crop_is_clamped_to_the_image_and_ignored_outside_it() {
        let mut tools = session();
        tools.sync(&blank());
        tools.tool = Tool::Crop;
        let edit = drag(
            &mut tools,
            Point { x: -10.0, y: 40.0 },
            Point { x: 30.0, y: 90.0 },
        )
        .expect("crop");
        let Edit::Crop { after, .. } = edit else {
            panic!("crop edit");
        };
        assert_eq!(
            after,
            Some(Rect {
                x: 0,
                y: 40,
                width: 30,
                height: 20,
            })
        );
        let outside = drag(
            &mut tools,
            Point { x: 100.0, y: 100.0 },
            Point { x: 140.0, y: 130.0 },
        );
        assert!(outside.is_none());
    }

    #[test]
    fn strokes_scale_with_a_retina_capture() {
        let mut tools = session();
        let mut retina = blank();
        retina.base.scale_factor = 2.0;
        tools.sync(&retina);
        tools.style.width = 4.0;
        tools.tool = Tool::Rect;
        let Some(Edit::Add(item)) = drag(
            &mut tools,
            Point { x: 10.0, y: 10.0 },
            Point { x: 40.0, y: 40.0 },
        ) else {
            panic!("rectangle");
        };
        assert_eq!(item.style.width, 8.0);
        assert_eq!(tools.style.width, 4.0);
    }

    #[test]
    fn cancelled_gesture_releases_without_an_edit() {
        let mut doc = blank();
        doc.apply(Edit::Add(Item {
            id: 1,
            shape: Shape::Rect {
                rect: RectF {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 16.0,
                },
            },
            style: Style::default(),
            auto: false,
        }));
        let mut tools = session();
        tools.sync(&doc);
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Pressed,
            point: Point { x: 12.0, y: 12.0 },
        });
        tools.on_pointer(PointerEvent {
            kind: PointerKind::Moved,
            point: Point { x: 20.0, y: 20.0 },
        });
        assert!(doc.undo());
        tools.cancel_gesture();
        tools.sync(&doc);
        let edit = tools.on_pointer(PointerEvent {
            kind: PointerKind::Released,
            point: Point { x: 20.0, y: 20.0 },
        });
        assert!(edit.is_none());
        assert!(tools.display_items().is_empty());
    }

    #[test]
    fn text_resize_keeps_the_handle_under_the_pointer() {
        let item = Item {
            id: 1,
            shape: Shape::Text {
                at: Point { x: 10.0, y: 10.0 },
                text: "abc".into(),
                size: 10.0,
            },
            style: Style::default(),
            auto: false,
        };
        let resized = resize_item(&item, Handle::SouthEast, Point { x: 40.0, y: 30.0 });
        let rect = item_bounds(&resized).expect("bounds");
        assert!((rect.x + rect.width - 40.0).abs() < 0.01);
        let again = resize_item(&resized, Handle::SouthEast, Point { x: 40.0, y: 30.0 });
        assert_eq!(item_bounds(&again), Some(rect));
    }
}
