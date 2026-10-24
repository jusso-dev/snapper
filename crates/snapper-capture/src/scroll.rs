//! Stitch vertically scrolled screenshots into one tall image.
//!
//! Wayland is not handled here. Synthetic wheel events are not allowed on
//! Wayland, so the app refuses scrolling capture on that backend.

use snapper_core::Image;

/// Join `frames` by finding the vertical scroll between each pair.
///
/// `min_overlap` is the smallest band, in rows, that a candidate alignment
/// must cover. The search looks at the bottom 40 percent of the previous
/// frame (at least `min_overlap` rows) and slides it down the next frame.
///
/// A run of identical rows at the top of every frame is a sticky header and
/// is kept once. A run of identical rows at the bottom is a sticky footer and
/// is kept once, from the last frame.
///
/// An empty slice returns a zero-size image. One frame is returned unchanged.
pub fn stitch(frames: &[Image], min_overlap: u32) -> Image {
    let frames: Vec<&Image> = frames
        .iter()
        .filter(|frame| frame.width > 0 && frame.height > 0 && !frame.rgba.is_empty())
        .collect();
    if frames.is_empty() {
        return Image {
            width: 0,
            height: 0,
            scale_factor: 1.0,
            rgba: Vec::new(),
        };
    }
    if frames.len() == 1 {
        return (*frames[0]).clone();
    }

    let width = frames.iter().map(|frame| frame.width).min().unwrap_or(0);
    let scale = frames[0].scale_factor;
    let (header, footer) = sticky_bands(&frames, width);

    let mut rows: Vec<Vec<u8>> = Vec::new();
    let first_end = frames[0].height.saturating_sub(footer);
    push_rows(&mut rows, frames[0], width, 0, first_end);

    for pair in frames.windows(2) {
        let start = new_row_start(pair[0], pair[1], width, header, footer, min_overlap);
        let end = pair[1].height.saturating_sub(footer);
        if start < end {
            push_rows(&mut rows, pair[1], width, start, end);
        }
    }

    if footer > 0 {
        let last = frames[frames.len() - 1];
        let start = last.height.saturating_sub(footer);
        push_rows(&mut rows, last, width, start, last.height);
    }

    let height = rows.len() as u32;
    let mut rgba = Vec::with_capacity(rows.len() * width as usize * 4);
    for row in rows {
        rgba.extend_from_slice(&row);
    }
    Image {
        width,
        height,
        scale_factor: scale,
        rgba,
    }
}

fn sticky_bands(frames: &[&Image], width: u32) -> (u32, u32) {
    let min_height = frames.iter().map(|frame| frame.height).min().unwrap_or(0);
    let mut header = 0;
    while header < min_height && row_same(frames, width, header) {
        header += 1;
    }
    let mut footer = 0;
    while header + footer < min_height {
        let row = min_height - 1 - footer;
        if row_same(frames, width, row) {
            footer += 1;
        } else {
            break;
        }
    }
    (header, footer)
}

fn row_same(frames: &[&Image], width: u32, row: u32) -> bool {
    let Some(first) = row_bytes(frames[0], width, row) else {
        return false;
    };
    frames
        .iter()
        .skip(1)
        .all(|frame| row_bytes(frame, width, row) == Some(first))
}

fn new_row_start(
    prev: &Image,
    next: &Image,
    width: u32,
    header: u32,
    footer: u32,
    min_overlap: u32,
) -> u32 {
    let height = prev.height.min(next.height);
    let content_bottom = height.saturating_sub(footer.min(height));
    let content_top = header.min(content_bottom);
    let content_h = content_bottom.saturating_sub(content_top);
    if content_h == 0 {
        return content_bottom;
    }
    // The issue searches the bottom 40 percent. A scroll bigger than that has
    // no exact match on the wide band, so the band shrinks down to min_overlap.
    let mut max_band = (height * 2 / 5).max(min_overlap.max(1));
    if max_band > content_h {
        max_band = content_h;
    }
    let min_band = min_overlap.max(1).min(max_band);
    // Look for an exact match at every scroll distance: the widest band first,
    // then narrower bands for scrolls too big for it. Each check stops at the
    // first differing row, so a frame with no exact match stays cheap.
    let widest = content_h - max_band;
    let distances = (0..=widest).rev().chain(widest + 1..=content_h - min_band);
    for distance in distances {
        let band = max_band.min(content_h - distance);
        let next_row = content_bottom - band - distance;
        if band_sad(prev, next, width, content_bottom - band, next_row, band, 1) == 0 {
            return content_bottom - distance;
        }
    }
    let (offset, _) = align_band(prev, next, width, content_top, content_bottom, max_band);
    offset + max_band
}

fn align_band(
    prev: &Image,
    next: &Image,
    width: u32,
    content_top: u32,
    content_bottom: u32,
    band: u32,
) -> (u32, u64) {
    let height = prev.height.min(next.height);
    let band_prev = content_bottom - band;
    let last = content_bottom - band;
    let step = if height > 400 { 4 } else { 1 };

    let mut best_o = content_top;
    let mut best_sad = u64::MAX;
    let mut offset = content_top;
    while offset <= last {
        let sad = band_sad(prev, next, width, band_prev, offset, band, best_sad);
        if sad < best_sad {
            best_sad = sad;
            best_o = offset;
        }
        if sad == 0 {
            break;
        }
        let Some(next_offset) = offset.checked_add(step) else {
            break;
        };
        if next_offset > last {
            break;
        }
        offset = next_offset;
    }
    if step > 1 {
        let from = best_o.saturating_sub(step).max(content_top);
        let to = (best_o + step).min(last);
        for offset in from..=to {
            let sad = band_sad(prev, next, width, band_prev, offset, band, best_sad);
            if sad < best_sad {
                best_sad = sad;
                best_o = offset;
            }
        }
    }
    (best_o, best_sad)
}

fn band_sad(
    prev: &Image,
    next: &Image,
    width: u32,
    prev_row: u32,
    next_row: u32,
    band: u32,
    limit: u64,
) -> u64 {
    let mut sad = 0u64;
    for index in 0..band {
        let left = row_bytes(prev, width, prev_row + index);
        let right = row_bytes(next, width, next_row + index);
        match (left, right) {
            (Some(left), Some(right)) => sad = sad.saturating_add(row_sad(left, right)),
            _ => return u64::MAX,
        }
        // Past `limit` the caller has no use for the exact total.
        if sad >= limit {
            return sad;
        }
    }
    sad
}

fn row_sad(left: &[u8], right: &[u8]) -> u64 {
    let mut sad = 0u64;
    for (pixel_l, pixel_r) in left.chunks_exact(4).zip(right.chunks_exact(4)) {
        let left = luma(pixel_l[0], pixel_l[1], pixel_l[2]);
        let right = luma(pixel_r[0], pixel_r[1], pixel_r[2]);
        sad += u64::from(left.abs_diff(right));
    }
    sad
}

fn luma(red: u8, green: u8, blue: u8) -> u8 {
    let value = u16::from(red) * 30 + u16::from(green) * 59 + u16::from(blue) * 11;
    (value / 100) as u8
}

fn row_bytes(image: &Image, width: u32, row: u32) -> Option<&[u8]> {
    if row >= image.height || width == 0 {
        return None;
    }
    let start = (row as usize) * (image.width as usize) * 4;
    let len = width as usize * 4;
    image.rgba.get(start..start + len)
}

fn push_rows(rows: &mut Vec<Vec<u8>>, image: &Image, width: u32, start: u32, end: u32) {
    for row in start..end {
        if let Some(bytes) = row_bytes(image, width, row) {
            rows.push(bytes.to_vec());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_with_no_exact_match_stitch_quickly() {
        let noise = |seed: u32| {
            painted(640, 1000, move |x, y| {
                let value = (x.wrapping_mul(7919) ^ y.wrapping_mul(104_729) ^ seed)
                    .wrapping_mul(2_654_435_761)
                    >> 24;
                [value as u8, (value >> 1) as u8, 40, 255]
            })
        };
        let frames = [noise(1), noise(2), noise(3)];
        let started = std::time::Instant::now();
        let image = stitch(&frames, 40);
        assert!(image.height > 1000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "stitch took {:?}",
            started.elapsed()
        );
    }

    fn painted(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&pixel(x, y));
            }
        }
        Image {
            width,
            height,
            scale_factor: 2.0,
            rgba,
        }
    }

    fn unique(width: u32, height: u32) -> Image {
        painted(width, height, |x, y| {
            [
                (y % 251) as u8,
                (x.wrapping_mul(3).wrapping_add(y) % 251) as u8,
                (y.wrapping_mul(17) % 251) as u8,
                255,
            ]
        })
    }

    fn slice(image: &Image, start: u32, height: u32) -> Image {
        let mut rgba = Vec::with_capacity((image.width * height * 4) as usize);
        for row in start..start + height {
            let offset = (row * image.width * 4) as usize;
            let len = (image.width * 4) as usize;
            rgba.extend_from_slice(&image.rgba[offset..offset + len]);
        }
        Image {
            width: image.width,
            height,
            scale_factor: image.scale_factor,
            rgba,
        }
    }

    #[test]
    fn overlapping_slices_rebuild_the_tall_image() {
        let source = unique(6, 40);
        let frames = [
            slice(&source, 0, 16),
            slice(&source, 8, 16),
            slice(&source, 16, 16),
            slice(&source, 24, 16),
        ];
        let stitched = stitch(&frames, 4);
        assert_eq!(stitched.width, source.width);
        assert_eq!(stitched.height, source.height);
        assert_eq!(stitched.scale_factor, 2.0);
        assert_eq!(stitched.rgba, source.rgba);
    }

    #[test]
    fn sticky_header_is_kept_once() {
        let content = unique(4, 30);
        let header = [9u8, 8, 7, 255];
        let mut frames = Vec::new();
        for start in [0u32, 6, 12, 18] {
            let body = slice(&content, start, 12);
            let mut rgba = Vec::new();
            for _ in 0..3 {
                for _ in 0..body.width {
                    rgba.extend_from_slice(&header);
                }
            }
            rgba.extend_from_slice(&body.rgba);
            frames.push(Image {
                width: body.width,
                height: 15,
                scale_factor: 1.0,
                rgba,
            });
        }
        let stitched = stitch(&frames, 4);
        assert_eq!(stitched.height, 33, "3 header rows plus 30 content rows");
        let row_bytes = (stitched.width * 4) as usize;
        for row in 0..3 {
            let start = row * row_bytes;
            assert!(
                stitched.rgba[start..start + row_bytes]
                    .chunks(4)
                    .all(|pixel| pixel == header),
                "header row {row} was not kept"
            );
        }
        let content_out = &stitched.rgba[3 * row_bytes..];
        assert_eq!(content_out, content.rgba.as_slice());
    }

    #[test]
    fn sticky_footer_is_kept_once_at_the_end() {
        let content = unique(3, 20);
        let footer = [4u8, 5, 6, 255];
        let mut frames = Vec::new();
        for start in [0u32, 6, 12] {
            let body = slice(&content, start, 8);
            let mut rgba = body.rgba;
            for _ in 0..2 {
                for _ in 0..body.width {
                    rgba.extend_from_slice(&footer);
                }
            }
            frames.push(Image {
                width: body.width,
                height: 10,
                scale_factor: 1.0,
                rgba,
            });
        }
        let stitched = stitch(&frames, 2);
        assert_eq!(stitched.height, 22, "20 content rows plus 2 footer rows");
        let row_bytes = (stitched.width * 4) as usize;
        let content_out = &stitched.rgba[..20 * row_bytes];
        assert_eq!(content_out, content.rgba.as_slice());
        let tail = &stitched.rgba[20 * row_bytes..];
        assert!(tail.chunks(4).all(|pixel| pixel == footer));
    }

    #[test]
    fn identical_frames_are_not_repeated() {
        let frame = unique(4, 12);
        let stitched = stitch(&[frame.clone(), frame.clone(), frame.clone()], 3);
        assert_eq!(stitched.rgba, frame.rgba);
    }

    #[test]
    fn empty_input_is_an_empty_image() {
        let stitched = stitch(&[], 8);
        assert_eq!((stitched.width, stitched.height), (0, 0));
        assert!(stitched.rgba.is_empty());
    }
}
