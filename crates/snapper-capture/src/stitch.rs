//! Virtual-desktop stitching in physical pixels.

use snapper_core::Image;

use crate::CaptureError;

/// One captured monitor, already in physical pixels, ready to blit.
pub struct PlacedCapture {
    /// Left edge on the virtual desktop.
    pub x: i32,
    /// Top edge on the virtual desktop.
    pub y: i32,
    pub image: Image,
}

/// Blit `parts` into one image covering their union rectangle.
///
/// Each part is copied to `(part.x - union.x, part.y - union.y)`. Parts must
/// not overlap; a later part overwrites an earlier one where they do. The
/// returned [`Image::scale_factor`] is `scale_factor` (the primary monitor's
/// scale when the caller has one).
pub fn stitch(parts: &[PlacedCapture], scale_factor: f32) -> Result<Image, CaptureError> {
    if parts.is_empty() {
        return Err(CaptureError::Backend("no monitors to stitch".to_owned()));
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for part in parts {
        let (right, bottom) = extent(part)?;
        min_x = min_x.min(part.x);
        min_y = min_y.min(part.y);
        max_x = max_x.max(right);
        max_y = max_y.max(bottom);
    }

    let width = u32::try_from(max_x - min_x)
        .map_err(|_| CaptureError::Backend("stitched width is negative or too large".to_owned()))?;
    let height = u32::try_from(max_y - min_y).map_err(|_| {
        CaptureError::Backend("stitched height is negative or too large".to_owned())
    })?;
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CaptureError::Backend("stitched image is too large".to_owned()))?;
    let mut rgba = vec![0u8; bytes];

    for part in parts {
        blit(&mut rgba, width, min_x, min_y, part)?;
    }

    Ok(Image {
        width,
        height,
        scale_factor,
        rgba,
    })
}

fn extent(part: &PlacedCapture) -> Result<(i32, i32), CaptureError> {
    let width = i32::try_from(part.image.width)
        .map_err(|_| CaptureError::Backend("monitor width exceeds i32".to_owned()))?;
    let height = i32::try_from(part.image.height)
        .map_err(|_| CaptureError::Backend("monitor height exceeds i32".to_owned()))?;
    let right = part
        .x
        .checked_add(width)
        .ok_or_else(|| CaptureError::Backend("monitor right edge overflows".to_owned()))?;
    let bottom = part
        .y
        .checked_add(height)
        .ok_or_else(|| CaptureError::Backend("monitor bottom edge overflows".to_owned()))?;
    Ok((right, bottom))
}

fn blit(
    canvas: &mut [u8],
    canvas_width: u32,
    origin_x: i32,
    origin_y: i32,
    part: &PlacedCapture,
) -> Result<(), CaptureError> {
    let expected = (part.image.width as usize)
        .checked_mul(part.image.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CaptureError::Backend("monitor image is too large".to_owned()))?;
    if part.image.rgba.len() != expected {
        return Err(CaptureError::Backend(format!(
            "monitor image at {},{} has {} bytes, expected {expected}",
            part.x,
            part.y,
            part.image.rgba.len()
        )));
    }

    let dest_x = u32::try_from(part.x - origin_x)
        .map_err(|_| CaptureError::Backend("monitor x is outside the union".to_owned()))?;
    let dest_y = u32::try_from(part.y - origin_y)
        .map_err(|_| CaptureError::Backend("monitor y is outside the union".to_owned()))?;
    if dest_x.saturating_add(part.image.width) > canvas_width {
        return Err(CaptureError::Backend(
            "monitor extends past the stitched width".to_owned(),
        ));
    }

    let row_bytes = part.image.width as usize * 4;
    for row in 0..part.image.height {
        let src = (row as usize) * row_bytes;
        let dst_row = (dest_y as usize + row as usize) * canvas_width as usize + dest_x as usize;
        let dst = dst_row * 4;
        canvas[dst..dst + row_bytes].copy_from_slice(&part.image.rgba[src..src + row_bytes]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, pixel: [u8; 4], scale: f32) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        Image {
            width,
            height,
            scale_factor: scale,
            rgba,
        }
    }

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * image.width + x) * 4) as usize;
        image.rgba[index..index + 4].try_into().expect("pixel")
    }

    #[test]
    fn side_by_side_monitors_have_no_gap_or_overlap() {
        // Physical pixels already include scale. A 2x monitor of 2x1 logical
        // pixels is 4x2 physical, and the next monitor starts immediately after it.
        let left = solid(4, 2, [255, 0, 0, 255], 2.0);
        let right = solid(2, 2, [0, 0, 255, 255], 1.0);
        let stitched = stitch(
            &[
                PlacedCapture {
                    x: 0,
                    y: 0,
                    image: left,
                },
                PlacedCapture {
                    x: 4,
                    y: 0,
                    image: right,
                },
            ],
            2.0,
        )
        .expect("stitch");

        assert_eq!(stitched.width, 6);
        assert_eq!(stitched.height, 2);
        assert_eq!(stitched.scale_factor, 2.0);
        for x in 0..4 {
            assert_eq!(pixel(&stitched, x, 0), [255, 0, 0, 255], "left column {x}");
            assert_eq!(pixel(&stitched, x, 1), [255, 0, 0, 255]);
        }
        for x in 4..6 {
            assert_eq!(pixel(&stitched, x, 0), [0, 0, 255, 255], "right column {x}");
            assert_eq!(pixel(&stitched, x, 1), [0, 0, 255, 255]);
        }
    }

    #[test]
    fn negative_origins_are_shifted_into_the_union() {
        let left = solid(2, 2, [1, 2, 3, 255], 1.0);
        let right = solid(1, 1, [9, 9, 9, 255], 1.5);
        let stitched = stitch(
            &[
                PlacedCapture {
                    x: -2,
                    y: -1,
                    image: left,
                },
                PlacedCapture {
                    x: 0,
                    y: 0,
                    image: right,
                },
            ],
            1.0,
        )
        .expect("stitch");

        assert_eq!((stitched.width, stitched.height), (3, 2));
        assert_eq!(pixel(&stitched, 0, 0), [1, 2, 3, 255]);
        assert_eq!(pixel(&stitched, 1, 0), [1, 2, 3, 255]);
        // The 1x1 monitor sits at virtual (0, 0), which is canvas (2, 1).
        assert_eq!(pixel(&stitched, 2, 1), [9, 9, 9, 255]);
        // Uncovered corner stays transparent black.
        assert_eq!(pixel(&stitched, 2, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn vertical_stack_uses_the_taller_width() {
        let top = solid(2, 1, [4, 0, 0, 255], 1.0);
        let bottom = solid(3, 2, [0, 4, 0, 255], 1.0);
        let stitched = stitch(
            &[
                PlacedCapture {
                    x: 10,
                    y: 20,
                    image: top,
                },
                PlacedCapture {
                    x: 10,
                    y: 21,
                    image: bottom,
                },
            ],
            1.0,
        )
        .expect("stitch");

        assert_eq!((stitched.width, stitched.height), (3, 3));
        assert_eq!(pixel(&stitched, 0, 0), [4, 0, 0, 255]);
        assert_eq!(pixel(&stitched, 1, 0), [4, 0, 0, 255]);
        assert_eq!(pixel(&stitched, 2, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&stitched, 2, 1), [0, 4, 0, 255]);
        assert_eq!(pixel(&stitched, 0, 2), [0, 4, 0, 255]);
    }

    #[test]
    fn empty_input_is_an_error() {
        let err = stitch(&[], 1.0).expect_err("empty");
        assert!(matches!(err, CaptureError::Backend(_)));
    }
}
