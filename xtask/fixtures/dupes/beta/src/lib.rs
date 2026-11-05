//! Fixture crate: repeats `alpha`'s declarations under other names.

/// Same name as `alpha::Widget`, different fields.
pub struct Widget {
    pub depth: u8,
}

/// Same fields as `alpha::Widget`, different name.
pub struct Dimensions {
    pub width: i32,
    pub height: i32,
}

/// Same variant names as `alpha::Mode`, different name.
pub enum Kind {
    Fast,
    Slow,
}
