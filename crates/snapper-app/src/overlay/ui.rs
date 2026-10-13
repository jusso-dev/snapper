//! egui viewports for the frozen selection overlay.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use egui::{
    Color32, ColorImage, Context, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions,
    Vec2, ViewportBuilder, ViewportCommand, ViewportId,
};

use snapper_capture::MonitorInfo;
use snapper_core::Image;

use super::geometry::{self, format_hex, sample_rgb, union_rect};
use super::{OverlayResult, OverlayState};

const MAGNIFIER_LOGICAL: f32 = 120.0;
const MAGNIFIER_ZOOM: f32 = 8.0;

pub fn run_overlay(
    frozen: Image,
    monitors: Vec<MonitorInfo>,
    windows: Vec<snapper_capture::WindowInfo>,
) -> OverlayResult {
    if monitors.is_empty() {
        return OverlayResult::Cancelled;
    }
    let (tx, rx) = mpsc::channel();
    let state = Rc::new(RefCell::new(OverlayState::new(
        frozen,
        monitors.clone(),
        windows,
    )));
    let sent = Rc::new(Cell::new(false));
    let first = viewport_builder(&monitors[0]);
    let options = eframe::NativeOptions {
        viewport: first,
        persist_window: false,
        ..Default::default()
    };
    let app_state = state.clone();
    let app_sent = sent.clone();
    let _ = eframe::run_native(
        "Snapper",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(OverlayApp {
                state: app_state,
                tx,
                sent: app_sent,
                texture: Rc::new(RefCell::new(None)),
            }))
        }),
    );
    rx.try_recv().unwrap_or(OverlayResult::Cancelled)
}

struct OverlayApp {
    state: Rc<RefCell<OverlayState>>,
    tx: mpsc::Sender<OverlayResult>,
    sent: Rc<Cell<bool>>,
    texture: Rc<RefCell<Option<TextureHandle>>>,
}

impl eframe::App for OverlayApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        paint_monitor(ctx, &self.state, self.texture.as_ref(), 0);
        let count = self.state.borrow().monitors.len();
        for index in 1..count {
            let id = ViewportId::from_hash_of(("snapper-monitor", index));
            let builder = {
                let state = self.state.borrow();
                viewport_builder(&state.monitors[index])
            };
            let state = self.state.clone();
            let texture = Rc::clone(&self.texture);
            ctx.show_viewport_immediate(id, builder, move |ctx, _class| {
                paint_monitor(ctx, &state, texture.as_ref(), index);
            });
        }
        finish(ctx, &self.state, &self.tx, &self.sent);
    }
}

fn viewport_builder(monitor: &MonitorInfo) -> ViewportBuilder {
    let (x, y) = geometry::logical_origin(monitor);
    let (width, height) = geometry::logical_size(monitor);
    ViewportBuilder::default()
        .with_title("Snapper")
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_position(Pos2::new(x, y))
        .with_inner_size(Vec2::new(width.max(1.0), height.max(1.0)))
}

fn paint_monitor(
    ctx: &Context,
    state: &RefCell<OverlayState>,
    texture: &RefCell<Option<TextureHandle>>,
    index: usize,
) {
    ensure_texture(ctx, state, texture);
    handle_keys(ctx, state);
    let pointer = ctx.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.primary_pressed(),
            input.pointer.primary_released(),
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary),
        )
    });
    if let Some(pos) = pointer.0 {
        if pointer.3 {
            state.borrow_mut().double_click(index, pos.x, pos.y);
        } else if pointer.1 {
            state.borrow_mut().pointer_down(index, pos.x, pos.y);
        } else if pointer.2 {
            state.borrow_mut().pointer_up(index, pos.x, pos.y);
        } else if ctx.input(|input| input.pointer.primary_down()) {
            state.borrow_mut().pointer_moved(index, pos.x, pos.y);
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(Color32::BLACK))
        .show(ctx, |ui| {
            let painter = ui.painter();
            let full = ui.max_rect();
            if let Some(handle) = texture.borrow().as_ref() {
                if let Some(uv) = monitor_uv(&state.borrow(), index) {
                    painter.image(handle.id(), full, uv, Color32::from_white_alpha(128));
                }
            }
            if let Some((x, y, w, h)) = state.borrow().local_selection(index) {
                let selection = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
                if let (Some(handle), Some(uv)) = (
                    texture.borrow().as_ref(),
                    selection_uv(&state.borrow(), index),
                ) {
                    painter.image(handle.id(), selection, uv, Color32::WHITE);
                }
                painter.rect_stroke(selection, 0.0, Stroke::new(1.0_f32, Color32::WHITE));
            } else if let Some(pos) = pointer.0 {
                let highlight = {
                    let state = state.borrow();
                    state.highlight_window(index, pos.x, pos.y).map(|window| {
                        let monitor = &state.monitors[index];
                        let (x, y) =
                            geometry::physical_to_local(monitor, window.rect.x, window.rect.y);
                        let scale = geometry::scale_of(monitor);
                        Rect::from_min_size(
                            Pos2::new(x, y),
                            Vec2::new(
                                window.rect.width as f32 / scale,
                                window.rect.height as f32 / scale,
                            ),
                        )
                    })
                };
                if let Some(highlight) = highlight {
                    painter.rect_stroke(
                        highlight,
                        0.0,
                        Stroke::new(2.0_f32, Color32::from_rgb(80, 160, 255)),
                    );
                }
            }
            if let Some(pos) = pointer.0 {
                paint_magnifier(ui, state, index, pos);
            }
            // Eat the whole panel so the window keeps keyboard focus.
            ui.allocate_rect(full, Sense::click_and_drag());
        });
}

fn handle_keys(ctx: &Context, state: &RefCell<OverlayState>) {
    ctx.input(|input| {
        if input.key_pressed(Key::Escape) {
            state.borrow_mut().cancel();
        }
        if input.key_pressed(Key::Enter) {
            state.borrow_mut().confirm_key();
        }
        let (dx, dy) = if input.key_pressed(Key::ArrowLeft) {
            (-1, 0)
        } else if input.key_pressed(Key::ArrowRight) {
            (1, 0)
        } else if input.key_pressed(Key::ArrowUp) {
            (0, -1)
        } else if input.key_pressed(Key::ArrowDown) {
            (0, 1)
        } else {
            (0, 0)
        };
        if dx != 0 || dy != 0 {
            state.borrow_mut().nudge_key(dx, dy);
        }
    });
}

fn ensure_texture(
    ctx: &Context,
    state: &RefCell<OverlayState>,
    texture: &RefCell<Option<TextureHandle>>,
) {
    if texture.borrow().is_some() {
        return;
    }
    let state = state.borrow();
    let width = state.frozen.width as usize;
    let height = state.frozen.height as usize;
    if width == 0 || height == 0 || state.frozen.rgba.len() != width * height * 4 {
        return;
    }
    let image = ColorImage::from_rgba_unmultiplied([width, height], &state.frozen.rgba);
    *texture.borrow_mut() =
        Some(ctx.load_texture("snapper-frozen", image, TextureOptions::NEAREST));
}

fn monitor_uv(state: &OverlayState, index: usize) -> Option<Rect> {
    let monitor = state.monitors.get(index)?;
    let union = union_rect(&state.monitors)?;
    pixel_uv(
        &state.frozen,
        union.x,
        union.y,
        monitor.rect.x,
        monitor.rect.y,
        monitor.rect.width,
        monitor.rect.height,
    )
}

fn selection_uv(state: &OverlayState, index: usize) -> Option<Rect> {
    let monitor = state.monitors.get(index)?;
    let selection = state.selection()?;
    let union = union_rect(&state.monitors)?;
    let left = selection.x.max(monitor.rect.x);
    let top = selection.y.max(monitor.rect.y);
    let right = (selection.x.saturating_add(selection.width as i32))
        .min(monitor.rect.x.saturating_add(monitor.rect.width as i32));
    let bottom = (selection.y.saturating_add(selection.height as i32))
        .min(monitor.rect.y.saturating_add(monitor.rect.height as i32));
    if right <= left || bottom <= top {
        return None;
    }
    pixel_uv(
        &state.frozen,
        union.x,
        union.y,
        left,
        top,
        (right - left) as u32,
        (bottom - top) as u32,
    )
}

fn pixel_uv(
    image: &Image,
    origin_x: i32,
    origin_y: i32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Option<Rect> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let u0 = (x - origin_x) as f32 / image.width as f32;
    let v0 = (y - origin_y) as f32 / image.height as f32;
    let u1 = u0 + width as f32 / image.width as f32;
    let v1 = v0 + height as f32 / image.height as f32;
    Some(Rect::from_min_max(
        Pos2::new(u0.clamp(0.0, 1.0), v0.clamp(0.0, 1.0)),
        Pos2::new(u1.clamp(0.0, 1.0), v1.clamp(0.0, 1.0)),
    ))
}

fn paint_magnifier(ui: &egui::Ui, state: &RefCell<OverlayState>, index: usize, cursor: Pos2) {
    let (label, rgb) = {
        let state = state.borrow();
        let Some(monitor) = state.monitors.get(index) else {
            return;
        };
        let (px, py) = geometry::logical_to_physical(monitor, cursor.x, cursor.y);
        let union = union_rect(&state.monitors);
        let rgb = union.and_then(|origin| sample_rgb(&state.frozen, origin.x, origin.y, px, py));
        (format!("{px}, {py}"), rgb)
    };
    let (panel_x, panel_y) = {
        let state = state.borrow();
        let (width, height) = state
            .monitors
            .get(index)
            .map(geometry::logical_size)
            .unwrap_or((MAGNIFIER_LOGICAL, MAGNIFIER_LOGICAL));
        place_magnifier(cursor, width, height)
    };
    let panel = Rect::from_min_size(
        Pos2::new(panel_x, panel_y),
        Vec2::new(MAGNIFIER_LOGICAL, MAGNIFIER_LOGICAL),
    );
    let painter = ui.painter();
    painter.rect_filled(panel, 0.0, Color32::from_rgb(20, 20, 20));
    if let Some(rgb) = rgb {
        let cells = MAGNIFIER_LOGICAL / MAGNIFIER_ZOOM;
        let half = cells / 2.0;
        let state = state.borrow();
        let union = union_rect(&state.monitors);
        if let (Some(monitor), Some(origin)) = (state.monitors.get(index), union) {
            let (px, py) = geometry::logical_to_physical(monitor, cursor.x, cursor.y);
            for row in 0..cells as i32 {
                for col in 0..cells as i32 {
                    let sx = px + col - half as i32;
                    let sy = py + row - half as i32;
                    let Some(sample) = sample_rgb(&state.frozen, origin.x, origin.y, sx, sy) else {
                        continue;
                    };
                    let cell = Rect::from_min_size(
                        Pos2::new(
                            panel.left() + col as f32 * MAGNIFIER_ZOOM,
                            panel.top() + row as f32 * MAGNIFIER_ZOOM,
                        ),
                        Vec2::splat(MAGNIFIER_ZOOM),
                    );
                    painter.rect_filled(
                        cell,
                        0.0,
                        Color32::from_rgb(sample[0], sample[1], sample[2]),
                    );
                }
            }
        }
        let mid_x = panel.center().x;
        let mid_y = panel.center().y;
        painter.line_segment(
            [
                Pos2::new(panel.left(), mid_y),
                Pos2::new(panel.right(), mid_y),
            ],
            Stroke::new(1.0_f32, Color32::WHITE),
        );
        painter.line_segment(
            [
                Pos2::new(mid_x, panel.top()),
                Pos2::new(mid_x, panel.bottom()),
            ],
            Stroke::new(1.0_f32, Color32::WHITE),
        );
        painter.text(
            Pos2::new(panel.left() + 4.0, panel.bottom() - 4.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{label}  {}", format_hex(rgb)),
            egui::FontId::monospace(12.0),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(panel, 0.0, Stroke::new(1.0_f32, Color32::WHITE));
}

fn place_magnifier(cursor: Pos2, viewport_w: f32, viewport_h: f32) -> (f32, f32) {
    let mut x = cursor.x + 16.0;
    let mut y = cursor.y + 16.0;
    if x + MAGNIFIER_LOGICAL > viewport_w {
        x = cursor.x - 16.0 - MAGNIFIER_LOGICAL;
    }
    if y + MAGNIFIER_LOGICAL > viewport_h {
        y = cursor.y - 16.0 - MAGNIFIER_LOGICAL;
    }
    (x.max(0.0), y.max(0.0))
}

fn finish(
    ctx: &Context,
    state: &RefCell<OverlayState>,
    tx: &mpsc::Sender<OverlayResult>,
    sent: &Cell<bool>,
) {
    let result = state.borrow().result().cloned();
    if let Some(result) = result {
        if !sent.get() {
            let _ = tx.send(result);
            sent.set(true);
        }
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}
