//! Capture backends. Real implementations land in M1 (see the backlog).

/// Errors a capture backend can return.
#[derive(Debug)]
pub enum CaptureError {
    Unsupported(&'static str),
    PermissionDenied,
}

/// Trait every platform backend implements.
pub trait Capturer {
    fn name(&self) -> &'static str;
}
