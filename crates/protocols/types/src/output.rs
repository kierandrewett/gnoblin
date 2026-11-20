//! What an output manager reports for each output, and what a configuration asks for one.

use gnoblin_types::OutputId;

/// One mode a head can be driven in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Refresh rate in millihertz, as the protocol reports it.
    pub refresh_mhz: i32,
    /// Whether the protocol marks this mode as preferred. The backend reports its one mode as preferred.
    pub preferred: bool,
}

/// What the host reports for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadInfo {
    /// The stable identity the host uses for this output.
    pub id: OutputId,
    /// The connector name the backend knows this output by.
    pub name: String,
    /// The description the protocol reports. The backend sets it to the connector name.
    pub description: String,
    /// The manufacturer string. The backend reports `Gnoblin`.
    pub make: String,
    /// The model string. The backend reports the connector name.
    pub model: String,
    /// The serial number. The backend reports none, so this is empty.
    pub serial: String,
    /// Physical size in millimetres.
    pub physical_mm: (i32, i32),
    /// Every mode the head can be driven in. [`HeadInfo::current`] indexes into this list.
    pub modes: Vec<ModeInfo>,
    /// Index into `modes` of the mode in use, if any.
    pub current: Option<usize>,
    /// Whether the output is on. The backend reports every output as enabled.
    pub enabled: bool,
    /// Top-left corner in the layout. `(0, 0)` when the layout has no position for the output.
    pub position: (i32, i32),
    /// `wl_output` transform value.
    pub transform: i32,
    /// Scale factor reported to clients, read from the backend's current scale.
    pub scale: f64,
    /// Whether adaptive sync is on. The backend always reports `false`.
    pub adaptive_sync: bool,
}

/// A mode a client asked for, either one the head lists or a custom one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeRequest {
    /// Requested width in pixels.
    pub width: i32,
    /// Requested height in pixels.
    pub height: i32,
    /// Requested refresh rate in millihertz.
    pub refresh_mhz: i32,
}

/// What a configuration asks for one head. A `None` field keeps the head's current value.
///
/// A request is read against the head as it is now and laid over the output's profile, so a request that names what
/// the head already has changes nothing. A transform other than `0` and adaptive sync are refused.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadRequest {
    /// The head this request configures.
    pub head: OutputId,
    /// Whether the head should be on. A configuration must leave at least one head on.
    pub enabled: bool,
    /// The mode to drive the head in, or `None` to keep the current one. A mode the backend does not list is refused.
    pub mode: Option<ModeRequest>,
    /// The position to move the head to, or `None` to keep it.
    pub position: Option<(i32, i32)>,
    /// The `wl_output` transform to apply, or `None` to keep it. Any value but `0` is refused.
    pub transform: Option<i32>,
    /// The scale to apply, or `None` to keep it.
    pub scale: Option<f64>,
    /// Whether adaptive sync should be on, or `None` to keep it. `true` is refused.
    pub adaptive_sync: Option<bool>,
}
