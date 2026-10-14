//! CPU export of a [`Document`](crate::Document).
//!
//! Order: copy the base, rewrite blur and pixelate regions in place, draw the
//! vector shapes, paint text, then apply the crop.

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use snapper_core::Image;
use tiny_skia::{
    FillRule, IntSize, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::{Document, Item, Point, RectF, Rgba, Shape, Style};

const INTER: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");
const SHADOW_OFFSET: f32 = 3.0;

/// Flatten `doc` to a straight RGBA image.
pub fn flatten(doc: &Document) -> Image {
    let width = doc.base.width;
    let height = doc.base.height;
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if width == 0 || height == 0 || doc.base.rgba.len() != expected {
        return crop_image(doc.base.clone(), doc.crop);
    }

    let mut pixels = doc.base.rgba.clone();
    for item in &doc.items {
        match &item.shape {
            Shape::Blur { rect, radius } => {
                if let Some(bounds) = pixel_bounds(*rect, width, height) {
                    let radius = radius.round().max(0.0) as i32;
                    box_blur(&mut pixels, width, bounds, radius);
                }
            }
            Shape::Pixelate { rect, block } => {
                if let Some(bounds) = pixel_bounds(*rect, width, height) {
                    pixelate(&mut pixels, width, bounds, (*block).max(1));
                }
            }
            _ => {}
        }
    }

    if let Some(mut pixmap) = pixmap_from_straight(&pixels, width, height) {
        draw_vectors(
            &mut pixmap,
            &doc.items,
            positive_scale(doc.base.scale_factor),
        );
        pixels = demultiply(pixmap.data());
    }

    draw_labels(
        &mut pixels,
        width,
        height,
        &doc.items,
        positive_scale(doc.base.scale_factor),
    );

    crop_image(
        Image {
            width,
            height,
            scale_factor: doc.base.scale_factor,
            rgba: pixels,
        },
        doc.crop,
    )
}

fn positive_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

fn pixel_bounds(rect: RectF, width: u32, height: u32) -> Option<Bounds> {
    let rect = rect.normalised();
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return None;
    }
    let x0 = rect.x.floor().max(0.0) as i32;
    let y0 = rect.y.floor().max(0.0) as i32;
    let x1 = (rect.x + rect.width).ceil().min(width as f32).max(0.0) as i32;
    let y1 = (rect.y + rect.height).ceil().min(height as f32).max(0.0) as i32;
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Bounds { x0, y0, x1, y1 })
    }
}

/// Three separable box-blur passes, sampling only inside `bounds`.
fn box_blur(pixels: &mut [u8], width: u32, bounds: Bounds, radius: i32) {
    if radius <= 0 {
        return;
    }
    let mut src = pixels.to_vec();
    let mut dst = src.clone();
    for _ in 0..3 {
        blur_axis(&src, &mut dst, width, bounds, radius, true);
        blur_axis(&dst, &mut src, width, bounds, radius, false);
    }
    copy_bounds(pixels, &src, width, bounds);
}

fn blur_axis(
    src: &[u8],
    dst: &mut [u8],
    width: u32,
    bounds: Bounds,
    radius: i32,
    horizontal: bool,
) {
    dst.copy_from_slice(src);
    let x_start = bounds.x0;
    let x_end = bounds.x1;
    let y_start = bounds.y0;
    let y_end = bounds.y1;
    if x_end <= x_start || y_end <= y_start {
        return;
    }
    for y in y_start..y_end {
        for x in x_start..x_end {
            let mut sums = [0u32; 4];
            let mut count = 0u32;
            for offset in -radius..=radius {
                let (sx, sy) = if horizontal {
                    ((x + offset).clamp(x_start, x_end - 1), y)
                } else {
                    (x, (y + offset).clamp(y_start, y_end - 1))
                };
                let index = ((sy as u32 * width) + sx as u32) as usize * 4;
                for channel in 0..4 {
                    sums[channel] += src[index + channel] as u32;
                }
                count += 1;
            }
            let index = ((y as u32 * width) + x as u32) as usize * 4;
            for channel in 0..4 {
                dst[index + channel] = (sums[channel] / count) as u8;
            }
        }
    }
}

fn copy_bounds(dst: &mut [u8], src: &[u8], width: u32, bounds: Bounds) {
    for y in bounds.y0..bounds.y1 {
        let start = ((y as u32 * width) + bounds.x0 as u32) as usize * 4;
        let len = (bounds.x1 - bounds.x0) as usize * 4;
        dst[start..start + len].copy_from_slice(&src[start..start + len]);
    }
}

fn pixelate(pixels: &mut [u8], width: u32, bounds: Bounds, block: u32) {
    let block = block.max(1) as i32;
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let y1 = (y + block).min(bounds.y1);
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let x1 = (x + block).min(bounds.x1);
            let mut sums = [0u32; 4];
            let mut count = 0u32;
            for py in y..y1 {
                for px in x..x1 {
                    let index = ((py as u32 * width) + px as u32) as usize * 4;
                    for channel in 0..4 {
                        sums[channel] += pixels[index + channel] as u32;
                    }
                    count += 1;
                }
            }
            let average = [
                (sums[0] / count) as u8,
                (sums[1] / count) as u8,
                (sums[2] / count) as u8,
                (sums[3] / count) as u8,
            ];
            for py in y..y1 {
                for px in x..x1 {
                    let index = ((py as u32 * width) + px as u32) as usize * 4;
                    pixels[index..index + 4].copy_from_slice(&average);
                }
            }
            x = x1;
        }
        y = y1;
    }
}

fn pixmap_from_straight(pixels: &[u8], width: u32, height: u32) -> Option<Pixmap> {
    let size = IntSize::from_wh(width, height)?;
    Pixmap::from_vec(premultiply(pixels), size)
}

fn premultiply(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for pixel in out.chunks_mut(4) {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha) / 255) as u8;
    }
    out
}

fn demultiply(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for pixel in out.chunks_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
        } else {
            pixel[0] = ((u16::from(pixel[0]) * 255) / u16::from(alpha)).min(255) as u8;
            pixel[1] = ((u16::from(pixel[1]) * 255) / u16::from(alpha)).min(255) as u8;
            pixel[2] = ((u16::from(pixel[2]) * 255) / u16::from(alpha)).min(255) as u8;
        }
    }
    out
}

fn draw_vectors(pixmap: &mut Pixmap, items: &[Item], scale: f32) {
    for item in items {
        match &item.shape {
            Shape::Line { from, to } => {
                if let Some(path) = line_path(*from, *to) {
                    paint_path(pixmap, &path, &item.style, false, true);
                }
            }
            Shape::Arrow { from, to } => draw_arrow(pixmap, *from, *to, &item.style),
            Shape::Rect { rect } => {
                if let Some(path) = rect_path(*rect) {
                    paint_path(pixmap, &path, &item.style, true, true);
                }
            }
            Shape::Ellipse { rect } => {
                if let Some(path) = ellipse_path(*rect) {
                    paint_path(pixmap, &path, &item.style, true, true);
                }
            }
            Shape::Highlight { rect } => {
                if let Some(path) = rect_path(*rect) {
                    let mut style = item.style;
                    style.fill = Some(highlight_colour(&item.style));
                    style.width = 0.0;
                    paint_path(pixmap, &path, &style, true, false);
                }
            }
            Shape::Step { at, .. } => {
                let radius = 14.0 * scale;
                if let Some(path) = circle_path(*at, radius) {
                    let mut style = item.style;
                    style.fill = Some(item.style.fill.unwrap_or(item.style.stroke));
                    style.width = 0.0;
                    paint_path(pixmap, &path, &style, true, false);
                }
            }
            Shape::Text { .. } | Shape::Blur { .. } | Shape::Pixelate { .. } => {}
        }
    }
}

fn draw_arrow(pixmap: &mut Pixmap, from: Point, to: Point, style: &Style) {
    let length = arrow_head_length(style.width);
    let (line_end, head) = arrow_parts(from, to, length);
    if let Some(path) = line_path(from, line_end) {
        paint_path(pixmap, &path, style, false, true);
    }
    if let Some(path) = head {
        let mut filled = *style;
        filled.fill = Some(style.stroke);
        paint_path(pixmap, &path, &filled, true, false);
    }
}

pub(crate) fn arrow_head_length(width: f32) -> f32 {
    (width.abs() * 4.0).max(10.0)
}

fn arrow_parts(from: Point, to: Point, length: f32) -> (Point, Option<Path>) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let span = dx.hypot(dy);
    if span <= f32::EPSILON {
        return (from, None);
    }
    let ux = dx / span;
    let uy = dy / span;
    let head = length.min(span);
    let base = Point {
        x: to.x - ux * head,
        y: to.y - uy * head,
    };
    let half = head * 0.45;
    let left = Point {
        x: base.x + -uy * half,
        y: base.y + ux * half,
    };
    let right = Point {
        x: base.x - -uy * half,
        y: base.y - ux * half,
    };
    let mut builder = PathBuilder::new();
    builder.move_to(to.x, to.y);
    builder.line_to(left.x, left.y);
    builder.line_to(right.x, right.y);
    builder.close();
    (base, builder.finish())
}

fn paint_path(pixmap: &mut Pixmap, path: &Path, style: &Style, fill: bool, stroke: bool) {
    let shadow = Transform::from_translate(SHADOW_OFFSET, SHADOW_OFFSET);
    if style.shadow {
        let shadow_paint = paint_from(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 90,
        });
        if fill {
            pixmap.fill_path(path, &shadow_paint, FillRule::Winding, shadow, None);
        }
        if stroke && style.width > 0.0 {
            pixmap.stroke_path(path, &shadow_paint, &stroke_from(style.width), shadow, None);
        }
    }
    if fill {
        if let Some(colour) = style.fill {
            pixmap.fill_path(
                path,
                &paint_from(colour),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    if stroke && style.width > 0.0 {
        pixmap.stroke_path(
            path,
            &paint_from(style.stroke),
            &stroke_from(style.width),
            Transform::identity(),
            None,
        );
    }
}

fn paint_from(colour: Rgba) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color_rgba8(colour.r, colour.g, colour.b, colour.a);
    paint
}

fn stroke_from(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn line_path(from: Point, to: Point) -> Option<Path> {
    let mut builder = PathBuilder::new();
    builder.move_to(from.x, from.y);
    builder.line_to(to.x, to.y);
    builder.finish()
}

fn rect_path(rect: RectF) -> Option<Path> {
    let rect = rect.normalised();
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let mut builder = PathBuilder::new();
    let bounds = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)?;
    builder.push_rect(bounds);
    builder.finish()
}

fn circle_path(centre: Point, radius: f32) -> Option<Path> {
    if radius <= 0.0 {
        return None;
    }
    let mut builder = PathBuilder::new();
    builder.push_circle(centre.x, centre.y, radius);
    builder.finish()
}

fn ellipse_path(rect: RectF) -> Option<Path> {
    let rect = rect.normalised();
    let rx = rect.width * 0.5;
    let ry = rect.height * 0.5;
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    let cx = rect.x + rx;
    let cy = rect.y + ry;
    let kx = rx * 0.552_284_8;
    let ky = ry * 0.552_284_8;
    let mut builder = PathBuilder::new();
    builder.move_to(cx + rx, cy);
    builder.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    builder.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    builder.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    builder.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    builder.close();
    builder.finish()
}

fn highlight_colour(style: &Style) -> Rgba {
    if let Some(fill) = style.fill {
        return fill;
    }
    let alpha = if style.stroke.a == 0 {
        80
    } else {
        (u16::from(style.stroke.a) / 3).max(1) as u8
    };
    Rgba {
        a: alpha,
        ..style.stroke
    }
}

fn draw_labels(pixels: &mut [u8], width: u32, height: u32, items: &[Item], scale: f32) {
    let needs_text = items
        .iter()
        .any(|item| matches!(item.shape, Shape::Text { .. } | Shape::Step { .. }));
    if !needs_text {
        return;
    }
    let mut fonts = bundled_fonts();
    let mut cache = SwashCache::new();
    for item in items {
        match &item.shape {
            Shape::Text { at, text, size } => {
                draw_text(
                    pixels,
                    width,
                    height,
                    &mut fonts,
                    &mut cache,
                    text,
                    *at,
                    *size,
                    item.style.stroke,
                    false,
                    false,
                    item.style.shadow,
                );
            }
            Shape::Step { at, n } => {
                let radius = 14.0 * scale;
                let label = n.to_string();
                let size = match label.chars().count() {
                    1 => radius * 1.25,
                    2 => radius * 0.95,
                    _ => radius * 0.7,
                };
                let colour = contrast(item.style.fill.unwrap_or(item.style.stroke));
                draw_text(
                    pixels, width, height, &mut fonts, &mut cache, &label, *at, size, colour, true,
                    true, false,
                );
            }
            _ => {}
        }
    }
}

fn bundled_fonts() -> FontSystem {
    let mut database = cosmic_text::fontdb::Database::new();
    database.load_font_data(INTER.to_vec());
    database.set_sans_serif_family("Inter");
    database.set_serif_family("Inter");
    database.set_monospace_family("Inter");
    FontSystem::new_with_locale_and_db("en-US".to_string(), database)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
    text: &str,
    at: Point,
    size: f32,
    colour: Rgba,
    bold: bool,
    centre: bool,
    shadow: bool,
) {
    if text.is_empty() || !size.is_finite() || size <= 0.0 {
        return;
    }
    let metrics = Metrics::new(size, size * 1.2);
    let mut buffer = Buffer::new(fonts, metrics);
    let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
    let attrs = Attrs::new().family(Family::Name("Inter")).weight(weight);
    buffer.set_size(fonts, Some(4096.0), Some(size * 4.0));
    buffer.set_text(fonts, text, attrs, Shaping::Basic);
    buffer.shape_until_scroll(fonts, true);

    let Some((line_w, line_top, line_height)) = first_line_metrics(&buffer) else {
        return;
    };
    let (origin_x, visual_top) = if centre {
        (at.x - line_w * 0.5, at.y - line_height * 0.5)
    } else {
        (at.x, at.y)
    };
    let buffer_top = visual_top - line_top;
    let mut surface = Surface {
        pixels,
        width,
        height,
    };
    if shadow {
        blit_buffer(
            &mut surface,
            fonts,
            cache,
            &buffer,
            origin_x + SHADOW_OFFSET,
            buffer_top + SHADOW_OFFSET,
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 120,
            },
        );
    }
    blit_buffer(
        &mut surface,
        fonts,
        cache,
        &buffer,
        origin_x,
        buffer_top,
        colour,
    );
}

fn first_line_metrics(buffer: &Buffer) -> Option<(f32, f32, f32)> {
    let run = buffer.layout_runs().next()?;
    let height = if run.line_height > 0.0 {
        run.line_height
    } else {
        1.0
    };
    Some((run.line_w, run.line_top, height))
}

struct Surface<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
}

fn blit_buffer(
    surface: &mut Surface<'_>,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
    buffer: &Buffer,
    origin_x: f32,
    buffer_top: f32,
    colour: Rgba,
) {
    let text_colour = Color::rgba(colour.r, colour.g, colour.b, colour.a);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((origin_x, buffer_top + run.line_y), 1.0);
            let cache_key = physical.cache_key;
            let glyph_x = physical.x;
            let glyph_y = physical.y;
            cache.with_pixels(fonts, cache_key, text_colour, |x, y, pixel| {
                blend_at(
                    surface,
                    glyph_x + x,
                    glyph_y + y,
                    Rgba {
                        r: pixel.r(),
                        g: pixel.g(),
                        b: pixel.b(),
                        a: pixel.a(),
                    },
                );
            });
        }
    }
}

fn blend_at(surface: &mut Surface<'_>, x: i32, y: i32, colour: Rgba) {
    if colour.a == 0 || x < 0 || y < 0 || x as u32 >= surface.width || y as u32 >= surface.height {
        return;
    }
    let index = ((y as u32 * surface.width) + x as u32) as usize * 4;
    let dst = &mut surface.pixels[index..index + 4];
    if colour.a == 255 {
        dst[0] = colour.r;
        dst[1] = colour.g;
        dst[2] = colour.b;
        dst[3] = 255;
        return;
    }
    let src_a = u32::from(colour.a);
    let inv = 255 - src_a;
    let dst_a = u32::from(dst[3]);
    let out_a = src_a + dst_a * inv / 255;
    let src = [
        u32::from(colour.r),
        u32::from(colour.g),
        u32::from(colour.b),
    ];
    for channel in 0..3 {
        let mixed = src[channel] * src_a + u32::from(dst[channel]) * dst_a * inv / 255;
        dst[channel] = (mixed / out_a.max(1)).min(255) as u8;
    }
    dst[3] = out_a.min(255) as u8;
}

fn contrast(colour: Rgba) -> Rgba {
    let luminance =
        0.299 * f32::from(colour.r) + 0.587 * f32::from(colour.g) + 0.114 * f32::from(colour.b);
    if luminance > 160.0 {
        Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }
    } else {
        Rgba {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }
}

fn crop_image(image: Image, crop: Option<snapper_core::Rect>) -> Image {
    let Some(crop) = crop else {
        return image;
    };
    if image.width == 0 || image.height == 0 {
        return image;
    }
    let x0 = crop.x.max(0).min(image.width as i32) as u32;
    let y0 = crop.y.max(0).min(image.height as i32) as u32;
    let x1 = crop
        .x
        .saturating_add(crop.width as i32)
        .max(0)
        .min(image.width as i32) as u32;
    let y1 = crop
        .y
        .saturating_add(crop.height as i32)
        .max(0)
        .min(image.height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Image {
            width: 0,
            height: 0,
            scale_factor: image.scale_factor,
            rgba: Vec::new(),
        };
    }
    let out_w = x1 - x0;
    let out_h = y1 - y0;
    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];
    for row in 0..out_h {
        let src = (((y0 + row) * image.width + x0) * 4) as usize;
        let dst = (row * out_w * 4) as usize;
        let len = (out_w * 4) as usize;
        rgba[dst..dst + len].copy_from_slice(&image.rgba[src..src + len]);
    }
    Image {
        width: out_w,
        height: out_h,
        scale_factor: image.scale_factor,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edit, Point, RectF, Rgba, Shape, Style};

    fn solid(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        Image {
            width,
            height,
            scale_factor: 1.0,
            rgba,
        }
    }

    fn checker(width: u32, height: u32) -> Image {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                let value = if (x + y) % 2 == 0 { 0 } else { 255 };
                rgba[index] = value;
                rgba[index + 1] = value;
                rgba[index + 2] = value;
                rgba[index + 3] = 255;
            }
        }
        Image {
            width,
            height,
            scale_factor: 1.0,
            rgba,
        }
    }

    #[test]
    fn arrow_head_is_at_least_ten_pixels() {
        assert_eq!(arrow_head_length(1.0), 10.0);
        assert_eq!(arrow_head_length(4.0), 16.0);
        assert_eq!(arrow_head_length(3.0), 12.0);
    }

    #[test]
    fn flatten_without_items_keeps_opaque_pixels() {
        let mut rgba = Vec::new();
        for value in 0..24u8 {
            rgba.extend_from_slice(&[value, 255 - value, 10, 255]);
        }
        let base = Image {
            width: 6,
            height: 4,
            scale_factor: 1.5,
            rgba,
        };
        let out = flatten(&Document::new(base.clone()));
        assert_eq!(out.rgba, base.rgba);
        assert_eq!(out.scale_factor, 1.5);
    }

    #[test]
    fn blur_and_pixelate_remove_the_source_pixels() {
        let base = checker(24, 24);
        let mut blurred = Document::new(base.clone());
        blurred.items.push(Item {
            id: 1,
            shape: Shape::Blur {
                rect: RectF {
                    x: 2.0,
                    y: 2.0,
                    width: 16.0,
                    height: 16.0,
                },
                radius: 3.0,
            },
            style: Style::default(),
            auto: false,
        });
        let blurred = flatten(&blurred);
        for y in 2..18 {
            for x in 2..18 {
                let index = ((y * 24 + x) * 4) as usize;
                assert_ne!(
                    &blurred.rgba[index..index + 3],
                    &base.rgba[index..index + 3],
                    "blur kept the source pixel at {x},{y}"
                );
            }
        }

        let mut blocks = Document::new(base.clone());
        blocks.items.push(Item {
            id: 1,
            shape: Shape::Pixelate {
                rect: RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 24.0,
                    height: 24.0,
                },
                block: 4,
            },
            style: Style::default(),
            auto: false,
        });
        let blocks = flatten(&blocks);
        for y in 0..24 {
            for x in 0..24 {
                let index = ((y * 24 + x) * 4) as usize;
                assert_ne!(
                    &blocks.rgba[index..index + 3],
                    &base.rgba[index..index + 3],
                    "pixelate kept the source pixel at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn text_step_and_crop_change_the_export() {
        let mut doc = Document::new(solid(80, 60, [255, 255, 255, 255]));
        doc.items.push(Item {
            id: 1,
            shape: Shape::Text {
                at: Point { x: 8.0, y: 8.0 },
                text: "Hi".into(),
                size: 28.0,
            },
            style: Style {
                stroke: Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                ..Style::default()
            },
            auto: false,
        });
        doc.items.push(Item {
            id: 2,
            shape: Shape::Step {
                at: Point { x: 50.0, y: 30.0 },
                n: 4,
            },
            style: Style::default(),
            auto: false,
        });
        let image = flatten(&doc);
        assert!(
            image.rgba.chunks(4).any(|pixel| pixel[0] < 250),
            "text or the step marker should darken the white base"
        );
        let centre = ((30 * 80 + 50) * 4) as usize;
        assert!(image.rgba[centre] > 180 && image.rgba[centre + 2] < 120);

        doc.crop = Some(snapper_core::Rect {
            x: 40,
            y: 16,
            width: 20,
            height: 20,
        });
        let cropped = flatten(&doc);
        assert_eq!((cropped.width, cropped.height), (20, 20));
        assert_eq!(cropped.rgba.len(), 20 * 20 * 4);
    }

    #[test]
    fn crop_copies_the_chosen_pixels() {
        let mut rgba = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let index = ((y * 4 + x) * 4) as usize;
                rgba[index] = x as u8;
                rgba[index + 1] = y as u8;
                rgba[index + 3] = 255;
            }
        }
        let mut doc = Document::new(Image {
            width: 4,
            height: 4,
            scale_factor: 1.0,
            rgba,
        });
        doc.apply(Edit::Crop {
            before: None,
            after: Some(snapper_core::Rect {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
        });
        let out = flatten(&doc);
        assert_eq!((out.width, out.height), (2, 2));
        assert_eq!(&out.rgba[0..4], &[1, 1, 0, 255]);
    }
}
