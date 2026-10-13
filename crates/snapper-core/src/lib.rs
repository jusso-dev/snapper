//! Shared types for Snapper.

pub mod settings;

pub use settings::Settings;

/// A rectangle in physical pixels on the virtual desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// What the user asked to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    FullScreen { monitor: Option<u32> },
    AllMonitors,
    Window { id: u64 },
    Region(Rect),
}

/// An RGBA8 image with its scale factor.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub rgba: Vec<u8>,
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::version().is_empty());
    }
}
