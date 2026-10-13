//! Golden PNG comparisons for the export renderer.
//!
//! Set `UPDATE_GOLDEN=1` to regenerate `tests/golden/*.png`.

use snapper_core::{Image, Rect};
use snapper_editor::{flatten, Document, Item, Point, RectF, Rgba, Shape, Style};

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

fn scene() -> Document {
    let mut doc = Document::new(solid(120, 80, [240, 240, 240, 255]));
    doc.items = vec![
        Item {
            id: 1,
            shape: Shape::Arrow {
                from: Point { x: 12.0, y: 64.0 },
                to: Point { x: 52.0, y: 18.0 },
            },
            style: Style {
                stroke: Rgba {
                    r: 226,
                    g: 61,
                    b: 61,
                    a: 255,
                },
                width: 4.0,
                shadow: true,
                fill: None,
            },
        },
        Item {
            id: 2,
            shape: Shape::Line {
                from: Point { x: 64.0, y: 18.0 },
                to: Point { x: 108.0, y: 18.0 },
            },
            style: Style {
                stroke: Rgba {
                    r: 20,
                    g: 90,
                    b: 200,
                    a: 255,
                },
                width: 3.0,
                ..Style::default()
            },
        },
        Item {
            id: 3,
            shape: Shape::Rect {
                rect: RectF {
                    x: 64.0,
                    y: 28.0,
                    width: 40.0,
                    height: 22.0,
                },
            },
            style: Style {
                stroke: Rgba {
                    r: 20,
                    g: 140,
                    b: 60,
                    a: 255,
                },
                fill: Some(Rgba {
                    r: 210,
                    g: 240,
                    b: 210,
                    a: 255,
                }),
                width: 2.0,
                shadow: false,
            },
        },
        Item {
            id: 4,
            shape: Shape::Ellipse {
                rect: RectF {
                    x: 8.0,
                    y: 8.0,
                    width: 36.0,
                    height: 22.0,
                },
            },
            style: Style {
                stroke: Rgba {
                    r: 20,
                    g: 20,
                    b: 20,
                    a: 255,
                },
                fill: Some(Rgba {
                    r: 250,
                    g: 220,
                    b: 60,
                    a: 255,
                }),
                width: 2.0,
                shadow: false,
            },
        },
        Item {
            id: 5,
            shape: Shape::Highlight {
                rect: RectF {
                    x: 40.0,
                    y: 36.0,
                    width: 28.0,
                    height: 14.0,
                },
            },
            style: Style {
                stroke: Rgba {
                    r: 255,
                    g: 220,
                    b: 0,
                    a: 255,
                },
                ..Style::default()
            },
        },
        Item {
            id: 6,
            shape: Shape::Text {
                at: Point { x: 68.0, y: 54.0 },
                text: "Ok".into(),
                size: 18.0,
            },
            style: Style {
                stroke: Rgba {
                    r: 10,
                    g: 10,
                    b: 10,
                    a: 255,
                },
                width: 1.0,
                ..Style::default()
            },
        },
        Item {
            id: 7,
            shape: Shape::Step {
                at: Point { x: 28.0, y: 56.0 },
                n: 1,
            },
            style: Style::default(),
        },
    ];
    doc
}

fn assert_golden(name: &str, image: &Image) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden dir")).expect("create golden dir");
        let buffer = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
            .expect("rgba buffer");
        buffer.save(&path).expect("write golden");
        return;
    }
    let expected = image::open(&path)
        .unwrap_or_else(|err| {
            panic!(
                "read {} (set UPDATE_GOLDEN=1 to create it): {err}",
                path.display()
            )
        })
        .to_rgba8();
    assert_eq!(
        (expected.width(), expected.height()),
        (image.width, image.height),
        "size mismatch for {}",
        path.display()
    );
    let raw = expected.as_raw();
    assert_eq!(raw.len(), image.rgba.len());
    for (index, (actual, expected)) in image.rgba.iter().zip(raw).enumerate() {
        let delta = i16::from(*actual) - i16::from(*expected);
        assert!(
            delta.abs() <= 2,
            "{} pixel {index}: actual {actual} expected {expected}",
            path.display()
        );
    }
}

#[test]
fn shapes_match_the_golden_png() {
    assert_golden("shapes.png", &flatten(&scene()));
}

#[test]
fn cropped_shapes_match_the_golden_png() {
    let mut doc = scene();
    doc.crop = Some(Rect {
        x: 8,
        y: 8,
        width: 48,
        height: 36,
    });
    assert_golden("crop.png", &flatten(&doc));
}
