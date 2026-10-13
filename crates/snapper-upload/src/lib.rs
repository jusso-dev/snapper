//! Upload targets. See ADR 0004.

/// An upload destination.
pub trait UploadTarget {
    fn id(&self) -> &'static str;
}
