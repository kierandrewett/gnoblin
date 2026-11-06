//! Fixture crate: declares the types the other fixture crates repeat.

/// Repeated by name in `beta`, and with the same fields under another name.
pub struct Widget {
    pub width: i32,
    pub height: i32,
}

/// Repeated with the same variant names under another name in `beta`.
pub enum Mode {
    Fast,
    Slow,
}

/// A four-field rectangle that `gamma::Frame` copies and extends.
pub struct Layout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
mod tests {
    /// Test-only helper: the scanner must skip `cfg(test)` modules.
    pub struct TestOnly {
        pub value: i32,
    }
}
