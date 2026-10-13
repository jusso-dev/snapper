//! Frozen-screen selection overlay.
//!
//! The caller captures the desktop first, then [`run_overlay`] shows one
//! borderless viewport per monitor. The result is a physical-pixel region, a
//! window id, or cancel.

mod geometry;
mod ui;

pub use geometry::logical_to_physical;
pub use ui::run_overlay;

use egui::Pos2;
use snapper_capture::{window_at, MonitorInfo, WindowInfo};
use snapper_core::{Image, Rect};

use geometry::{
    clamp_rect, hit_resize_handle, nudge, physical_to_local, rect_from_points, resize_rect,
    scale_of, union_rect, ResizeHandle,
};

/// What the overlay hands back to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayResult {
    Region(Rect),
    Window(u64),
    Cancelled,
}

/// How the current pointer gesture should update the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    Hover,
    Dragging,
    Resizing(ResizeHandle),
}

/// Shared state for every monitor viewport.
pub struct OverlayState {
    pub frozen: Image,
    pub monitors: Vec<MonitorInfo>,
    pub windows: Vec<WindowInfo>,
    /// Physical corners of the gesture or the finished selection.
    pub drag: Option<(Pos2, Pos2)>,
    pub mode: OverlayMode,
    result: Option<OverlayResult>,
}

const CLICK_SLOP_PX: i32 = 4;

impl OverlayState {
    pub fn new(frozen: Image, monitors: Vec<MonitorInfo>, windows: Vec<WindowInfo>) -> Self {
        Self {
            frozen,
            monitors,
            windows,
            drag: None,
            mode: OverlayMode::Hover,
            result: None,
        }
    }

    pub fn result(&self) -> Option<&OverlayResult> {
        self.result.as_ref()
    }

    pub fn selection(&self) -> Option<Rect> {
        let (start, end) = self.drag?;
        let rect = rect_from_points(pos_i32(start), pos_i32(end));
        if rect.width == 0 || rect.height == 0 {
            None
        } else {
            Some(rect)
        }
    }

    /// Physical pixel under the local logical pointer, if it lands on a monitor.
    pub fn physical_at(
        &self,
        monitor_index: usize,
        local_x: f32,
        local_y: f32,
    ) -> Option<(i32, i32)> {
        let monitor = self.monitors.get(monitor_index)?;
        Some(logical_to_physical(monitor, local_x, local_y))
    }

    pub fn pointer_down(&mut self, monitor_index: usize, local_x: f32, local_y: f32) {
        if self.result.is_some() {
            return;
        }
        let Some((x, y)) = self.physical_at(monitor_index, local_x, local_y) else {
            return;
        };
        if let Some(rect) = self.selection() {
            let tolerance = self.handle_tolerance(monitor_index);
            if let Some(handle) = hit_resize_handle(rect, x, y, tolerance) {
                self.mode = OverlayMode::Resizing(handle);
                return;
            }
        }
        let point = Pos2::new(x as f32, y as f32);
        self.drag = Some((point, point));
        self.mode = OverlayMode::Dragging;
    }

    pub fn pointer_moved(&mut self, monitor_index: usize, local_x: f32, local_y: f32) {
        if self.result.is_some() {
            return;
        }
        let Some((x, y)) = self.physical_at(monitor_index, local_x, local_y) else {
            return;
        };
        match self.mode {
            OverlayMode::Dragging => {
                if let Some((start, _)) = self.drag {
                    self.drag = Some((start, Pos2::new(x as f32, y as f32)));
                }
            }
            OverlayMode::Resizing(handle) => {
                if let Some(rect) = self.selection() {
                    let resized = self.clamp(resize_rect(rect, handle, x, y));
                    self.drag = Some(corners(resized));
                }
            }
            OverlayMode::Hover => {}
        }
    }

    pub fn pointer_up(&mut self, monitor_index: usize, local_x: f32, local_y: f32) {
        if self.result.is_some() {
            return;
        }
        let Some((x, y)) = self.physical_at(monitor_index, local_x, local_y) else {
            self.mode = OverlayMode::Hover;
            return;
        };
        match self.mode {
            OverlayMode::Dragging => {
                if let Some((start, _)) = self.drag {
                    let (sx, sy) = pos_i32(start);
                    let slop = (x - sx).abs().max((y - sy).abs());
                    if slop <= CLICK_SLOP_PX {
                        self.drag = None;
                        if let Some(window) = window_at(&self.windows, x, y) {
                            self.result = Some(OverlayResult::Window(window.id));
                        }
                    } else {
                        self.drag = Some((start, Pos2::new(x as f32, y as f32)));
                    }
                }
            }
            OverlayMode::Resizing(handle) => {
                if let Some(rect) = self.selection() {
                    self.drag = Some(corners(self.clamp(resize_rect(rect, handle, x, y))));
                }
            }
            OverlayMode::Hover => {}
        }
        self.mode = OverlayMode::Hover;
    }

    pub fn double_click(&mut self, monitor_index: usize, local_x: f32, local_y: f32) {
        if self.result.is_some() {
            return;
        }
        if let Some(rect) = self.selection() {
            self.result = Some(OverlayResult::Region(rect));
            return;
        }
        if let Some((x, y)) = self.physical_at(monitor_index, local_x, local_y) {
            if let Some(window) = window_at(&self.windows, x, y) {
                self.result = Some(OverlayResult::Window(window.id));
            }
        }
    }

    pub fn confirm_key(&mut self) {
        if self.result.is_some() {
            return;
        }
        if let Some(rect) = self.selection() {
            self.result = Some(OverlayResult::Region(rect));
        }
    }

    pub fn cancel(&mut self) {
        self.result = Some(OverlayResult::Cancelled);
        self.mode = OverlayMode::Hover;
    }

    pub fn nudge_key(&mut self, dx: i32, dy: i32) {
        if self.result.is_some() {
            return;
        }
        let Some(rect) = self.selection() else {
            return;
        };
        let Some(bounds) = union_rect(&self.monitors) else {
            return;
        };
        self.drag = Some(corners(nudge(rect, dx, dy, bounds)));
        self.mode = OverlayMode::Hover;
    }

    /// Window under a hover point, used to draw the highlight. Hidden while dragging.
    pub fn highlight_window(
        &self,
        monitor_index: usize,
        local_x: f32,
        local_y: f32,
    ) -> Option<&WindowInfo> {
        if !matches!(self.mode, OverlayMode::Hover) || self.selection().is_some() {
            return None;
        }
        let (x, y) = self.physical_at(monitor_index, local_x, local_y)?;
        window_at(&self.windows, x, y)
    }

    pub fn local_selection(&self, monitor_index: usize) -> Option<(f32, f32, f32, f32)> {
        let monitor = self.monitors.get(monitor_index)?;
        let rect = self.selection()?;
        let (x, y) = physical_to_local(monitor, rect.x, rect.y);
        let scale = scale_of(monitor);
        Some((x, y, rect.width as f32 / scale, rect.height as f32 / scale))
    }

    fn handle_tolerance(&self, monitor_index: usize) -> i32 {
        let scale = self
            .monitors
            .get(monitor_index)
            .map(scale_of)
            .unwrap_or(1.0);
        (8.0 * scale).round().max(1.0) as i32
    }

    fn clamp(&self, rect: Rect) -> Rect {
        match union_rect(&self.monitors) {
            Some(bounds) => clamp_rect(rect, bounds),
            None => rect,
        }
    }
}

fn pos_i32(pos: Pos2) -> (i32, i32) {
    (pos.x.round() as i32, pos.y.round() as i32)
}

fn corners(rect: Rect) -> (Pos2, Pos2) {
    (
        Pos2::new(rect.x as f32, rect.y as f32),
        Pos2::new(
            rect.x.saturating_add(rect.width as i32) as f32,
            rect.y.saturating_add(rect.height as i32) as f32,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: u32, x: i32, y: i32, width: u32, height: u32, scale: f32) -> MonitorInfo {
        MonitorInfo {
            id,
            name: format!("m{id}"),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            scale_factor: scale,
            is_primary: id == 1,
        }
    }

    fn window(id: u64, x: i32, y: i32, width: u32, height: u32) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("w{id}"),
            app_name: "app".to_owned(),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            is_minimised: false,
        }
    }

    fn image() -> Image {
        Image {
            width: 4,
            height: 2,
            scale_factor: 1.0,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 1, 2, 3, 255, 4, 5, 6, 255, 7, 8,
                9, 255, 10, 11, 12, 255, 13, 14, 15, 255,
            ],
        }
    }

    fn state() -> OverlayState {
        OverlayState::new(
            image(),
            vec![
                monitor(1, 0, 0, 100, 80, 1.0),
                monitor(2, 100, 0, 200, 100, 2.0),
            ],
            vec![window(7, 10, 10, 20, 20), window(8, 120, 10, 40, 40)],
        )
    }

    #[test]
    fn drag_across_mixed_scale_monitors_is_a_physical_rect() {
        let mut overlay = state();
        overlay.pointer_down(0, 10.0, 15.0);
        overlay.pointer_moved(1, 5.0, 4.0);
        overlay.pointer_up(1, 5.0, 4.0);
        assert_eq!(
            overlay.selection(),
            Some(Rect {
                x: 10,
                y: 8,
                width: 100,
                height: 7,
            })
        );
        overlay.confirm_key();
        assert_eq!(
            overlay.result(),
            Some(&OverlayResult::Region(Rect {
                x: 10,
                y: 8,
                width: 100,
                height: 7,
            }))
        );
    }

    #[test]
    fn escape_cancels_and_a_click_selects_the_front_window() {
        let mut overlay = state();
        overlay.pointer_down(0, 12.0, 12.0);
        overlay.cancel();
        assert_eq!(overlay.result(), Some(&OverlayResult::Cancelled));

        let mut overlay = state();
        overlay.pointer_down(0, 12.0, 12.0);
        overlay.pointer_up(0, 13.0, 12.0);
        assert_eq!(overlay.result(), Some(&OverlayResult::Window(7)));
    }

    #[test]
    fn arrow_nudge_moves_the_selection_by_one_physical_pixel() {
        let mut overlay = state();
        overlay.pointer_down(0, 4.0, 4.0);
        overlay.pointer_moved(0, 20.0, 18.0);
        overlay.pointer_up(0, 20.0, 18.0);
        overlay.nudge_key(-1, 2);
        let rect = overlay.selection().expect("selection");
        assert_eq!(rect.x, 3);
        assert_eq!(rect.y, 6);
    }

    #[test]
    fn magnifier_sample_matches_the_frozen_pixel() {
        let overlay = state();
        let (x, y) = overlay.physical_at(0, 1.0, 0.0).expect("point");
        let rgb = geometry::sample_rgb(&overlay.frozen, 0, 0, x, y).expect("pixel");
        assert_eq!(rgb, [0, 255, 0]);
    }

    #[test]
    fn hover_highlight_uses_window_at_until_a_drag_starts() {
        let overlay = state();
        assert_eq!(
            overlay
                .highlight_window(0, 12.0, 12.0)
                .map(|window| window.id),
            Some(7)
        );
        assert_eq!(
            overlay
                .highlight_window(1, 15.0, 8.0)
                .map(|window| window.id),
            Some(8)
        );
        let mut overlay = overlay;
        overlay.pointer_down(0, 40.0, 40.0);
        overlay.pointer_moved(0, 60.0, 50.0);
        assert_eq!(overlay.highlight_window(0, 12.0, 12.0), None);
    }
}
