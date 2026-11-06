//! The value rules of the `appearance` table: what an accent is, and how it reads
//! as a name for toolkits that only know the nine named accents.
//!
//! Failure modes, each a named test below:
//!
//! 1. A hex accent must be `#rgb` or `#rrggbb`, case-insensitive: `bad_hex_is_refused`.
//! 2. A hex accent with an alpha channel is refused, not silently dropped:
//!    `hex_with_alpha_is_refused`.
//! 3. A named accent is one of the nine libadwaita names, spelled exactly:
//!    `unknown_names_are_refused`.
//! 4. A hex accent halfway between two names maps to the same name every time, the
//!    earlier one in [`NAMED_ACCENTS`]: `a_tie_goes_to_the_earlier_name`.
//! 5. A named accent keeps its own name, even though its colour is also a hex:
//!    `a_named_accent_keeps_its_name`.

/// One colour in 8-bit sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccentRgb {
    /// The red channel.
    pub red: u8,
    /// The green channel.
    pub green: u8,
    /// The blue channel.
    pub blue: u8,
}

impl AccentRgb {
    /// A colour from its three channels.
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// The channels as the portal's `(ddd)` expects them: each divided by 255, in [0, 1].
    #[must_use]
    pub fn unit(self) -> (f64, f64, f64) {
        (
            f64::from(self.red) / 255.0,
            f64::from(self.green) / 255.0,
            f64::from(self.blue) / 255.0,
        )
    }

    /// The squared distance to another colour in sRGB. Squared, because only the order
    /// of distances matters and the root would add rounding for no gain.
    fn distance_squared(self, other: Self) -> u32 {
        let channel = |a: u8, b: u8| {
            let difference = u32::from(a.abs_diff(b));
            difference * difference
        };
        channel(self.red, other.red) + channel(self.green, other.green) + channel(self.blue, other.blue)
    }
}

/// The nine named accents, in the order libadwaita lists them, with the colour
/// libadwaita draws each one in (`adw_accent_color_to_rgba`, libadwaita `main`).
///
/// The order is part of the contract: a hex colour exactly between two names resolves
/// to the earlier one.
pub const NAMED_ACCENTS: &[(&str, AccentRgb)] = &[
    ("blue", AccentRgb::new(0x35, 0x84, 0xe4)),
    ("teal", AccentRgb::new(0x21, 0x90, 0xa4)),
    ("green", AccentRgb::new(0x3a, 0x94, 0x4a)),
    ("yellow", AccentRgb::new(0xc8, 0x88, 0x00)),
    ("orange", AccentRgb::new(0xed, 0x5b, 0x00)),
    ("red", AccentRgb::new(0xe6, 0x2d, 0x42)),
    ("pink", AccentRgb::new(0xd5, 0x61, 0x99)),
    ("purple", AccentRgb::new(0x91, 0x41, 0xac)),
    ("slate", AccentRgb::new(0x6f, 0x83, 0x96)),
];

/// The accent the document names: a named accent or an exact colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    /// One of [`NAMED_ACCENTS`], by its name.
    Named(&'static str),
    /// An exact colour, from `#rgb` or `#rrggbb`.
    Hex(AccentRgb),
}

impl Accent {
    /// Parse the configured accent: a name from [`NAMED_ACCENTS`], or `#rgb` / `#rrggbb`.
    ///
    /// # Errors
    ///
    /// A message naming what was accepted, when the text is none of those.
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex).map(Self::Hex).ok_or_else(|| {
                if hex.len() == 8 || hex.len() == 4 {
                    format!("{text:?} has an alpha channel; an accent is an opaque colour")
                } else {
                    format!("{text:?} is not #rgb or #rrggbb")
                }
            });
        }
        NAMED_ACCENTS
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(name, _)| Self::Named(*name))
            .ok_or_else(|| format!("{text:?} is not one of {}, or #rrggbb", named_list()))
    }

    /// The colour this accent is drawn in.
    #[must_use]
    pub fn rgb(self) -> AccentRgb {
        match self {
            Self::Named(name) => NAMED_ACCENTS
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map_or(NAMED_ACCENTS[0].1, |(_, rgb)| *rgb),
            Self::Hex(rgb) => rgb,
        }
    }

    /// The name a toolkit that only knows the named accents should use: the accent's own
    /// name, or for a hex colour the nearest named accent.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Named(name) => name,
            Self::Hex(rgb) => nearest_named(rgb),
        }
    }
}

/// The named accent nearest to a colour, by squared sRGB distance.
///
/// A tie goes to the earlier entry of [`NAMED_ACCENTS`], which is what makes the answer
/// the same on every run: the comparison is strict, so a later name never displaces an
/// earlier one of the same distance.
#[must_use]
pub fn nearest_named(rgb: AccentRgb) -> &'static str {
    let mut best = NAMED_ACCENTS[0];
    for entry in &NAMED_ACCENTS[1..] {
        if entry.1.distance_squared(rgb) < best.1.distance_squared(rgb) {
            best = *entry;
        }
    }
    best.0
}

/// `rgb` or `rrggbb`, each digit hexadecimal; `None` for any other length or character.
fn parse_hex(hex: &str) -> Option<AccentRgb> {
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |index: usize| u8::from_str_radix(&hex[index..=index], 16).ok();
    let pair = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    match hex.len() {
        3 => Some(AccentRgb::new(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
        6 => Some(AccentRgb::new(pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}

/// The nine names, comma-separated, for a message.
fn named_list() -> String {
    NAMED_ACCENTS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_hex_is_refused() {
        for text in [
            "#", "#12", "#12345", "#1234567", "#gg0000", "#00 000", "336699", "#-3366",
        ] {
            assert!(Accent::parse(text).is_err(), "{text} must be refused");
        }
    }

    #[test]
    fn hex_with_alpha_is_refused() {
        for text in ["#1234", "#11223344"] {
            let error = Accent::parse(text).expect_err("alpha is not an accent");
            assert!(error.contains("alpha"), "{error}");
        }
    }

    #[test]
    fn short_and_long_hex_are_the_same_colour() {
        assert_eq!(Accent::parse("#FA0").unwrap().rgb(), AccentRgb::new(0xff, 0xaa, 0x00));
        assert_eq!(
            Accent::parse("#ffaa00").unwrap().rgb(),
            AccentRgb::new(0xff, 0xaa, 0x00)
        );
        assert_eq!(
            Accent::parse("#FFAA00").unwrap().rgb(),
            AccentRgb::new(0xff, 0xaa, 0x00)
        );
    }

    #[test]
    fn unknown_names_are_refused() {
        for text in ["Blue", "navy", "", " blue"] {
            assert!(Accent::parse(text).is_err(), "{text:?} must be refused");
        }
    }

    #[test]
    fn a_named_accent_keeps_its_name() {
        let accent = Accent::parse("teal").expect("teal is named");
        assert_eq!(accent.name(), "teal");
        assert_eq!(accent.rgb(), AccentRgb::new(0x21, 0x90, 0xa4));
    }

    #[test]
    fn a_hex_equal_to_a_name_maps_to_that_name() {
        for (name, rgb) in NAMED_ACCENTS {
            let hex = format!("#{:02x}{:02x}{:02x}", rgb.red, rgb.green, rgb.blue);
            assert_eq!(Accent::parse(&hex).unwrap().name(), *name);
        }
    }

    #[test]
    fn a_tie_goes_to_the_earlier_name() {
        // Halfway between blue (#3584e4) and teal (#2190a4) on every channel, so both
        // are at the same squared distance; blue comes first in the table.
        let halfway = Accent::parse("#2b8ac4").expect("valid hex");
        assert_eq!(halfway.rgb(), AccentRgb::new(43, 138, 196));
        assert_eq!(halfway.name(), "blue");
        assert_eq!(nearest_named(AccentRgb::new(43, 138, 196)), "blue");
    }

    #[test]
    fn the_unit_form_is_the_channel_over_255() {
        let (red, green, blue) = AccentRgb::new(255, 0, 51).unit();
        assert!((red - 1.0).abs() < f64::EPSILON);
        assert!(green.abs() < f64::EPSILON);
        assert!((blue - 0.2).abs() < 1e-12);
    }
}
