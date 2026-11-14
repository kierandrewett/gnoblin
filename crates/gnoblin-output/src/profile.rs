//! Persistent output profiles: the hardware policy that survives a hotplug.
//!
//! A profile is what a person or a script decided for one monitor (mode, scale, position, enabled, mirror).
//! It is keyed by the connector and the monitor's EDID identity, so the same panel on the same port gets its
//! arrangement back after an unplug, and a different panel that happens to take the port does not inherit it.
//! A profile whose hardware is absent is parked: it stays in the store, changes no layout, and applies the moment
//! the monitor is connected again.

use gnoblin_types::{Orientation, Scale};

use crate::layout::{LayoutConfig, OutputConfig, Placement};
use crate::mode::DisplayMode;

/// The scale range a profile accepts, matching what the output management protocol accepts.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.5..=4.0;

/// Why a change request could not be read.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ProfileError {
    /// The request has a key this build does not know.
    #[error("unknown output setting `{0}`")]
    UnknownKey(String),
    /// A line is not `key=value`.
    #[error("`{0}` is not `key=value`")]
    Malformed(String),
    /// A value is not valid for its key.
    #[error("{key}: {reason}")]
    BadValue {
        /// The setting.
        key: String,
        /// What is wrong with the value.
        reason: String,
    },
}

/// What one monitor is configured to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputProfile {
    /// The connector the profile was made for, such as `"DP-1"`.
    pub connector: String,
    /// The monitor's EDID identity, or `None` for an output that reports none (a virtual one).
    pub identity: Option<String>,
    /// Whether the output is on.
    pub enabled: bool,
    /// The mode to drive, or `None` for the preferred one.
    pub mode: Option<DisplayMode>,
    /// The scale, or `None` for the default.
    pub scale: Option<Scale>,
    /// The rotation or reflection.
    pub orientation: Orientation,
    /// The logical position, or `None` to follow the row.
    pub position: Option<(i32, i32)>,
    /// The connector this output shows a copy of.
    pub mirror_of: Option<String>,
}

impl OutputProfile {
    /// A profile that changes nothing for `connector`.
    #[must_use]
    pub fn new(connector: impl Into<String>, identity: Option<String>) -> Self {
        Self {
            connector: connector.into(),
            identity,
            enabled: true,
            mode: None,
            scale: None,
            orientation: Orientation::Normal,
            position: None,
            mirror_of: None,
        }
    }

    /// This profile with `change` applied over it.
    #[must_use]
    pub fn with(mut self, change: &ProfileChange) -> Self {
        if let Some(mode) = change.mode {
            self.mode = mode;
        }
        if let Some(scale) = change.scale {
            self.scale = scale;
        }
        if let Some(orientation) = change.orientation {
            self.orientation = orientation;
        }
        if let Some(position) = change.position {
            self.position = position;
        }
        if let Some(enabled) = change.enabled {
            self.enabled = enabled;
        }
        if let Some(mirror) = &change.mirror_of {
            self.mirror_of.clone_from(mirror);
        }
        self
    }
}

/// A request to change some settings of one profile. A field left `None` keeps the profile's value; an inner `None`
/// sets the value back to its default (the preferred mode, the default scale, the row position, no mirror).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProfileChange {
    /// The mode.
    pub mode: Option<Option<DisplayMode>>,
    /// The scale.
    pub scale: Option<Option<Scale>>,
    /// The rotation or reflection.
    pub orientation: Option<Orientation>,
    /// The logical position.
    pub position: Option<Option<(i32, i32)>>,
    /// Whether the output is on.
    pub enabled: Option<bool>,
    /// The connector to mirror.
    pub mirror_of: Option<Option<String>>,
}

fn bad(key: &str, reason: impl Into<String>) -> ProfileError {
    ProfileError::BadValue {
        key: key.to_owned(),
        reason: reason.into(),
    }
}

fn parse_mode(text: &str) -> Result<Option<DisplayMode>, ProfileError> {
    if text == "preferred" {
        return Ok(None);
    }
    let shaped = || bad("mode", format!("`{text}` is not `preferred` or `WIDTHxHEIGHT@HERTZ`"));
    let (size, rate) = text.split_once('@').ok_or_else(shaped)?;
    let (width, height) = size.split_once('x').ok_or_else(shaped)?;
    let width: u32 = width.parse().map_err(|_| shaped())?;
    let height: u32 = height.parse().map_err(|_| shaped())?;
    let hertz: f64 = rate.parse().map_err(|_| shaped())?;
    if width == 0 || height == 0 || !hertz.is_finite() || hertz <= 0.0 || hertz > 1000.0 {
        return Err(shaped());
    }
    Ok(Some(DisplayMode::new(width, height, (hertz * 1000.0).round() as u32)))
}

fn parse_scale(text: &str) -> Result<Option<Scale>, ProfileError> {
    if text == "default" {
        return Ok(None);
    }
    let value: f64 = text
        .parse()
        .map_err(|_| bad("scale", format!("`{text}` is not a number or `default`")))?;
    if !value.is_finite() || !SCALE_RANGE.contains(&value) {
        return Err(bad("scale", format!("{text} is outside 0.5 to 4")));
    }
    Ok(Some(Scale::from_percent((value * 100.0).round() as u32)))
}

fn parse_orientation(text: &str) -> Result<Orientation, ProfileError> {
    Ok(match text {
        "normal" => Orientation::Normal,
        "90" => Orientation::Rotate90,
        "180" => Orientation::Rotate180,
        "270" => Orientation::Rotate270,
        "flipped" => Orientation::Flipped,
        "flipped-90" => Orientation::Flipped90,
        "flipped-180" => Orientation::Flipped180,
        "flipped-270" => Orientation::Flipped270,
        other => return Err(bad("transform", format!("`{other}` is not a transform name"))),
    })
}

fn parse_position(text: &str) -> Result<Option<(i32, i32)>, ProfileError> {
    if text == "auto" {
        return Ok(None);
    }
    let shaped = || bad("position", format!("`{text}` is not `auto` or `X,Y`"));
    let (x, y) = text.split_once(',').ok_or_else(shaped)?;
    Ok(Some((
        x.trim().parse().map_err(|_| shaped())?,
        y.trim().parse().map_err(|_| shaped())?,
    )))
}

impl ProfileChange {
    /// Reads a request: one `key=value` per line. Keys are `mode` (`WxH@Hz` or `preferred`), `scale` (a number or
    /// `default`), `transform`, `position` (`X,Y` or `auto`), `enabled` (`true` or `false`) and `mirror_of`
    /// (a connector, or empty for none).
    ///
    /// # Errors
    ///
    /// [`ProfileError`] names the first key or value that is wrong, so a caller can report its key path.
    pub fn parse(text: &str) -> Result<Self, ProfileError> {
        let mut change = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| ProfileError::Malformed(line.to_owned()))?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                "mode" => change.mode = Some(parse_mode(value)?),
                "scale" => change.scale = Some(parse_scale(value)?),
                "transform" => change.orientation = Some(parse_orientation(value)?),
                "position" => change.position = Some(parse_position(value)?),
                "enabled" => {
                    change.enabled = Some(match value {
                        "true" => true,
                        "false" => false,
                        other => return Err(bad("enabled", format!("`{other}` is not `true` or `false`"))),
                    });
                }
                "mirror_of" => change.mirror_of = Some((!value.is_empty()).then(|| value.to_owned())),
                other => return Err(ProfileError::UnknownKey(other.to_owned())),
            }
        }
        Ok(change)
    }
}

/// One connected output as the store sees it: the connector and the EDID identity, if the monitor has one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutputSighting<'a> {
    /// The connector.
    pub connector: &'a str,
    /// The EDID identity.
    pub identity: Option<&'a str>,
}

/// Every profile the session remembers.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProfileStore {
    profiles: Vec<OutputProfile>,
}

/// Whether `profile` was made for the output `seen`: the same connector, and the same monitor when both sides know
/// which monitor it is. A virtual output has no identity and matches on the connector alone.
fn matches(profile: &OutputProfile, seen: OutputSighting<'_>) -> bool {
    profile.connector == seen.connector
        && match (profile.identity.as_deref(), seen.identity) {
            (Some(known), Some(now)) => known == now,
            _ => true,
        }
}

impl ProfileStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The profile for an output, if one was kept.
    #[must_use]
    pub fn find(&self, seen: OutputSighting<'_>) -> Option<&OutputProfile> {
        self.profiles.iter().find(|profile| matches(profile, seen))
    }

    /// Keeps `profile`, replacing the one it matches.
    pub fn keep(&mut self, profile: OutputProfile) {
        let seen = OutputSighting {
            connector: &profile.connector,
            identity: profile.identity.as_deref(),
        };
        match self.profiles.iter().position(|kept| matches(kept, seen)) {
            Some(index) => self.profiles[index] = profile,
            None => self.profiles.push(profile),
        }
    }

    /// Every profile, in the order they were first kept.
    #[must_use]
    pub fn all(&self) -> &[OutputProfile] {
        &self.profiles
    }

    /// The profiles with no hardware connected: they change no layout and wait for their monitor.
    #[must_use]
    pub fn parked<'a>(&'a self, connected: &[OutputSighting<'_>]) -> Vec<&'a OutputProfile> {
        self.profiles
            .iter()
            .filter(|profile| !connected.iter().any(|seen| matches(profile, *seen)))
            .collect()
    }

    /// The layout configuration for `connected`, in connection order: a row of outputs with every matching profile
    /// laid over it.
    ///
    /// A mirror whose source is not connected and on, or is itself a mirror, shows its own picture instead: a profile
    /// must never leave an output that fails the layout, so what it cannot do is parked, not refused.
    #[must_use]
    pub fn layout(&self, connected: &[OutputSighting<'_>]) -> LayoutConfig {
        let order: Vec<String> = connected.iter().map(|seen| seen.connector.to_owned()).collect();
        let mut config = LayoutConfig::row_over(&order);
        let profiles: Vec<Option<&OutputProfile>> = connected.iter().map(|seen| self.find(*seen)).collect();
        let source_ok = |name: &str| {
            connected.iter().zip(&profiles).any(|(seen, profile)| {
                seen.connector == name && profile.is_none_or(|profile| profile.enabled && profile.mirror_of.is_none())
            })
        };
        for (entry, profile) in config.outputs.iter_mut().zip(&profiles) {
            let Some(profile) = profile else {
                continue;
            };
            let mirror = profile
                .mirror_of
                .as_deref()
                .filter(|source| *source != entry.connector && source_ok(source));
            entry.enabled = profile.enabled;
            if let Some(source) = mirror {
                // A mirror shows the source's mode and scale on its own panel, so it declares neither.
                entry.mirror_of = Some(source.to_owned());
                entry.placement = None;
                continue;
            }
            entry.mode = profile.mode;
            entry.scale = profile.scale;
            entry.orientation = profile.orientation;
            if let Some((x, y)) = profile.position {
                entry.placement = Some(Placement::Absolute { x, y });
            }
        }
        // The first output that has a monitor of its own is the primary; a mirror or a disabled output cannot be.
        config.primary = config
            .outputs
            .iter()
            .find(|entry| entry.enabled && entry.mirror_of.is_none())
            .map(|entry: &OutputConfig| entry.connector.clone());
        config
    }
}
