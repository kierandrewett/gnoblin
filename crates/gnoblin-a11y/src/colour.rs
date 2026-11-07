//! Colour filters: which named filter is on, and the colour matrix each one
//! stands for.
//!
//! This module is pure data. It returns the four rows of a colour matrix as
//! plain arrays, one row per output channel, each with four weights and an
//! offset, in the same layout the renderer's `ColourMatrix` uses. It does not
//! name that type, because this crate must not depend on a renderer; the
//! compositor wraps the rows when it builds the frame.
//!
//! Every matrix is applied to the colour the renderer produces, with the
//! channels in the order red, green, blue, alpha.

/// The four rows of a colour matrix: each row is `[r, g, b, a, offset]`.
pub type Rows = [[f32; 5]; 4];

/// A named colour filter. [`ColourFilter::None`] is the default and leaves
/// colours untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColourFilter {
    /// No filter: colours pass through unchanged.
    #[default]
    None,
    /// Inverts each colour channel and keeps alpha.
    Invert,
    /// Replaces each colour with its luma, using the Rec. 709 weights.
    Greyscale,
    /// Simulates protanopia (no working long-wavelength cones).
    Protanopia,
    /// Simulates deuteranopia (no working medium-wavelength cones).
    Deuteranopia,
    /// Simulates tritanopia (no working short-wavelength cones).
    Tritanopia,
}

/// The identity matrix: every colour unchanged.
const IDENTITY: Rows = [
    [1.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

/// Rec. 709 luma weights (ITU-R BT.709-6, section 3): 0.2126, 0.7152, 0.0722.
/// These are the published coefficients, not measured values.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Machado, Oliveira and Fernandes (2009), "A Physiologically-based Model for
/// Simulation of Color Vision Deficiency", IEEE TVCG 15(6), table for severity
/// 1.0 (full dichromacy). Their paper applies these to linear RGB. Gnoblin
/// applies them to the same values it renders, which is an approximation; a
/// gamma-correct pass would need a linearisation step the renderer does not
/// have. The numbers below were transcribed from the paper's published table
/// and have not been re-checked against the PDF in this change.
const PROTANOPIA: Rows = [
    [0.152_286, 1.052_583, -0.204_868, 0.0, 0.0],
    [0.114_503, 0.786_281, 0.099_216, 0.0, 0.0],
    [-0.003_882, -0.048_116, 1.051_998, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

/// Machado et al. (2009), severity 1.0, deuteranopia. Same provenance note as
/// [`PROTANOPIA`].
const DEUTERANOPIA: Rows = [
    [0.367_322, 0.860_646, -0.227_968, 0.0, 0.0],
    [0.280_085, 0.672_501, 0.047_413, 0.0, 0.0],
    [-0.011_820, 0.042_940, 0.968_881, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

/// Machado et al. (2009), severity 1.0, tritanopia. Same provenance note as
/// [`PROTANOPIA`].
const TRITANOPIA: Rows = [
    [1.255_528, -0.076_749, -0.178_779, 0.0, 0.0],
    [-0.078_411, 0.930_809, 0.147_602, 0.0, 0.0],
    [0.004_733, 0.691_367, 0.303_900, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

impl ColourFilter {
    /// The colour matrix rows this filter applies. [`ColourFilter::None`] gives
    /// the identity, so a caller can always apply the result without a special
    /// case for "off".
    #[must_use]
    pub const fn matrix(self) -> Rows {
        match self {
            Self::None => IDENTITY,
            Self::Invert => [
                [-1.0, 0.0, 0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
            Self::Greyscale => [
                [LUMA[0], LUMA[1], LUMA[2], 0.0, 0.0],
                [LUMA[0], LUMA[1], LUMA[2], 0.0, 0.0],
                [LUMA[0], LUMA[1], LUMA[2], 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
            Self::Protanopia => PROTANOPIA,
            Self::Deuteranopia => DEUTERANOPIA,
            Self::Tritanopia => TRITANOPIA,
        }
    }
}

#[cfg(test)]
mod colour_tests {
    use super::{ColourFilter, IDENTITY};

    /// Compares two matrices within a tolerance, because the colour-blindness
    /// rows are decimals and float equality would only test the literals.
    fn close(actual: super::Rows, expected: super::Rows) {
        for (row, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            for (col, (x, y)) in a.iter().zip(e.iter()).enumerate() {
                assert!((x - y).abs() < 1e-6, "row {row} col {col}: {x} != {y}");
            }
        }
    }

    #[test]
    fn none_is_the_identity() {
        close(ColourFilter::None.matrix(), IDENTITY);
        assert_eq!(ColourFilter::default(), ColourFilter::None);
    }

    #[test]
    fn invert_negates_colour_and_keeps_alpha() {
        close(
            ColourFilter::Invert.matrix(),
            [
                [-1.0, 0.0, 0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        );
    }

    #[test]
    fn greyscale_maps_every_channel_to_rec709_luma() {
        let m = ColourFilter::Greyscale.matrix();
        for row in &m[..3] {
            assert!((row[0] - 0.2126).abs() < 1e-6);
            assert!((row[1] - 0.7152).abs() < 1e-6);
            assert!((row[2] - 0.0722).abs() < 1e-6);
            assert!(row[3].abs() < 1e-6 && row[4].abs() < 1e-6);
        }
        // Each luma weight set sums to one, so white stays white.
        let sum: f32 = m[0][..3].iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn colour_blindness_filters_keep_white_white() {
        // Each Machado row sums to about one, so white (1, 1, 1) maps to white.
        for filter in [
            ColourFilter::Protanopia,
            ColourFilter::Deuteranopia,
            ColourFilter::Tritanopia,
        ] {
            let m = filter.matrix();
            for row in &m[..3] {
                let sum: f32 = row[..3].iter().sum();
                assert!((sum - 1.0).abs() < 1e-3, "{filter:?} row sums to {sum}");
            }
        }
    }

    #[test]
    fn protanopia_table_values() {
        close(
            ColourFilter::Protanopia.matrix(),
            [
                [0.152_286, 1.052_583, -0.204_868, 0.0, 0.0],
                [0.114_503, 0.786_281, 0.099_216, 0.0, 0.0],
                [-0.003_882, -0.048_116, 1.051_998, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        );
    }

    #[test]
    fn deuteranopia_table_values() {
        close(
            ColourFilter::Deuteranopia.matrix(),
            [
                [0.367_322, 0.860_646, -0.227_968, 0.0, 0.0],
                [0.280_085, 0.672_501, 0.047_413, 0.0, 0.0],
                [-0.011_820, 0.042_940, 0.968_881, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        );
    }

    #[test]
    fn tritanopia_table_values() {
        close(
            ColourFilter::Tritanopia.matrix(),
            [
                [1.255_528, -0.076_749, -0.178_779, 0.0, 0.0],
                [-0.078_411, 0.930_809, 0.147_602, 0.0, 0.0],
                [0.004_733, 0.691_367, 0.303_900, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        );
    }

    #[test]
    fn every_filter_leaves_alpha_alone() {
        for filter in [
            ColourFilter::None,
            ColourFilter::Invert,
            ColourFilter::Greyscale,
            ColourFilter::Protanopia,
            ColourFilter::Deuteranopia,
            ColourFilter::Tritanopia,
        ] {
            let m = filter.matrix();
            assert_eq!(m[3], [0.0, 0.0, 0.0, 1.0, 0.0], "{filter:?} changes alpha");
        }
    }
}
