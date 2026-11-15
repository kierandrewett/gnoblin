//! Supersampling: composing an output at a multiple of its mode.
//!
//! An output driven at `mode` with scale `u` and supersample factor `s` is composed on a virtual surface of
//! `mode * s` pixels at the compositing scale `u * s`, then shown at `mode`. The logical size (`mode / u`) does not
//! change, so this module is only about the factor itself: how it is written, which values are recommended, and
//! whether a factor suits a mode (a whole virtual size, within the limit). See `design/supersample.md`.

use gnoblin_types::{Scale, Size};

use crate::mode::DisplayMode;

/// The largest virtual edge, in pixels, an output is composed at.
///
/// It bounds the memory a virtual surface may take (8192 x 8192 x 4 bytes is 256 MiB per target) and sits at or below
/// every GLES texture limit this compositor meets. A renderer with a smaller texture limit passes it to
/// [`virtual_size`] instead.
pub const MAX_VIRTUAL_EDGE: u32 = 8192;

/// The largest factor a request may name. A bigger one could not fit [`MAX_VIRTUAL_EDGE`] for any real mode.
pub const MAX_FACTOR: u32 = 16;

/// The factors the compositor recommends: whole powers of two give an exact box filter.
pub const RECOMMENDED: [u32; 3] = [1, 2, 4];

/// Why a supersample factor was refused.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum SupersampleError {
    /// The text is not a number from 1 to [`MAX_FACTOR`].
    #[error("`{text}` is not `default` or a number from 1 to {}", MAX_FACTOR)]
    NotAFactor {
        /// The text as written.
        text: String,
    },
    /// The factor does not give a whole number of pixels on an axis of the mode.
    #[error(
        "supersample {factor} on {width}x{height} gives a virtual {axis} of {fraction} pixels, which is not a whole number"
    )]
    NotWhole {
        /// The factor, as written.
        factor: String,
        /// The mode's width.
        width: u32,
        /// The mode's height.
        height: u32,
        /// `width` or `height`.
        axis: &'static str,
        /// The virtual extent that is not whole, as a decimal.
        fraction: String,
    },
    /// The virtual surface would be larger than the limit.
    #[error(
        "supersample {factor} on {width}x{height} needs a {virtual_width}x{virtual_height} surface, above the {limit}x{limit} limit"
    )]
    TooLarge {
        /// The factor, as written.
        factor: String,
        /// The mode's width.
        width: u32,
        /// The mode's height.
        height: u32,
        /// The virtual width the factor asks for.
        virtual_width: u64,
        /// The virtual height the factor asks for.
        virtual_height: u64,
        /// The limit on each edge.
        limit: u32,
    },
}

/// The factor as a person writes it: `2`, `1.5`.
#[must_use]
pub fn format_factor(factor: Scale) -> String {
    if factor.denominator() == 1 {
        factor.numerator().to_string()
    } else {
        let text = format!("{:.3}", factor.as_f64());
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Reads a factor: a decimal of at most three places from 1 to [`MAX_FACTOR`], exactly, so `1.5` is `3/2` and not a
/// float that happens to round.
///
/// # Errors
///
/// [`SupersampleError::NotAFactor`] for anything else, `NaN`, `inf`, a sign and a value below 1 included.
pub fn parse_factor(text: &str) -> Result<Scale, SupersampleError> {
    let refused = || SupersampleError::NotAFactor { text: text.to_owned() };
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    // A point must be followed by digits (`2.` is not a number), and three places is the precision a factor keeps.
    if !digits(whole) || (text.contains('.') && !digits(fraction)) || fraction.len() > 3 {
        return Err(refused());
    }
    let places = u32::try_from(fraction.len()).map_err(|_| refused())?;
    let divisor = 10u32.pow(places);
    let whole: u32 = whole.parse().map_err(|_| refused())?;
    let fraction: u32 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().map_err(|_| refused())?
    };
    let numerator = whole
        .checked_mul(divisor)
        .and_then(|scaled| scaled.checked_add(fraction))
        .ok_or_else(refused)?;
    if numerator == 0 {
        // `Scale::new` turns a zero into the identity scale, which would make `0` read as a factor of one.
        return Err(refused());
    }
    let factor = Scale::new(numerator, divisor);
    if factor < Scale::IDENTITY || factor > Scale::integral(MAX_FACTOR) {
        return Err(refused());
    }
    Ok(factor)
}

/// Whether the compositor recommends `factor`: 1, 2 or 4.
#[must_use]
pub fn is_recommended(factor: Scale) -> bool {
    RECOMMENDED.iter().any(|value| Scale::integral(*value) == factor)
}

/// The notice a factor that is not recommended earns, or [`None`] for one that is.
#[must_use]
pub fn notice(factor: Scale) -> Option<String> {
    (!is_recommended(factor)).then(|| {
        format!(
            "supersample {} is not recommended: use 1, 2 or 4",
            format_factor(factor)
        )
    })
}

/// The virtual size of `mode` at `factor`, within `limit` pixels on each edge.
///
/// # Errors
///
/// [`SupersampleError::NotWhole`] when an axis does not scale to a whole number of pixels, and
/// [`SupersampleError::TooLarge`] when an edge is above `limit`. Both name the numbers, so a person can see which
/// mode and factor to change.
pub fn virtual_size(mode: DisplayMode, factor: Scale, limit: u32) -> Result<Size, SupersampleError> {
    let (numerator, denominator) = (u64::from(factor.numerator()), u64::from(factor.denominator()));
    let axis = |extent: u32, name: &'static str| -> Result<u64, SupersampleError> {
        let scaled = u64::from(extent) * numerator;
        if scaled % denominator != 0 {
            return Err(SupersampleError::NotWhole {
                factor: format_factor(factor),
                width: mode.width,
                height: mode.height,
                axis: name,
                fraction: format!("{:.3}", scaled as f64 / denominator as f64)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned(),
            });
        }
        Ok(scaled / denominator)
    };
    let width = axis(mode.width, "width")?;
    let height = axis(mode.height, "height")?;
    if width > u64::from(limit) || height > u64::from(limit) {
        return Err(SupersampleError::TooLarge {
            factor: format_factor(factor),
            width: mode.width,
            height: mode.height,
            virtual_width: width,
            virtual_height: height,
            limit,
        });
    }
    // Both fit the limit, which is a `u32`.
    Ok(Size::new(
        u32::try_from(width).unwrap_or(limit),
        u32::try_from(height).unwrap_or(limit),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: DisplayMode = DisplayMode::new(1920, 1080, 60_000);

    #[test]
    fn a_factor_is_read_exactly() {
        assert_eq!(parse_factor("1"), Ok(Scale::IDENTITY));
        assert_eq!(parse_factor("2"), Ok(Scale::integral(2)));
        assert_eq!(parse_factor("1.5"), Ok(Scale::new(3, 2)));
        assert_eq!(parse_factor("2.250"), Ok(Scale::new(9, 4)));
        assert_eq!(parse_factor("16"), Ok(Scale::integral(16)));
    }

    #[test]
    fn what_is_not_a_factor_is_refused() {
        for text in [
            "", "0", "0.5", "0.999", "17", "-2", "+2", "nan", "inf", "1e1", "1.5.5", ".5", "2.", "1.2345", "two",
        ] {
            assert!(parse_factor(text).is_err(), "`{text}` must not be a factor");
        }
    }

    #[test]
    fn one_two_and_four_are_the_recommended_factors() {
        for value in [1, 2, 4] {
            assert!(is_recommended(Scale::integral(value)));
            assert_eq!(notice(Scale::integral(value)), None);
        }
        assert_eq!(
            notice(Scale::integral(3)).as_deref(),
            Some("supersample 3 is not recommended: use 1, 2 or 4")
        );
        assert_eq!(
            notice(Scale::new(3, 2)).as_deref(),
            Some("supersample 1.5 is not recommended: use 1, 2 or 4")
        );
    }

    #[test]
    fn the_virtual_size_is_the_mode_times_the_factor() {
        assert_eq!(
            virtual_size(FULL_HD, Scale::integral(2), MAX_VIRTUAL_EDGE),
            Ok(Size::new(3840, 2160))
        );
        assert_eq!(
            virtual_size(FULL_HD, Scale::new(3, 2), MAX_VIRTUAL_EDGE),
            Ok(Size::new(2880, 1620))
        );
    }

    #[test]
    fn a_factor_that_leaves_a_fraction_of_a_pixel_names_the_axis() {
        let error = virtual_size(DisplayMode::new(1367, 768, 60_000), Scale::new(3, 2), MAX_VIRTUAL_EDGE)
            .expect_err("1367 * 1.5 is 2050.5");
        assert!(
            matches!(&error, SupersampleError::NotWhole { axis: "width", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("2050.5"), "{error}");
    }

    #[test]
    fn a_surface_over_the_limit_names_the_numbers() {
        let error = virtual_size(
            DisplayMode::new(3840, 2160, 60_000),
            Scale::integral(4),
            MAX_VIRTUAL_EDGE,
        )
        .expect_err("15360 is above 8192");
        assert_eq!(
            error.to_string(),
            "supersample 4 on 3840x2160 needs a 15360x8640 surface, above the 8192x8192 limit"
        );
        assert!(
            virtual_size(
                DisplayMode::new(3840, 2160, 60_000),
                Scale::integral(2),
                MAX_VIRTUAL_EDGE
            )
            .is_ok()
        );
        assert!(virtual_size(FULL_HD, Scale::integral(4), MAX_VIRTUAL_EDGE).is_ok());
        assert!(
            virtual_size(FULL_HD, Scale::integral(4), 4096).is_err(),
            "a renderer's own limit is honoured"
        );
    }
}
