//! Cursor images from an ordered chain of pure providers (D20).
//!
//! A [`CursorProvider`] answers one question: for this theme, shape and
//! physical size, is there an image? It reads theme files only through a
//! [`CursorFiles`] it is handed, so none of the format code touches the
//! filesystem and the tests run against an in-memory theme tree. The
//! [`CursorChain`] asks the configured providers in order, runs each one under
//! `catch_unwind`, caches the answer, and falls back to the built-in arrow so
//! the pointer is never invisible.
//!
//! # Failure modes first
//!
//! 1. A missing theme draws the `default` theme instead, and the miss is logged
//!    once. `chain::tests::a_missing_theme_uses_the_default_theme`.
//! 2. A corrupt, oversized or inconsistent theme entry stops that provider with
//!    a named `Failed` and the next provider is asked.
//!    `chain::tests::a_failed_provider_hands_the_shape_to_the_next_one`.
//! 3. A panicking provider takes the pointer down: the panic is caught and
//!    counted as a failure. `chain::tests::a_panicking_provider_is_a_failure`.
//! 4. No provider has the shape: the built-in arrow answers.
//!    `chain::tests::with_no_provider_the_built_in_arrow_answers`.
//! 5. The cache serves a result for the wrong size or scale:
//!    `chain::tests::the_cache_is_keyed_by_size_and_scale`.
//! 6. The cache grows without bound: least recently used entries are evicted
//!    above the byte cap. `chain::tests::the_cache_evicts_the_least_recently_used`.
//! 7. A physical size above the cap reaches a provider:
//!    `chain::tests::the_physical_size_is_clamped_before_lookup`.
//! 8. A provider name that is not built in is accepted: `ProviderKind::from_name`
//!    returns `None` for it, and `gnoblin-config` refuses the name.
//!
//! # Deviation from the plan text
//!
//! The request carries a physical size, not a logical size and a scale. The
//! scale is folded in once by [`physical_size`], so no provider can apply it a
//! second time.

#![allow(dead_code, unused_imports)] // The compositor wiring lane consumes this API; until then nothing names it.

mod builtin;
mod chain;
mod files;
mod hyprcursor;
mod image;
mod selection;
mod shapes;
mod svg;
mod svg_scalable;
mod xcursor;

pub use builtin::arrow;
pub use chain::{CursorChain, physical_size};
pub use files::{CursorFiles, FileError, FsFiles, MAX_FILE_BYTES, MemoryFiles, SearchPath, is_contained};
pub use image::{CursorFrame, CursorImages, ImageError, MAX_IMAGE_EDGE, MAX_PHYSICAL_SIZE, pack_argb, premultiplied};
pub use shapes::candidates;
pub use svg::SvgError;

pub use selection::{
    CursorBackend, CursorBuild, CursorSelection, DEFAULT_CURSOR_SIZE, DEFAULT_CURSOR_THEME, InstalledTheme,
    MAX_CURSOR_SIZE, MIN_CURSOR_SIZE, select_cursor,
};

/// The most frames one animated cursor may have. Past this a theme entry is a
/// named failure, which bounds the work one shape can ask of a rasteriser.
pub const MAX_FRAMES: usize = 256;

/// One request to a provider: a theme, a shape and a physical pixel size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    /// The theme directory name, such as `Adwaita`.
    pub theme: &'a str,
    /// The cursor-spec or cursor-shape-v1 name, such as `default` or `wait`.
    pub shape: &'a str,
    /// The square edge the image must be drawn at, in physical pixels.
    pub physical: u32,
}

/// What one provider answered for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// The provider has the shape; the images are ready to draw.
    Found(CursorImages),
    /// The provider has no such theme or shape. The chain asks the next one.
    Miss,
    /// The provider has the shape but it cannot be used. The reason names the
    /// file, and the chain logs it and asks the next provider.
    Failed(String),
}

/// One provider in the chain. Implementations are pure: all reads go through
/// the [`CursorFiles`] they are given.
pub trait CursorProvider: Send + Sync {
    /// The name the configuration uses for this provider.
    fn kind(&self) -> ProviderKind;

    /// Looks up `request` in the theme files `files` exposes.
    fn lookup(&self, request: &Request<'_>, files: &dyn CursorFiles) -> Lookup;
}

/// The providers a configuration can name, in the names the configuration uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// Compiled Hyprcursor themes.
    Hyprcursor,
    /// `cursors_scalable` SVG themes with a `metadata.json` per shape.
    KdeGnomeSvg,
    /// Xcursor themes, with `index.theme` inheritance.
    Xcursor,
}

impl ProviderKind {
    /// Every provider this build has, in the default order of D20.
    pub const DEFAULT_ORDER: [Self; 3] = [Self::Hyprcursor, Self::KdeGnomeSvg, Self::Xcursor];

    /// The configuration name of this provider.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hyprcursor => "hyprcursor",
            Self::KdeGnomeSvg => "kde-gnome-svg",
            Self::Xcursor => "xcursor",
        }
    }

    /// The provider named `name`, or `None` for an unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "hyprcursor" => Some(Self::Hyprcursor),
            "kde-gnome-svg" => Some(Self::KdeGnomeSvg),
            "xcursor" => Some(Self::Xcursor),
            _ => None,
        }
    }

    /// Builds the provider for this kind.
    #[must_use]
    pub fn provider(self) -> Box<dyn CursorProvider> {
        match self {
            Self::Hyprcursor => Box::new(hyprcursor::Hyprcursor),
            Self::KdeGnomeSvg => Box::new(svg_scalable::KdeGnomeSvg),
            Self::Xcursor => Box::new(xcursor::Xcursor),
        }
    }
}

/// The nominal size a provider draws from for `physical`: the smallest offered
/// size at least `physical`, or the largest offered size when none is that big.
///
/// Returns `None` when `sizes` is empty.
#[must_use]
pub(crate) fn nominal_for(physical: u32, sizes: impl IntoIterator<Item = u32>) -> Option<u32> {
    let mut at_least: Option<u32> = None;
    let mut largest: Option<u32> = None;
    for size in sizes {
        largest = largest.max(Some(size));
        if size >= physical {
            at_least = Some(at_least.map_or(size, |best| best.min(size)));
        }
    }
    at_least.or(largest)
}

/// Scales one pixel length from `from` to `to` nominal pixels, rounding half up
/// and keeping at least one pixel.
#[must_use]
pub(crate) fn scale_length(value: u32, from: u32, to: u32) -> u32 {
    let scaled = (u64::from(value) * u64::from(to) + u64::from(from) / 2) / u64::from(from.max(1));
    scaled.clamp(1, u64::from(u32::MAX)) as u32
}

/// Scales one hotspot coordinate like a pixel length. Zero stays zero, so a
/// cursor whose hotspot sits at the edge keeps it there.
#[must_use]
pub(crate) fn scale_hotspot(value: u32, from: u32, to: u32) -> u32 {
    if value == 0 { 0 } else { scale_length(value, from, to) }
}
