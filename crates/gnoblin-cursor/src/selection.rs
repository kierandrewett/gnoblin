//! Cursor theme and size selection as a pure function.
//!
//! The configuration layer supplies what the user asked for and what is
//! installed; this module returns the one cursor the renderer should load. No
//! filesystem access happens here, which is what lets the selection rules be
//! tested without an icon theme on the machine.
//!
//! The rules come from the cursor guide:
//!
//! - `cursor.theme` names an installed theme, and an uninstalled name shows the
//!   default cursor instead. The fallback is the installed theme actually named
//!   `default`; if that is missing too, the compositor's built-in artwork is the
//!   cursor and [`CursorSelection::theme`] is `None`. Falling back to "any
//!   installed theme" would silently give a user with one downloaded theme that
//!   theme instead of a pointer.
//! - `cursor.size` is 1 to 256 logical pixels, default 24. A value outside the
//!   range is clamped rather than refused: configuration validation reports the
//!   range, and the selection function still has to return something usable if
//!   it is called with a stale or hand-edited value.
//! - Hyprcursor is used when the build supports it and the theme ships it,
//!   otherwise Xcursor, otherwise the theme is not usable.
//! - The output scale comes from `gnoblin-types` as an exact fraction, so the
//!   physical pixel size is computed with integer arithmetic and rounded once.
//!   The theme's own nominal sizes are matched against the *logical* request,
//!   because a theme declares its sizes at scale 1; matching them against the
//!   physical size would apply the scale twice.

use gnoblin_types::Scale;

/// The theme name used when none is configured.
pub const DEFAULT_CURSOR_THEME: &str = "default";

/// The logical size used when none is configured, in pixels.
pub const DEFAULT_CURSOR_SIZE: u16 = 24;

/// The smallest accepted logical size, in pixels.
pub const MIN_CURSOR_SIZE: u16 = 1;

/// The largest accepted logical size, in pixels.
pub const MAX_CURSOR_SIZE: u16 = 256;

/// One installed theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledTheme {
    /// The directory or theme name.
    pub name: String,
    /// Whether the theme contains Xcursor images.
    pub xcursor: bool,
    /// Whether the theme contains Hyprcursor images.
    pub hyprcursor: bool,
    /// The nominal sizes the theme declares, in pixels at scale 1.
    pub sizes: Vec<u16>,
}

/// Which loaders the running build includes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorBuild {
    /// Whether the build includes Hyprcursor support.
    pub hyprcursor: bool,
}

/// Where the cursor images come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorBackend {
    /// Hyprcursor images.
    Hyprcursor,
    /// Xcursor images.
    Xcursor,
    /// The compositor's built-in pointer artwork.
    Builtin,
}

/// The cursor the renderer should load.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorSelection {
    /// The installed theme, or `None` for the built-in pointer.
    pub theme: Option<String>,
    /// The image format.
    pub backend: CursorBackend,
    /// The requested logical size after clamping.
    pub logical_size: u16,
    /// The theme's nominal size that was selected, in pixels at scale 1.
    pub nominal_size: u16,
    /// The physical pixel size to load.
    pub physical_size: u32,
    /// The output scale that was applied.
    pub scale: Scale,
}

/// Choose the cursor for a configured theme name and logical size.
///
/// The two configured values are plain options rather than a request struct:
/// `gnoblin-config` already owns the configuration shape, and a second identical
/// struct here would be a duplicate of it.
#[must_use]
pub fn select_cursor(
    theme: Option<&str>,
    size: Option<u16>,
    installed: &[InstalledTheme],
    scale: Scale,
    build: CursorBuild,
) -> CursorSelection {
    let logical_size = size
        .unwrap_or(DEFAULT_CURSOR_SIZE)
        .clamp(MIN_CURSOR_SIZE, MAX_CURSOR_SIZE);
    let requested = theme
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_CURSOR_THEME);

    let usable =
        usable_theme(installed, requested, build).or_else(|| usable_theme(installed, DEFAULT_CURSOR_THEME, build));
    let (theme, backend, nominal_size) = match usable {
        Some((theme, backend)) => (
            Some(theme.name.clone()),
            backend,
            nearest_nominal_size(&theme.sizes, logical_size),
        ),
        None => (None, CursorBackend::Builtin, logical_size),
    };
    CursorSelection {
        theme,
        backend,
        logical_size,
        nominal_size,
        physical_size: physical_size(nominal_size, scale),
        scale,
    }
}

/// The physical pixel size for a logical size at a compositing scale.
///
/// `Scale` is an exact fraction of physical pixels per logical pixel, so the
/// multiplication and the half-up rounding stay in integers. The result is at
/// least one pixel: a cursor that loads at zero is not a cursor.
pub(super) fn physical_size(logical: u16, scale: Scale) -> u32 {
    let numerator = u64::from(logical) * u64::from(scale.numerator());
    let denominator = u64::from(scale.denominator());
    let rounded = (numerator + denominator / 2) / denominator;
    u32::try_from(rounded).unwrap_or(u32::MAX).max(1)
}

/// Find a theme by name and the loader it can be read with.
fn usable_theme<'a>(
    installed: &'a [InstalledTheme],
    name: &str,
    build: CursorBuild,
) -> Option<(&'a InstalledTheme, CursorBackend)> {
    let theme = installed.iter().find(|theme| theme.name == name)?;
    let backend = if build.hyprcursor && theme.hyprcursor {
        CursorBackend::Hyprcursor
    } else if theme.xcursor {
        CursorBackend::Xcursor
    } else {
        return None;
    };
    Some((theme, backend))
}

/// The theme size to load.
///
/// The smallest declared size at least as large as the request wins, because
/// downscaling a slightly larger cursor is sharper than upscaling a smaller one;
/// when the theme has nothing that large, its largest size is the closest it can
/// offer. An empty size list means the theme does not declare sizes, and the
/// requested size is passed through.
fn nearest_nominal_size(sizes: &[u16], requested: u16) -> u16 {
    let mut best: Option<u16> = None;
    for size in sizes.iter().copied() {
        match best {
            None => best = Some(size),
            Some(current) if rank(size, requested) < rank(current, requested) => best = Some(size),
            Some(_) => {}
        }
    }
    best.unwrap_or(requested)
}

/// A sort key: sizes at or above the request first, then by distance.
fn rank(size: u16, requested: u16) -> (u8, u16) {
    if size >= requested {
        (0, size - requested)
    } else {
        (1, requested - size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_types::Scale;

    fn theme(name: &str, xcursor: bool, hyprcursor: bool, sizes: &[u16]) -> InstalledTheme {
        InstalledTheme {
            name: name.to_owned(),
            xcursor,
            hyprcursor,
            sizes: sizes.to_vec(),
        }
    }

    #[test]
    fn a_missing_theme_falls_back_to_the_installed_default_theme() {
        let installed = vec![
            theme("default", true, false, &[24]),
            theme("SomeTheme", true, false, &[24]),
        ];
        let request = (Some("NotInstalled"), None);
        let selection = select_cursor(
            request.0,
            request.1,
            &installed,
            Scale::IDENTITY,
            CursorBuild::default(),
        );
        assert_eq!(selection.theme.as_deref(), Some("default"));
        assert_eq!(selection.backend, CursorBackend::Xcursor);
        assert_eq!(selection.logical_size, DEFAULT_CURSOR_SIZE);
    }

    #[test]
    fn with_nothing_installed_the_built_in_pointer_is_used() {
        let selection = select_cursor(None, None, &[], Scale::integral(2), CursorBuild::default());
        assert_eq!(selection.theme, None);
        assert_eq!(selection.backend, CursorBackend::Builtin);
        assert_eq!(selection.logical_size, DEFAULT_CURSOR_SIZE);
        assert_eq!(selection.physical_size, u32::from(DEFAULT_CURSOR_SIZE) * 2);
    }

    #[test]
    fn hyprcursor_is_preferred_only_when_the_build_supports_it() {
        let installed = vec![theme("default", true, false, &[24]), theme("Vector", true, true, &[24])];
        let request = (Some("Vector"), None);
        let supported = select_cursor(
            request.0,
            request.1,
            &installed,
            Scale::IDENTITY,
            CursorBuild { hyprcursor: true },
        );
        assert_eq!(supported.backend, CursorBackend::Hyprcursor);
        let unsupported = select_cursor(
            request.0,
            request.1,
            &installed,
            Scale::IDENTITY,
            CursorBuild::default(),
        );
        assert_eq!(unsupported.backend, CursorBackend::Xcursor);
        assert_eq!(unsupported.theme.as_deref(), Some("Vector"));
    }

    #[test]
    fn a_hyprcursor_only_theme_is_unusable_without_build_support() {
        let installed = vec![
            theme("default", true, false, &[24]),
            theme("Vector", false, true, &[24]),
        ];
        let request = (Some("Vector"), None);
        let selection = select_cursor(
            request.0,
            request.1,
            &installed,
            Scale::IDENTITY,
            CursorBuild::default(),
        );
        assert_eq!(selection.theme.as_deref(), Some("default"));
        assert_eq!(selection.backend, CursorBackend::Xcursor);
    }

    #[test]
    fn the_size_is_clamped_to_the_documented_range() {
        let installed = vec![theme("default", true, false, &[])];
        let too_small = select_cursor(None, Some(0), &installed, Scale::IDENTITY, CursorBuild::default());
        assert_eq!(too_small.logical_size, MIN_CURSOR_SIZE);
        let too_large = select_cursor(None, Some(4096), &installed, Scale::IDENTITY, CursorBuild::default());
        assert_eq!(too_large.logical_size, MAX_CURSOR_SIZE);
    }

    #[test]
    fn the_nearest_declared_size_prefers_one_at_least_as_large() {
        let installed = vec![theme("default", true, false, &[16, 24, 32, 48])];
        let at =
            |size| select_cursor(None, Some(size), &installed, Scale::IDENTITY, CursorBuild::default()).nominal_size;
        assert_eq!(at(20), 24);
        assert_eq!(at(24), 24);
        assert_eq!(at(100), 48);
        assert_eq!(at(8), 16);
    }

    #[test]
    fn the_output_scale_multiplies_the_selected_size_once() {
        let installed = vec![theme("default", true, false, &[24])];
        let selection = select_cursor(None, None, &installed, Scale::new(3, 2), CursorBuild::default());
        assert_eq!(selection.nominal_size, 24);
        assert_eq!(selection.physical_size, 36);
        assert_eq!(selection.scale, Scale::new(3, 2));
    }

    #[test]
    fn a_blank_theme_name_means_the_default() {
        let installed = vec![theme("default", true, false, &[24])];
        let request = (Some("   "), None);
        let selection = select_cursor(
            request.0,
            request.1,
            &installed,
            Scale::IDENTITY,
            CursorBuild::default(),
        );
        assert_eq!(selection.theme.as_deref(), Some("default"));
    }
}
