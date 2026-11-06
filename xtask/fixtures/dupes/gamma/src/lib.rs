//! Fixture crate: a near copy of `alpha::Layout` with one extra field.

/// Four shared fields and a fifth of its own: a Jaccard similarity of 0.8.
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub label: String,
}
