//! The gamma lookup table: what it is made of, and when it must not be uploaded.
//!
//! A KMS CRTC applies its gamma table to every pixel it scans out, which is how night light
//! (a colour temperature shift) and a brightness or contrast curve reach the display without
//! touching a single frame. The table's *length* is a property of the CRTC, not of the
//! request ([`CRTC::gamma_length`](drm::control::crtc::Info::gamma_length)), and the kernel
//! refuses a table of any other length.
//!
//! That refusal is the failure this module exists to prevent: the kernel's answer is a bare
//! `EINVAL` from `DRM_IOCTL_MODE_SETGAMMA`, which arrives at the backend as "the driver
//! refused an operation" and says nothing about the length that was wrong. So the table is
//! built for a stated length and checked against the CRTC's before the ioctl
//! ([`GammaRamp::matches`]), and a mismatch is a named error with both lengths
//! ([`GammaError::Length`]).
//!
//! Nothing here touches a device: a ramp is arithmetic on `u16`s, which is why it is
//! unit-tested on a machine with no display.
//!
//! # Failure modes first
//!
//! 1. **A table of the wrong length is uploaded.** [`GammaRamp::matches`] refuses it with
//!    both lengths named, before any ioctl: `a_table_of_the_wrong_length_is_refused`.
//! 2. **An empty table is uploaded as "no gamma".** A CRTC with no gamma table cannot be
//!    driven, so a length of zero is [`GammaError::Degenerate`] rather than a silent
//!    identity: `an_empty_table_is_refused_rather_than_ignored`.
//! 3. **A colour temperature leaves the display brighter than it was.** The multipliers are
//!    normalised so the brightest channel stays at full scale, so night light cannot raise
//!    the panel's peak: `a_warm_temperature_dims_blue_and_keeps_the_peak`.
//! 4. **Two temperatures produce the same table.** The shift is computed in floating point
//!    and rounded once, in the direction that keeps a channel inside its range:
//!    `a_cooler_temperature_leaves_more_blue`.

use smithay::reexports::drm::control::crtc;

/// How many entries a gamma table has when the caller does not say.
///
/// It is the length the kernel offers for most CRTCs (`256`), and it is only a default: the
/// CRTC's own length is what a table is checked against.
pub const DEFAULT_LENGTH: usize = 256;

/// The coldest temperature the shift is computed for, in kelvin.
///
/// Below it the blackbody curve's approximation stops being one, and a display driven at
/// 1000 K is unusable anyway.
pub const MIN_KELVIN: u32 = 1000;

/// The warmest temperature at which the shift is the identity, in kelvin.
///
/// At and above it the table is the linear ramp: a display asked for its own white point
/// must not be tinted by rounding.
pub const NEUTRAL_KELVIN: u32 = 6500;

/// A gamma lookup table: one channel per colour, all of the same length.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GammaRamp {
    /// The requested length, which is what [`GammaRamp::matches`] compares.
    length: usize,
    /// The red channel, `length` entries from black to full scale.
    red: Vec<u16>,
    /// The green channel.
    green: Vec<u16>,
    /// The blue channel.
    blue: Vec<u16>,
}

/// Why a gamma table cannot be uploaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GammaError {
    /// The CRTC's table has a different length than the one that was built.
    Length {
        /// The length of the table that was built.
        table: usize,
        /// The length the CRTC applies.
        hardware: usize,
    },
    /// The CRTC has no gamma table, so there is nothing to upload to.
    Degenerate,
}

impl std::fmt::Display for GammaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Length { table, hardware } => write!(
                formatter,
                "a gamma table of {table} entries does not fit a CRTC with {hardware}"
            ),
            Self::Degenerate => write!(formatter, "the CRTC has no gamma table"),
        }
    }
}

impl std::error::Error for GammaError {}

impl GammaRamp {
    /// The linear ramp: every channel passes its value through unchanged.
    ///
    /// It is what a display with no night light and no calibration curve is given, and it is
    /// exact: entry `i` of a `length`-entry table is `i * 65535 / (length - 1)`.
    #[must_use]
    pub fn identity(length: usize) -> Self {
        if length == 0 {
            return Self {
                length,
                red: Vec::new(),
                green: Vec::new(),
                blue: Vec::new(),
            };
        }
        let channel = linear(length);
        let multiplied = |factor: f64| scale(&channel, factor);
        Self {
            length,
            red: multiplied(1.0),
            green: multiplied(1.0),
            blue: multiplied(1.0),
        }
    }

    /// The ramp for a colour temperature, normalised so the peak stays at full scale.
    ///
    /// A temperature at or above [`NEUTRAL_KELVIN`] is the identity, because the panel is
    /// already close to that white point and tinting it by a rounding error would be worse
    /// than doing nothing. Below it the red channel is left at full scale and green and blue
    /// are scaled down, which is the direction a warmer white point goes.
    #[must_use]
    pub fn for_temperature(kelvin: u32, length: usize) -> Self {
        if kelvin >= NEUTRAL_KELVIN {
            return Self::identity(length);
        }
        let kelvin = kelvin.max(MIN_KELVIN);
        let (red, green, blue) = white_point(kelvin);
        let channel = linear(length);
        Self {
            length,
            red: scale(&channel, red),
            green: scale(&channel, green),
            blue: scale(&channel, blue),
        }
    }

    /// How many entries the table has.
    #[must_use]
    pub fn length(&self) -> usize {
        self.length
    }

    /// The three channels, in the order the kernel takes them.
    #[must_use]
    pub fn channels(&self) -> (&[u16], &[u16], &[u16]) {
        (&self.red, &self.green, &self.blue)
    }

    /// Whether this table can be uploaded to a CRTC whose table has `hardware` entries.
    ///
    /// # Errors
    ///
    /// [`GammaError::Length`] when the lengths differ, and [`GammaError::Degenerate`] when
    /// either is zero.
    pub fn matches(&self, hardware: usize) -> Result<(), GammaError> {
        if hardware == 0 || self.length == 0 {
            return Err(GammaError::Degenerate);
        }
        if hardware != self.length {
            return Err(GammaError::Length {
                table: self.length,
                hardware,
            });
        }
        Ok(())
    }
}

/// The length of a CRTC's gamma table, as the CRTC reports it.
#[must_use]
pub fn crtc_length(info: &crtc::Info) -> usize {
    usize::try_from(info.gamma_length()).unwrap_or(0)
}

/// The linear ramp of `length` entries, from 0 to full scale inclusive.
fn linear(length: usize) -> Vec<u16> {
    if length == 0 {
        return Vec::new();
    }
    if length == 1 {
        return vec![u16::MAX];
    }
    let last = (length - 1) as f64;
    (0..length)
        .map(|index| ((index as f64 / last) * f64::from(u16::MAX)).round() as u16)
        .collect()
}

/// A linear ramp scaled by `factor`, clamped to the channel's range.
fn scale(channel: &[u16], factor: f64) -> Vec<u16> {
    if !factor.is_finite() || factor <= 0.0 {
        return vec![0; channel.len()];
    }
    channel
        .iter()
        .map(|value| {
            let scaled = f64::from(*value) * factor;
            if scaled <= 0.0 {
                0
            } else if scaled >= f64::from(u16::MAX) {
                u16::MAX
            } else {
                scaled.round() as u16
            }
        })
        .collect()
}

/// The relative brightness of each channel for a blackbody at `kelvin`.
///
/// The approximation is Tanner Helland's fit of the Planckian locus, which is what the
/// night-light tools in this ecosystem use; it is only defined above 1000 K, and the caller
/// clamps to that. The result is normalised so the largest channel is 1.0, because a gamma
/// table with a channel above 1.0 would clip.
fn white_point(kelvin: u32) -> (f64, f64, f64) {
    let temperature = f64::from(kelvin) / 100.0;
    let red = if temperature <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (temperature - 60.0).powf(-0.133_204_759_2)
    };
    let green = if temperature <= 66.0 {
        99.470_802_586_1 * temperature.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (temperature - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if temperature >= 66.0 {
        255.0
    } else if temperature <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (temperature - 10.0).ln() - 305.044_792_730_7
    };
    let peak = red.max(green).max(blue).max(1.0);
    (
        (red / peak).clamp(0.0, 1.0),
        (green / peak).clamp(0.0, 1.0),
        (blue / peak).clamp(0.0, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_table_is_exactly_linear() {
        let ramp = GammaRamp::identity(256);
        let (red, green, blue) = ramp.channels();
        assert_eq!(red, green);
        assert_eq!(red, blue);
        assert_eq!(red[0], 0);
        assert_eq!(red[255], u16::MAX);
        assert_eq!(red[128], 32896);
        assert!(red.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn every_channel_has_the_length_it_was_asked_for() {
        for length in [1, 2, 17, 256, 1024] {
            let ramp = GammaRamp::for_temperature(3000, length);
            let (red, green, blue) = ramp.channels();
            assert_eq!(ramp.length(), length, "length {length}");
            assert_eq!(red.len(), length);
            assert_eq!(green.len(), length);
            assert_eq!(blue.len(), length);
        }
    }

    #[test]
    fn a_table_of_the_wrong_length_is_refused() {
        let ramp = GammaRamp::identity(256);
        assert_eq!(ramp.matches(256), Ok(()));
        assert_eq!(
            ramp.matches(1024),
            Err(GammaError::Length {
                table: 256,
                hardware: 1024
            })
        );
    }

    #[test]
    fn an_empty_table_is_refused_rather_than_ignored() {
        assert_eq!(GammaRamp::identity(0).matches(256), Err(GammaError::Degenerate));
        assert_eq!(GammaRamp::identity(256).matches(0), Err(GammaError::Degenerate));
    }

    #[test]
    fn a_temperature_at_the_panel_white_point_is_the_identity() {
        assert_eq!(
            GammaRamp::for_temperature(NEUTRAL_KELVIN, 256),
            GammaRamp::identity(256)
        );
        assert_eq!(GammaRamp::for_temperature(9000, 256), GammaRamp::identity(256));
    }

    #[test]
    fn a_warm_temperature_dims_blue_and_keeps_the_peak() {
        let ramp = GammaRamp::for_temperature(2700, 256);
        let (red, green, blue) = ramp.channels();
        assert_eq!(red[255], u16::MAX, "the red channel stays at full scale");
        assert!(blue[255] < green[255], "blue is dimmed more than green");
        assert!(green[255] < red[255]);
    }

    #[test]
    fn a_cooler_temperature_leaves_more_blue() {
        let warm = GammaRamp::for_temperature(2700, 256);
        let cool = GammaRamp::for_temperature(4500, 256);
        assert!(cool.channels().2[255] > warm.channels().2[255]);
        // Both keep red at full scale, so the difference is the white point and nothing
        // else.
        assert_eq!(warm.channels().0[255], cool.channels().0[255]);
    }

    #[test]
    fn a_temperature_below_the_curve_is_clamped_not_undefined() {
        let ramp = GammaRamp::for_temperature(1, 64);
        assert_eq!(ramp.length(), 64);
        assert_eq!(ramp.channels().0[63], u16::MAX);
        assert!(ramp.channels().2[63] < ramp.channels().1[63]);
    }
}
