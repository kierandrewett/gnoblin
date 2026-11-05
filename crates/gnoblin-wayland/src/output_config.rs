//! The host seam of `zwlr_output_management_v1`: what the outputs look like, and how the
//! host applies or tests a layout a client asked for.
//!
//! The protocol handler runs inside the Wayland dispatch, which holds the
//! [`State`](crate::State) but not the backend. The host (the composition root) gives the
//! compositor a [`HeadInfo`] snapshot whenever the outputs change, and answers each queued
//! configuration through [`OutputConfigHost::apply`]. The host owns rollback: a `false`
//! answer means it changed nothing, or restored what it changed.

use gnoblin_types::OutputId;

/// One mode a head can be driven in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    /// Refresh rate in millihertz, as the protocol reports it.
    pub refresh_mhz: i32,
    pub preferred: bool,
}

/// What the host reports for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadInfo {
    pub id: OutputId,
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    /// Physical size in millimetres.
    pub physical_mm: (i32, i32),
    pub modes: Vec<ModeInfo>,
    /// Index into `modes` of the mode in use, if any.
    pub current: Option<usize>,
    pub enabled: bool,
    pub position: (i32, i32),
    /// `wl_output` transform value.
    pub transform: i32,
    pub scale: f64,
    pub adaptive_sync: bool,
}

/// A mode a client asked for, either one the head lists or a custom one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeRequest {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
}

/// What a configuration asks for one head. A `None` field keeps the head's current value.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadRequest {
    pub head: OutputId,
    pub enabled: bool,
    pub mode: Option<ModeRequest>,
    pub position: Option<(i32, i32)>,
    pub transform: Option<i32>,
    pub scale: Option<f64>,
    pub adaptive_sync: Option<bool>,
}

/// The host side of output configuration.
pub trait OutputConfigHost {
    /// The outputs as they are now.
    fn heads(&self) -> Vec<HeadInfo>;

    /// Applies `requests`, or only checks them when `test_only` is set.
    ///
    /// `Ok` means the whole layout was accepted (and, when not a test, is now in effect).
    /// `Err` means nothing was left changed: the host rolls back any part it had applied.
    fn apply(&mut self, requests: &[HeadRequest], test_only: bool) -> Result<(), String>;
}

/// A host with no outputs to configure: every layout is refused.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOutputConfig;

impl OutputConfigHost for NoOutputConfig {
    fn heads(&self) -> Vec<HeadInfo> {
        Vec::new()
    }

    fn apply(&mut self, _requests: &[HeadRequest], _test_only: bool) -> Result<(), String> {
        Err("the host configures no outputs".to_owned())
    }
}
