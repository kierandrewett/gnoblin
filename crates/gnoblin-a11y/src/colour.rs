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
/// have.
///
/// Provenance: the nine severity-1.0 numbers below were checked digit by digit
/// against `CVD_MATRICES_MACHADO2010` in colour-science,
/// `colour/blindness/datasets/machado2010.py`
/// (https://raw.githubusercontent.com/colour-science/colour/develop/colour/blindness/datasets/machado2010.py),
/// entries `Protanomaly`, `Deuteranomaly` and `Tritanomaly` at key `1.0`. That
/// file cites Machado's 2010 thesis (http://www.lume.ufrgs.br/handle/10183/26950),
/// not the 2009 TVCG paper directly; the TVCG PDF itself was not read.
///
/// These matrices SIMULATE colour blindness: they show what a viewer with the
/// deficiency sees. They are not a correction. Using them to recolour content
/// for a colour-blind viewer (daltonisation) is a different question, and is
/// not decided here.
const PROTANOPIA: Rows = [
    [0.152_286, 1.052_583, -0.204_868, 0.0, 0.0],
    [0.114_503, 0.786_281, 0.099_216, 0.0, 0.0],
    [-0.003_882, -0.048_116, 1.051_998, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

/// Machado et al. (2009), severity 1.0, deuteranopia. Simulation only, and
/// verified as described on [`PROTANOPIA`].
const DEUTERANOPIA: Rows = [
    [0.367_322, 0.860_646, -0.227_968, 0.0, 0.0],
    [0.280_085, 0.672_501, 0.047_413, 0.0, 0.0],
    [-0.011_820, 0.042_940, 0.968_881, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0, 0.0],
];

/// Machado et al. (2009), severity 1.0, tritanopia. Simulation only, and
/// verified as described on [`PROTANOPIA`].
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

    /// Applies a matrix to one colour (RGB, alpha 1) the way the renderer does.
    fn apply(m: super::Rows, c: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0_f32; 3];
        for (channel, slot) in out.iter_mut().enumerate() {
            let r = m[channel];
            *slot = r[0] * c[0] + r[1] * c[1] + r[2] * c[2] + r[3] + r[4];
        }
        out
    }

    #[test]
    fn invert_maps_black_to_white_and_back() {
        let m = ColourFilter::Invert.matrix();
        assert_eq!(apply(m, [0.0, 0.0, 0.0]), [1.0, 1.0, 1.0]);
        assert_eq!(apply(m, [1.0, 1.0, 1.0]), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn colour_blindness_filters_map_white_to_white() {
        for filter in [
            ColourFilter::Protanopia,
            ColourFilter::Deuteranopia,
            ColourFilter::Tritanopia,
        ] {
            let out = apply(filter.matrix(), [1.0, 1.0, 1.0]);
            for channel in out {
                assert!((channel - 1.0).abs() < 1e-3, "{filter:?} white became {out:?}");
            }
        }
    }

    /// Non-neutral samples: the colours that catch swapped or wrong coefficients,
    /// which white-to-white and the row sums cannot. The expected outputs are
    /// worked out by hand from the nine verified severity-1.0 digits, not by the
    /// code under test. For pure red the output is column 0 of the matrix, for
    /// pure green column 1, for pure blue column 2. For the mixed colour
    /// (0.8, 0.2, 0.5) each output is row `dot` (0.8, 0.2, 0.5), rounded to six
    /// places. The offsets and alpha are zero, so they add nothing.
    #[test]
    fn colour_blindness_filters_map_sample_colours() {
        let cases: [(ColourFilter, [[f32; 3]; 4]); 3] = [
            (
                // Protanopia digits: rows [0.152286, 1.052583, -0.204868],
                // [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998].
                ColourFilter::Protanopia,
                [
                    [0.152286, 0.114503, -0.003882],
                    [1.052583, 0.786281, -0.048116],
                    [-0.204868, 0.099216, 1.051998],
                    [0.229911, 0.298467, 0.513270],
                ],
            ),
            (
                // Deuteranopia digits: rows [0.367322, 0.860646, -0.227968],
                // [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881].
                ColourFilter::Deuteranopia,
                [
                    [0.367322, 0.280085, -0.011820],
                    [0.860646, 0.672501, 0.042940],
                    [-0.227968, 0.047413, 0.968881],
                    [0.352003, 0.382275, 0.483573],
                ],
            ),
            (
                // Tritanopia digits: rows [1.255528, -0.076749, -0.178779],
                // [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900].
                ColourFilter::Tritanopia,
                [
                    [1.255528, -0.078411, 0.004733],
                    [-0.076749, 0.930809, 0.691367],
                    [-0.178779, 0.147602, 0.303900],
                    [0.899683, 0.197234, 0.294010],
                ],
            ),
        ];
        // Input colours in order: pure red, pure green, pure blue, mixed.
        let inputs = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.8, 0.2, 0.5]];
        for (filter, expected) in cases {
            for (input, want) in inputs.iter().zip(expected.iter()) {
                let got = apply(filter.matrix(), *input);
                for (channel, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                    assert!(
                        (g - w).abs() < 1e-5,
                        "{filter:?} input {input:?} channel {channel}: got {g}, want {w}"
                    );
                }
            }
        }
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
