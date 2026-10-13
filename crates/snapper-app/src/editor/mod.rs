//! egui annotation editor.

mod output;
mod tools;

use eframe::egui::{
    self, Color32, ColorImage, Key, PointerButton, Pos2, Rect, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2, ViewportBuilder, ViewportCommand,
};
use snapper_core::Settings;
use snapper_editor::{Document, Edit, Item, Point, Rgba, Shape, Style};

use output::{flattened, perform, OutputRequest};
use tools::{
    close_needs_confirm, image_to_screen, interpret, remember_style, screen_to_image,
    style_from_settings, zoom_to_fit, zoom_wheel, Action, CommandKey, PointerEvent, PointerKind,
    TextDraft, Tool, ToolSession, View,
};
pub(crate) use tools::{format_colour, parse_colour};

/// Annotation window.
pub struct EditorApp {
    pub doc: Document,
    pub tool: Tool,
    pub style: Style,
    pub zoom: f32,
    pub pan: Vec2,
    pub texture: Option<TextureHandle>,
    pub dirty: bool,
    session: ToolSession,
    settings: Settings,
    confirm_close: bool,
    fitted: bool,
    reload_texture: bool,
    output_note: String,
}

impl EditorApp {
    pub fn new(doc: Document, settings: Settings) -> Self {
        let style =
            style_from_settings(&settings.editor.stroke_color, settings.editor.stroke_width);
        let mut session = ToolSession::new(style);
        session.sync(&doc);
        Self {
            doc,
            tool: Tool::Select,
            style,
            zoom: 1.0,
            pan: Vec2::ZERO,
            texture: None,
            dirty: false,
            session,
            settings,
            confirm_close: false,
            fitted: false,
            reload_texture: true,
            output_note: String::new(),
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.session.tool = tool;
    }

    fn apply_edit(&mut self, edit: Edit) {
        let before = pixel_effects(&self.doc);
        self.doc.apply(edit);
        self.dirty = true;
        self.session.sync(&self.doc);
        self.reload_texture = pixel_effects(&self.doc) != before;
    }

    fn remember(&mut self) {
        remember_style(
            &mut self.settings.editor.stroke_color,
            &mut self.settings.editor.stroke_width,
            &self.style,
        );
        self.session.style = self.style;
        if let Err(err) = self.settings.save() {
            tracing::warn!("could not save editor settings: {err}");
        }
    }

    fn output(&mut self, request: OutputRequest) {
        if let Some(edit) = self.session.commit_text() {
            self.apply_edit(edit);
        }
        let done = perform(request, &self.doc, &self.settings);
        self.output_note = done.note;
        if done.saved {
            self.dirty = false;
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Tool(tool) => self.set_tool(tool),
            Action::Undo => self.restore(|doc| doc.undo()),
            Action::Redo => self.restore(|doc| doc.redo()),
            Action::Copy => self.output(OutputRequest::Copy),
            Action::Save => self.output(OutputRequest::Save),
            Action::Delete => {
                if let Some(edit) = self.session.delete_selected() {
                    self.apply_edit(edit);
                }
            }
            Action::CommitText => {
                if let Some(edit) = self.session.commit_text() {
                    self.apply_edit(edit);
                }
            }
            Action::Cancel => self.session.cancel(),
        }
    }

    fn restore(&mut self, step: impl FnOnce(&mut Document) -> bool) {
        let before = pixel_effects(&self.doc);
        if step(&mut self.doc) {
            self.dirty = true;
            self.session.sync(&self.doc);
            self.reload_texture = pixel_effects(&self.doc) != before;
        }
    }
}

pub fn run_editor(doc: Document, settings: Settings) {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Snapper editor")
            .with_inner_size([960.0, 640.0]),
        persist_window: false,
        ..Default::default()
    };
    let app = EditorApp::new(doc, settings);
    if let Err(err) = eframe::run_native(
        "Snapper editor",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    ) {
        eprintln!("editor failed: {err}");
        std::process::exit(1);
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().close_requested()) && close_needs_confirm(self.dirty)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        self.shortcuts(ctx);
        self.toolbar(ctx);
        self.bottom_bar(ctx);
        self.canvas(ctx);
        if self.confirm_close {
            self.discard_dialog(ctx);
        }
    }
}

impl EditorApp {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let texting = self.session.text.is_some();
        let actions: Vec<Action> = ctx.input(|input| {
            let ctrl = input.modifiers.command;
            KEYS.into_iter()
                .filter(|(key, _)| input.key_pressed(*key))
                .filter_map(|(_, command)| interpret(ctrl, command))
                .filter(|action| !texting || shortcut_during_text(*action))
                .collect()
        });
        for action in actions {
            self.handle_action(action);
        }
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let mut chosen = None;
        let mut fit = false;
        let mut actual = false;
        let mut style_changed = false;
        egui::TopBottomPanel::top("tools").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tool, label) in TOOLS {
                    if ui.selectable_label(self.tool == tool, label).clicked() {
                        chosen = Some(tool);
                    }
                }
                let mut colour = Color32::from_rgba_unmultiplied(
                    self.style.stroke.r,
                    self.style.stroke.g,
                    self.style.stroke.b,
                    self.style.stroke.a,
                );
                if ui.color_edit_button_srgba(&mut colour).changed() {
                    let [r, g, b, a] = colour.to_array();
                    self.style.stroke = Rgba { r, g, b, a };
                    style_changed = true;
                }
                if ui
                    .add(egui::Slider::new(&mut self.style.width, 1.0..=32.0).text("width"))
                    .changed()
                {
                    style_changed = true;
                }
                if ui.button("Fit").clicked() {
                    fit = true;
                }
                if ui.button("100%").clicked() {
                    actual = true;
                }
            });
        });
        if let Some(tool) = chosen {
            self.set_tool(tool);
        }
        if style_changed {
            self.remember();
        }
        if fit {
            self.fitted = false;
        }
        if actual {
            self.zoom = 1.0;
            self.session.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
    }

    fn bottom_bar(&mut self, ctx: &egui::Context) {
        let mut request = None;
        egui::TopBottomPanel::bottom("actions").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    request = Some(OutputRequest::Copy);
                }
                if ui.button("Save").clicked() {
                    request = Some(OutputRequest::Save);
                }
                if ui.button("Upload").clicked() {
                    request = Some(OutputRequest::Upload);
                }
                if !self.output_note.is_empty() {
                    ui.label(&self.output_note);
                }
            });
        });
        if let Some(request) = request {
            self.output(request);
        }
    }

    fn canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            if !self.fitted && rect.width() > 1.0 && rect.height() > 1.0 {
                self.zoom = zoom_to_fit(
                    rect.width(),
                    rect.height(),
                    self.doc.base.width as f32,
                    self.doc.base.height as f32,
                );
                self.session.zoom = self.zoom;
                self.fitted = true;
            }
            self.pan_and_zoom(ui);
            self.ensure_texture(ui.ctx());
            let view = View {
                origin_x: rect.min.x,
                origin_y: rect.min.y,
                zoom: self.zoom,
                pan_x: self.pan.x,
                pan_y: self.pan.y,
            };
            if let Some(handle) = &self.texture {
                let (w, h) = image_to_screen(
                    Point {
                        x: self.doc.base.width as f32,
                        y: self.doc.base.height as f32,
                    },
                    view,
                );
                let image_rect = Rect::from_min_max(
                    Pos2::new(view.origin_x + view.pan_x, view.origin_y + view.pan_y),
                    Pos2::new(w, h),
                );
                ui.painter().image(
                    handle.id(),
                    image_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            for item in self.session.display_items() {
                paint_item(ui, &item, view, self.session.selected == Some(item.id));
            }
            if let Some(crop) = self.doc.crop {
                stroke_image_rect(
                    ui,
                    snapper_editor::RectF {
                        x: crop.x as f32,
                        y: crop.y as f32,
                        width: crop.width as f32,
                        height: crop.height as f32,
                    },
                    view,
                    Color32::WHITE,
                );
            }
            if let Some(crop) = self.session.preview_crop() {
                stroke_image_rect(ui, crop, view, Color32::WHITE);
            }
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            self.pointer(ui, &response, view);
            if let Some(draft) = self.session.text.clone() {
                self.text_popup(ui.ctx(), draft, view);
            }
        });
    }

    fn pan_and_zoom(&mut self, ui: &egui::Ui) {
        let (zoom_steps, pan_delta, middle) = ui.input(|input| {
            let ctrl = input.modifiers.command;
            let steps = if ctrl {
                input.smooth_scroll_delta.y / 40.0
            } else {
                0.0
            };
            let middle = input.pointer.button_down(PointerButton::Middle);
            (steps, input.pointer.delta(), middle)
        });
        if zoom_steps.abs() > f32::EPSILON {
            self.zoom = zoom_wheel(self.zoom, zoom_steps);
            self.session.zoom = self.zoom;
        }
        if middle {
            self.pan += pan_delta;
        }
    }

    fn pointer(&mut self, ui: &egui::Ui, response: &egui::Response, view: View) {
        let event = ui.input(|input| {
            let pos = input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())?;
            let kind = if input.pointer.primary_pressed() {
                PointerKind::Pressed
            } else if input.pointer.primary_released() {
                PointerKind::Released
            } else if input.pointer.primary_down() && response.dragged() {
                PointerKind::Moved
            } else {
                return None;
            };
            Some(PointerEvent {
                kind,
                point: screen_to_image(pos.x, pos.y, view),
            })
        });
        if let Some(event) = event {
            if let Some(edit) = self.session.on_pointer(event) {
                self.apply_edit(edit);
            }
        }
        self.session.tool = self.tool;
        self.session.style = self.style;
        self.session.zoom = self.zoom;
    }

    fn text_popup(&mut self, ctx: &egui::Context, mut draft: TextDraft, view: View) {
        let (x, y) = image_to_screen(draft.at, view);
        let mut commit = false;
        egui::Window::new("editor-text")
            .fixed_pos(Pos2::new(x, y))
            .title_bar(false)
            .resizable(false)
            .show(ctx, |ui| {
                let response =
                    ui.add(egui::TextEdit::singleline(&mut draft.text).hint_text("Text"));
                if draft.armed && response.lost_focus() {
                    commit = true;
                } else {
                    response.request_focus();
                    draft.armed = true;
                }
            });
        self.session.text = Some(draft);
        if commit {
            if let Some(edit) = self.session.commit_text() {
                self.apply_edit(edit);
            }
        }
    }

    fn ensure_texture(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() && !self.reload_texture {
            return;
        }
        let (width, height, rgba) = preview_bytes(&self.doc);
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            return;
        }
        let image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        self.texture = Some(ctx.load_texture("snapper-editor", image, TextureOptions::LINEAR));
        self.reload_texture = false;
    }

    fn discard_dialog(&mut self, ctx: &egui::Context) {
        let mut discard = false;
        let mut keep = false;
        egui::Window::new("Discard edits?")
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Close the editor and discard unsaved annotations?");
                ui.horizontal(|ui| {
                    if ui.button("Discard").clicked() {
                        discard = true;
                    }
                    if ui.button("Keep editing").clicked() {
                        keep = true;
                    }
                });
            });
        if discard {
            self.dirty = false;
            self.confirm_close = false;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if keep {
            self.confirm_close = false;
        }
    }
}

fn shortcut_during_text(action: Action) -> bool {
    matches!(
        action,
        Action::CommitText
            | Action::Cancel
            | Action::Undo
            | Action::Redo
            | Action::Copy
            | Action::Save
    )
}

fn pixel_effects(doc: &Document) -> Vec<Item> {
    doc.items
        .iter()
        .filter(|item| matches!(item.shape, Shape::Blur { .. } | Shape::Pixelate { .. }))
        .cloned()
        .collect()
}

/// Base image, with blur and pixelate baked in.
///
/// Vector shapes stay as egui shapes. Flattening the whole document here
/// would paint those vectors into the texture and then again on top.
fn preview_bytes(doc: &Document) -> (u32, u32, Vec<u8>) {
    let effects = pixel_effects(doc);
    if effects.is_empty() {
        return (doc.base.width, doc.base.height, doc.base.rgba.clone());
    }
    let mut copy = doc.clone();
    copy.crop = None;
    copy.items = effects;
    let image = flattened(&copy);
    (image.width, image.height, image.rgba)
}

fn stroke_image_rect(ui: &egui::Ui, rect: snapper_editor::RectF, view: View, colour: Color32) {
    let (x, y) = image_to_screen(
        Point {
            x: rect.x,
            y: rect.y,
        },
        view,
    );
    let (x2, y2) = image_to_screen(
        Point {
            x: rect.x + rect.width,
            y: rect.y + rect.height,
        },
        view,
    );
    ui.painter().rect_stroke(
        Rect::from_min_max(Pos2::new(x, y), Pos2::new(x2, y2)),
        0.0,
        Stroke::new(1.0_f32, colour),
    );
}

fn paint_item(ui: &egui::Ui, item: &Item, view: View, selected: bool) {
    let painter = ui.painter();
    let colour = Color32::from_rgba_unmultiplied(
        item.style.stroke.r,
        item.style.stroke.g,
        item.style.stroke.b,
        item.style.stroke.a,
    );
    let stroke = Stroke::new(item.style.width.max(1.0) * view.zoom.max(0.2), colour);
    let screen = |point: Point| {
        let (x, y) = image_to_screen(point, view);
        Pos2::new(x, y)
    };
    match &item.shape {
        Shape::Arrow { from, to } | Shape::Line { from, to } => {
            painter.line_segment([screen(*from), screen(*to)], stroke);
            if matches!(item.shape, Shape::Arrow { .. }) {
                painter.circle_filled(screen(*to), 4.0 * view.zoom, colour);
            }
        }
        Shape::Rect { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => {
            let rect = rect.normalised();
            let drawn = Rect::from_min_max(
                screen(Point {
                    x: rect.x,
                    y: rect.y,
                }),
                screen(Point {
                    x: rect.x + rect.width,
                    y: rect.y + rect.height,
                }),
            );
            if matches!(item.shape, Shape::Highlight { .. }) {
                let [r, g, b, _] = colour.to_array();
                painter.rect_filled(drawn, 0.0, Color32::from_rgba_unmultiplied(r, g, b, 80));
            } else {
                painter.rect_stroke(drawn, 0.0, stroke);
            }
        }
        Shape::Ellipse { rect } => {
            let rect = rect.normalised();
            let centre = screen(Point {
                x: rect.x + rect.width * 0.5,
                y: rect.y + rect.height * 0.5,
            });
            let radius_x = rect.width * 0.5 * view.zoom;
            let radius_y = rect.height * 0.5 * view.zoom;
            let mut points = Vec::with_capacity(32);
            for step in 0..32 {
                let angle = step as f32 / 32.0 * std::f32::consts::TAU;
                points.push(Pos2::new(
                    centre.x + angle.cos() * radius_x,
                    centre.y + angle.sin() * radius_y,
                ));
            }
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        Shape::Text { at, text, size } => {
            painter.text(
                screen(*at),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(size * view.zoom),
                colour,
            );
        }
        Shape::Step { at, n } => {
            let centre = screen(*at);
            painter.circle_filled(centre, 14.0 * view.zoom, colour);
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                egui::FontId::proportional(14.0 * view.zoom),
                Color32::WHITE,
            );
        }
    }
    if selected {
        if let Some(bounds) = selection_bounds(item) {
            let drawn = Rect::from_min_max(
                screen(Point {
                    x: bounds.x,
                    y: bounds.y,
                }),
                screen(Point {
                    x: bounds.x + bounds.width,
                    y: bounds.y + bounds.height,
                }),
            );
            painter.rect_stroke(
                drawn,
                0.0,
                Stroke::new(1.0_f32, Color32::from_rgb(80, 160, 255)),
            );
        }
    }
}

fn selection_bounds(item: &Item) -> Option<snapper_editor::RectF> {
    match &item.shape {
        Shape::Rect { rect }
        | Shape::Ellipse { rect }
        | Shape::Highlight { rect }
        | Shape::Blur { rect, .. }
        | Shape::Pixelate { rect, .. } => Some(rect.normalised()),
        Shape::Arrow { from, to } | Shape::Line { from, to } => Some(snapper_editor::RectF {
            x: from.x.min(to.x),
            y: from.y.min(to.y),
            width: (from.x - to.x).abs().max(1.0),
            height: (from.y - to.y).abs().max(1.0),
        }),
        Shape::Text { at, size, .. } => Some(snapper_editor::RectF {
            x: at.x,
            y: at.y,
            width: size * 2.0,
            height: *size,
        }),
        Shape::Step { at, .. } => Some(snapper_editor::RectF {
            x: at.x - 14.0,
            y: at.y - 14.0,
            width: 28.0,
            height: 28.0,
        }),
    }
}

const TOOLS: [(Tool, &str); 10] = [
    (Tool::Select, "Select"),
    (Tool::Arrow, "Arrow"),
    (Tool::Rect, "Rect"),
    (Tool::Ellipse, "Ellipse"),
    (Tool::Highlight, "Highlight"),
    (Tool::Text, "Text"),
    (Tool::Step, "Step"),
    (Tool::Blur, "Blur"),
    (Tool::Pixelate, "Pixelate"),
    (Tool::Crop, "Crop"),
];

const KEYS: [(Key, CommandKey); 16] = [
    (Key::A, CommandKey::A),
    (Key::R, CommandKey::R),
    (Key::E, CommandKey::E),
    (Key::H, CommandKey::H),
    (Key::T, CommandKey::T),
    (Key::N, CommandKey::N),
    (Key::B, CommandKey::B),
    (Key::P, CommandKey::P),
    (Key::C, CommandKey::C),
    (Key::Z, CommandKey::Z),
    (Key::Y, CommandKey::Y),
    (Key::S, CommandKey::S),
    (Key::Delete, CommandKey::Delete),
    (Key::Backspace, CommandKey::Backspace),
    (Key::Enter, CommandKey::Enter),
    (Key::Escape, CommandKey::Escape),
];

#[cfg(test)]
mod preview_tests {
    use super::preview_bytes;
    use snapper_core::Image;
    use snapper_editor::{Document, Item, RectF, Shape, Style};

    fn checker() -> Image {
        let mut rgba = vec![0; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let value = if (x + y) % 2 == 0 { 0 } else { 255 };
                let index = (y * 8 + x) * 4;
                rgba[index] = value;
                rgba[index + 1] = value;
                rgba[index + 2] = value;
                rgba[index + 3] = 255;
            }
        }
        Image {
            width: 8,
            height: 8,
            scale_factor: 1.0,
            rgba,
        }
    }

    #[test]
    fn vector_shapes_stay_out_of_the_cached_texture() {
        let base = checker();
        let mut doc = Document::new(base.clone());
        doc.items.push(Item {
            id: 1,
            shape: Shape::Rect {
                rect: RectF {
                    x: 1.0,
                    y: 1.0,
                    width: 4.0,
                    height: 4.0,
                },
            },
            style: Style::default(),
        });
        let (_, _, rgba) = preview_bytes(&doc);
        assert_eq!(rgba, base.rgba);

        doc.items.push(Item {
            id: 2,
            shape: Shape::Pixelate {
                rect: RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                },
                block: 4,
            },
            style: Style::default(),
        });
        let (_, _, rgba) = preview_bytes(&doc);
        assert_ne!(rgba, base.rgba);
    }
}
