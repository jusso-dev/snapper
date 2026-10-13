//! Logical and physical geometry for the selection overlay.
//!
//! Pointer positions arrive in a monitor's local logical pixels. Capture
//! results are physical pixels on the virtual desktop.

use snapper_capture::MonitorInfo;
use snapper_core::{Image, Rect};

/// Edge or corner that resizes an existing selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

/// Scale factor that is safe to divide by.
pub fn scale_of(monitor: &MonitorInfo) -> f32 {
    if monitor.scale_factor.is_finite() && monitor.scale_factor > 0.0 {
        monitor.scale_factor
    } else {
        1.0
    }
}

/// Top-left of the monitor in logical pixels, for placing its viewport.
pub fn logical_origin(monitor: &MonitorInfo) -> (f32, f32) {
    let scale = scale_of(monitor);
    (monitor.rect.x as f32 / scale, monitor.rect.y as f32 / scale)
}

/// Viewport size in logical pixels.
pub fn logical_size(monitor: &MonitorInfo) -> (f32, f32) {
    let scale = scale_of(monitor);
    (
        monitor.rect.width as f32 / scale,
        monitor.rect.height as f32 / scale,
    )
}

/// Local logical pointer position to physical virtual-desktop pixels.
pub fn logical_to_physical(monitor: &MonitorInfo, local_x: f32, local_y: f32) -> (i32, i32) {
    let scale = scale_of(monitor);
    let x = monitor.rect.x as f32 + local_x * scale;
    let y = monitor.rect.y as f32 + local_y * scale;
    (x.round() as i32, y.round() as i32)
}

/// Physical point to local logical pixels inside `monitor`.
pub fn physical_to_local(monitor: &MonitorInfo, x: i32, y: i32) -> (f32, f32) {
    let scale = scale_of(monitor);
    (
        (x - monitor.rect.x) as f32 / scale,
        (y - monitor.rect.y) as f32 / scale,
    )
}

/// Axis-aligned rect covering every monitor. Empty input returns `None`.
pub fn union_rect(monitors: &[MonitorInfo]) -> Option<Rect> {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for monitor in monitors {
        let right = monitor.rect.x.saturating_add(monitor.rect.width as i32);
        let bottom = monitor.rect.y.saturating_add(monitor.rect.height as i32);
        min_x = min_x.min(monitor.rect.x);
        min_y = min_y.min(monitor.rect.y);
        max_x = max_x.max(right);
        max_y = max_y.max(bottom);
    }
    if monitors.is_empty() {
        return None;
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x) as u32,
        height: max_y.saturating_sub(min_y) as u32,
    })
}

/// Normalised rectangle from two physical corners. The right and bottom are exclusive.
pub fn rect_from_points(a: (i32, i32), b: (i32, i32)) -> Rect {
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    let right = a.0.max(b.0);
    let bottom = a.1.max(b.1);
    Rect {
        x: left,
        y: top,
        width: right.saturating_sub(left) as u32,
        height: bottom.saturating_sub(top) as u32,
    }
}

/// Move `rect` inside `bounds`, shrinking it if it is larger than the bounds.
pub fn clamp_rect(rect: Rect, bounds: Rect) -> Rect {
    let bounds_right = bounds.x.saturating_add(bounds.width as i32);
    let bounds_bottom = bounds.y.saturating_add(bounds.height as i32);
    let mut width = rect.width.min(bounds.width);
    let mut height = rect.height.min(bounds.height);
    let mut x = rect.x;
    let mut y = rect.y;
    if x < bounds.x {
        x = bounds.x;
    }
    if y < bounds.y {
        y = bounds.y;
    }
    let right = x.saturating_add(width as i32);
    let bottom = y.saturating_add(height as i32);
    if right > bounds_right {
        x = bounds_right.saturating_sub(width as i32);
    }
    if bottom > bounds_bottom {
        y = bounds_bottom.saturating_sub(height as i32);
    }
    if x < bounds.x {
        width = bounds_right.saturating_sub(bounds.x) as u32;
        x = bounds.x;
    }
    if y < bounds.y {
        height = bounds_bottom.saturating_sub(bounds.y) as u32;
        y = bounds.y;
    }
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Which resize handle `point` hits, if any. Corners win over edges.
pub fn hit_resize_handle(rect: Rect, x: i32, y: i32, tolerance: i32) -> Option<ResizeHandle> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let left = rect.x;
    let top = rect.y;
    let right = rect.x.saturating_add(rect.width as i32);
    let bottom = rect.y.saturating_add(rect.height as i32);
    let near = |value: i32, edge: i32| (value - edge).abs() <= tolerance;
    let within_x = x >= left.saturating_sub(tolerance) && x <= right.saturating_add(tolerance);
    let within_y = y >= top.saturating_sub(tolerance) && y <= bottom.saturating_add(tolerance);
    if !within_x || !within_y {
        return None;
    }
    let at_left = near(x, left);
    let at_right = near(x, right);
    let at_top = near(y, top);
    let at_bottom = near(y, bottom);
    match (at_left, at_right, at_top, at_bottom) {
        (true, _, true, _) => Some(ResizeHandle::NorthWest),
        (_, true, true, _) => Some(ResizeHandle::NorthEast),
        (true, _, _, true) => Some(ResizeHandle::SouthWest),
        (_, true, _, true) => Some(ResizeHandle::SouthEast),
        (_, _, true, _) => Some(ResizeHandle::North),
        (_, _, _, true) => Some(ResizeHandle::South),
        (true, _, _, _) => Some(ResizeHandle::West),
        (_, true, _, _) => Some(ResizeHandle::East),
        _ => None,
    }
}

/// Move one edge of `rect` to `point`, then normalise.
pub fn resize_rect(rect: Rect, handle: ResizeHandle, x: i32, y: i32) -> Rect {
    let right = rect.x.saturating_add(rect.width as i32);
    let bottom = rect.y.saturating_add(rect.height as i32);
    let (a, b) = match handle {
        ResizeHandle::NorthWest => ((x, y), (right, bottom)),
        ResizeHandle::NorthEast => ((rect.x, y), (x, bottom)),
        ResizeHandle::SouthWest => ((x, rect.y), (right, y)),
        ResizeHandle::SouthEast => ((rect.x, rect.y), (x, y)),
        ResizeHandle::North => ((rect.x, y), (right, bottom)),
        ResizeHandle::South => ((rect.x, rect.y), (right, y)),
        ResizeHandle::West => ((x, rect.y), (right, bottom)),
        ResizeHandle::East => ((rect.x, rect.y), (x, bottom)),
    };
    rect_from_points(a, b)
}

/// Shift a selection by one physical pixel and keep it inside `bounds`.
pub fn nudge(rect: Rect, dx: i32, dy: i32, bounds: Rect) -> Rect {
    let moved = Rect {
        x: rect.x.saturating_add(dx),
        y: rect.y.saturating_add(dy),
        width: rect.width,
        height: rect.height,
    };
    clamp_rect(moved, bounds)
}

/// RGB at a physical pixel of a stitched image whose top-left is `origin`.
pub fn sample_rgb(image: &Image, origin_x: i32, origin_y: i32, x: i32, y: i32) -> Option<[u8; 3]> {
    let ix = i64::from(x) - i64::from(origin_x);
    let iy = i64::from(y) - i64::from(origin_y);
    if ix < 0 || iy < 0 || ix > i64::from(u32::MAX) || iy > i64::from(u32::MAX) {
        return None;
    }
    let ix = ix as u32;
    let iy = iy as u32;
    if ix >= image.width || iy >= image.height {
        return None;
    }
    let index = (iy as usize)
        .checked_mul(image.width as usize)?
        .checked_add(ix as usize)?
        .checked_mul(4)?;
    let pixel = image.rgba.get(index..index + 4)?;
    Some([pixel[0], pixel[1], pixel[2]])
}

/// `#RRGGBB` for the magnifier label.
pub fn format_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, width: u32, height: u32, scale: f32, id: u32) -> MonitorInfo {
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

    #[test]
    fn mixed_scale_monitors_share_one_physical_rect() {
        let left = monitor(0, 0, 1920, 1080, 1.0, 1);
        let right = monitor(1920, 0, 2560, 1440, 2.0, 2);
        let start = logical_to_physical(&left, 10.0, 20.0);
        let end = logical_to_physical(&right, 5.0, 6.0);
        assert_eq!(start, (10, 20));
        assert_eq!(end, (1920 + 10, 12));
        let selected = rect_from_points(start, end);
        assert_eq!(
            selected,
            Rect {
                x: 10,
                y: 12,
                width: 1920,
                height: 8,
            }
        );
        let union = union_rect(&[left, right]).expect("union");
        assert_eq!(
            union,
            Rect {
                x: 0,
                y: 0,
                width: 4480,
                height: 1440,
            }
        );
        assert_eq!(clamp_rect(selected, union), selected);
    }

    #[test]
    fn logical_viewport_uses_each_monitor_scale() {
        let hidpi = monitor(1920, -100, 200, 100, 2.0, 2);
        assert_eq!(logical_origin(&hidpi), (960.0, -50.0));
        assert_eq!(logical_size(&hidpi), (100.0, 50.0));
        assert_eq!(physical_to_local(&hidpi, 1930, -90), (5.0, 5.0));
    }

    #[test]
    fn clamp_pulls_a_rect_back_inside_the_desktop() {
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        };
        let hanging = Rect {
            x: -10,
            y: 70,
            width: 30,
            height: 20,
        };
        assert_eq!(
            clamp_rect(hanging, bounds),
            Rect {
                x: 0,
                y: 60,
                width: 30,
                height: 20,
            }
        );
        let huge = Rect {
            x: -5,
            y: -5,
            width: 500,
            height: 500,
        };
        assert_eq!(clamp_rect(huge, bounds), bounds);
    }

    #[test]
    fn resize_handles_prefer_corners_and_move_one_edge() {
        let rect = Rect {
            x: 10,
            y: 20,
            width: 40,
            height: 30,
        };
        assert_eq!(
            hit_resize_handle(rect, 10, 20, 3),
            Some(ResizeHandle::NorthWest)
        );
        assert_eq!(
            hit_resize_handle(rect, 30, 20, 2),
            Some(ResizeHandle::North)
        );
        assert_eq!(hit_resize_handle(rect, 30, 35, 2), None);
        let grown = resize_rect(rect, ResizeHandle::East, 80, 40);
        assert_eq!(grown.width, 70);
        assert_eq!(grown.height, 30);
        assert_eq!(grown.x, 10);
    }

    #[test]
    fn nudge_stops_at_the_union_edge() {
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 40,
        };
        let rect = Rect {
            x: 0,
            y: 1,
            width: 10,
            height: 10,
        };
        assert_eq!(nudge(rect, -5, 0, bounds).x, 0);
        assert_eq!(nudge(rect, 0, -5, bounds).y, 0);
        assert_eq!(nudge(rect, 100, 3, bounds).x, 40);
    }

    #[test]
    fn sample_rgb_reads_the_physical_pixel_and_formats_hex() {
        let mut rgba = vec![0u8; 2 * 2 * 4];
        rgba[4..8].copy_from_slice(&[0x12, 0xAB, 0x00, 255]);
        let image = Image {
            width: 2,
            height: 2,
            scale_factor: 1.0,
            rgba,
        };
        assert_eq!(sample_rgb(&image, -2, 5, -1, 5), Some([0x12, 0xAB, 0x00]));
        assert_eq!(format_hex([0x12, 0xAB, 0x00]), "#12AB00");
        assert_eq!(sample_rgb(&image, 0, 0, 9, 0), None);
    }

    #[test]
    fn empty_union_is_none() {
        assert_eq!(union_rect(&[]), None);
    }
}
