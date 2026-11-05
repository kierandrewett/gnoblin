//! What is true about a connected output, independent of the layout.

use gnoblin_types::{OutputId, Scale};

use crate::edid::Edid;
use crate::mode::Mode;
use crate::physical::PhysicalSize;

/// Whether the connector can vary its refresh rate with the content.
///
/// The capability belongs to the connector and the requested state to the
/// configuration, because asking for VRR on a panel that has none is not an
/// error: the layout records that the request was downgraded instead of refusing
/// to light the monitor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VrrSupport {
    /// The connector has no variable refresh rate.
    #[default]
    Unsupported,
    /// The connector can follow the content's frame rate.
    Supported,
}

/// One connected output, described without any backend type.
///
/// Two identities meet here on purpose. [`OutputDescriptor::id`] is the graph
/// token the compositor hands to window management, accessibility and the locker,
/// because those tables want a cheap compare-and-hash key. The connector name is
/// the identity a person and the public output record use: it is what
/// `docs/shell-api/outputs.md` calls the output's `id` and what a saved
/// configuration has to survive a reboot with, since a token is allocated per
/// session. Output discovery owns the table between the two, which is why it is
/// this crate's job to keep them together.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputDescriptor {
    /// The graph token, as the backend numbers the connector.
    pub id: OutputId,
    /// The connector name, such as `"DP-1"`, which is the public output `id`.
    pub connector: String,
    /// A human-readable name, when the hardware provides one.
    pub name: Option<String>,
    /// The manufacturer string, three letters when it came from EDID.
    pub make: Option<String>,
    /// The model string.
    pub model: Option<String>,
    /// The serial string, which distinguishes two identical panels.
    pub serial: Option<String>,
    /// The panel's physical extent, zero where unknown.
    pub physical: PhysicalSize,
    /// Every mode the connector accepts, in preference order.
    pub modes: Vec<Mode>,
    /// Every scale the session is willing to drive this output at.
    pub scales: Vec<Scale>,
    /// Whether the connector can vary its refresh rate.
    pub vrr: VrrSupport,
}

impl OutputDescriptor {
    /// A connector with a mode list and no other identity.
    ///
    /// The supported scales start at the identity scale, because a connector that
    /// has not been asked about scaling can always be driven unscaled, and an
    /// empty list would make every configuration invalid.
    #[must_use]
    pub fn new(id: OutputId, connector: impl Into<String>, modes: Vec<Mode>) -> Self {
        Self {
            id,
            connector: connector.into(),
            name: None,
            make: None,
            model: None,
            serial: None,
            physical: PhysicalSize::default(),
            modes,
            scales: vec![Scale::integral(1)],
            vrr: VrrSupport::Unsupported,
        }
    }

    /// A descriptor built from what the monitor reported about itself.
    ///
    /// EDID describes identity and one timing, and says nothing about which
    /// scales the session offers or whether the connector can vary its rate, so
    /// those keep their defaults until the backend fills them in.
    #[must_use]
    pub fn from_edid(id: OutputId, connector: impl Into<String>, edid: &Edid) -> Self {
        Self {
            id,
            connector: connector.into(),
            name: edid.name.clone(),
            make: edid.manufacturer.clone(),
            model: None,
            serial: edid.serial.map(|serial| serial.to_string()),
            physical: edid.physical,
            modes: edid.preferred_mode.into_iter().collect(),
            scales: vec![Scale::integral(1)],
            vrr: VrrSupport::Unsupported,
        }
    }

    /// The mode to use when the configuration does not name one.
    ///
    /// The backend lists modes in preference order, so the first entry is the
    /// monitor's own preferred timing and the one a fresh output should use.
    #[must_use]
    pub fn preferred_mode(&self) -> Option<Mode> {
        self.modes.first().copied()
    }

    /// Whether the connector accepts exactly this mode.
    ///
    /// The comparison is on the full mode, refresh included: a 60 Hz request must
    /// not be satisfied by a 59.94 Hz mode, which would present judder the caller
    /// did not ask for.
    #[must_use]
    pub fn supports_mode(&self, mode: Mode) -> bool {
        self.modes.contains(&mode)
    }

    /// Whether the session may drive this connector at exactly this scale.
    #[must_use]
    pub fn supports_scale(&self, scale: Scale) -> bool {
        self.scales.contains(&scale)
    }

    /// The supported scale closest to `1`, which is the least surprising default.
    ///
    /// Ties go to the smaller scale so two equal candidates cannot make the choice
    /// depend on list order. An empty list still yields the identity scale, since
    /// an output has to be driveable somehow.
    #[must_use]
    pub fn default_scale(&self) -> Scale {
        let mut best: Option<Scale> = None;
        for candidate in &self.scales {
            let closer = match best {
                None => true,
                Some(current) => match (
                    is_closer_to_identity(*candidate, current),
                    is_closer_to_identity(current, *candidate),
                ) {
                    (true, _) => true,
                    (false, true) => false,
                    // Equal distance: the smaller scale wins, so two candidates
                    // cannot make the choice depend on list order.
                    (false, false) => *candidate < current,
                },
            };
            if closer {
                best = Some(*candidate);
            }
        }
        best.unwrap_or_default()
    }
}

/// Whether `candidate` is nearer to `1` than `current` is.
///
/// The distance is `|n/d - 1| = |n - d| / d`, and comparing two of those
/// cross-multiplies back to integers, so no float decides the default scale and no
/// two scales can compare equal by accident.
fn is_closer_to_identity(candidate: Scale, current: Scale) -> bool {
    let candidate_numerator = u128::from(candidate.numerator().abs_diff(candidate.denominator()));
    let candidate_denominator = u128::from(candidate.denominator());
    let current_numerator = u128::from(current.numerator().abs_diff(current.denominator()));
    let current_denominator = u128::from(current.denominator());
    candidate_numerator * current_denominator < current_numerator * candidate_denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(scales: Vec<Scale>) -> OutputDescriptor {
        let mut output = OutputDescriptor::new(
            OutputId::new(1),
            "DP-1",
            vec![Mode::new(2560, 1440, 60_000), Mode::new(1920, 1080, 60_000)],
        );
        output.scales = scales;
        output
    }

    #[test]
    fn the_preferred_mode_is_the_first_the_backend_listed() {
        assert_eq!(
            descriptor(vec![Scale::integral(1)]).preferred_mode(),
            Some(Mode::new(2560, 1440, 60_000))
        );
    }

    #[test]
    fn a_mode_matches_only_at_the_same_refresh_rate() {
        let output = descriptor(vec![Scale::integral(1)]);
        assert!(output.supports_mode(Mode::new(1920, 1080, 60_000)));
        assert!(!output.supports_mode(Mode::new(1920, 1080, 59_940)));
    }

    #[test]
    fn the_default_scale_is_the_supported_one_closest_to_one() {
        assert_eq!(
            descriptor(vec![Scale::integral(2), Scale::from_percent(125)]).default_scale(),
            Scale::from_percent(125)
        );
        assert_eq!(
            descriptor(vec![Scale::from_percent(150), Scale::from_percent(75)]).default_scale(),
            Scale::from_percent(75)
        );
        assert_eq!(descriptor(Vec::new()).default_scale(), Scale::integral(1));
    }
}
