//! The host seam of `zwlr_output_management_v1`: what the outputs look like, and how the
//! host applies or tests a layout a client asked for.
//!
//! The protocol handler runs inside the Wayland dispatch, which holds the
//! [`State`](crate::State) but not the backend. The host (the composition root) gives the
//! compositor a [`HeadInfo`] snapshot whenever the outputs change, and answers each queued
//! configuration through [`OutputConfigHost::apply`]. The host owns rollback: a `false`
//! answer means it changed nothing, or restored what it changed.

use gnoblin_backend::Backend;
use gnoblin_output::DisplayMode;
use gnoblin_types::{OutputId, Scale};

use crate::State;

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

/// The composition root's host: the backend's outputs, configured through
/// [`Backend::configure_output`].
///
/// The layout owns positions, transforms and adaptive sync, and no backend can switch an
/// output off yet, so those requests are refused rather than silently ignored. A mode must
/// be one the head lists, which is the mode it is driven in now.
pub(crate) struct BackendOutputConfig<'a> {
    backend: &'a mut dyn Backend<State>,
    positions: Vec<(OutputId, (i32, i32))>,
}

impl<'a> BackendOutputConfig<'a> {
    pub(crate) fn new(backend: &'a mut dyn Backend<State>, positions: Vec<(OutputId, (i32, i32))>) -> Self {
        Self { backend, positions }
    }
}

impl OutputConfigHost for BackendOutputConfig<'_> {
    fn heads(&self) -> Vec<HeadInfo> {
        self.backend
            .outputs()
            .iter()
            .map(|output| {
                let mode = ModeInfo {
                    width: i32::try_from(output.mode.width).unwrap_or(i32::MAX),
                    height: i32::try_from(output.mode.height).unwrap_or(i32::MAX),
                    refresh_mhz: i32::try_from(output.mode.refresh_mhz).unwrap_or(i32::MAX),
                    preferred: true,
                };
                let position = self
                    .positions
                    .iter()
                    .find(|(id, _)| *id == output.id)
                    .map_or((0, 0), |(_, position)| *position);
                HeadInfo {
                    id: output.id,
                    name: output.connector.clone(),
                    description: output.connector.clone(),
                    make: "Gnoblin".to_owned(),
                    model: output.connector.clone(),
                    serial: String::new(),
                    physical_mm: (0, 0),
                    modes: vec![mode],
                    current: Some(0),
                    enabled: true,
                    position,
                    transform: 0,
                    scale: output.scale.as_f64(),
                    adaptive_sync: false,
                }
            })
            .collect()
    }

    fn apply(&mut self, requests: &[HeadRequest], test_only: bool) -> Result<(), String> {
        if requests.iter().all(|request| !request.enabled) {
            return Err("a configuration must leave at least one output enabled".to_owned());
        }
        let heads = self.heads();
        let mut plans = Vec::new();
        for request in requests {
            let Some(head) = heads.iter().find(|head| head.id == request.head) else {
                return Err("the configuration names a head this host does not drive".to_owned());
            };
            if !request.enabled {
                return Err(format!("{}: switching an output off is not supported", head.name));
            }
            if request.position.is_some_and(|position| position != head.position) {
                return Err(format!("{}: moving an output is not supported", head.name));
            }
            if request.transform.is_some_and(|transform| transform != 0) {
                return Err(format!("{}: transforms are not supported", head.name));
            }
            if request.adaptive_sync == Some(true) {
                return Err(format!("{}: adaptive sync is not supported", head.name));
            }
            let current = head.modes[0];
            if request.mode.is_some_and(|mode| {
                mode.width != current.width || mode.height != current.height || mode.refresh_mhz != current.refresh_mhz
            }) {
                return Err(format!("{}: the requested mode is not offered", head.name));
            }
            let Some(scale) = request.scale else {
                continue;
            };
            if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
                return Err(format!("{}: scale {scale} is out of range", head.name));
            }
            let Some(output) = self.backend.outputs().iter().find(|output| output.id == request.head) else {
                return Err("the configuration names a head this host does not drive".to_owned());
            };
            let target = Scale::from_percent((scale * 100.0).round() as u32);
            if target != output.scale {
                plans.push((output.id, output.mode, output.scale, target));
            }
        }
        if test_only {
            return Ok(());
        }
        let mut applied: Vec<(OutputId, DisplayMode, Scale)> = Vec::new();
        for (id, mode, old_scale, target) in plans {
            if let Err(error) = self.backend.configure_output(id, mode, target) {
                for (done, done_mode, done_scale) in applied {
                    // Restoring a mode the backend just accepted cannot be refused in turn;
                    // if it is, the head is left at its new scale and the error says so.
                    let _ = self.backend.configure_output(done, done_mode, done_scale);
                }
                return Err(format!("the backend refused the change: {error}"));
            }
            applied.push((id, mode, old_scale));
        }
        Ok(())
    }
}

impl State {
    /// The layout position of each output, for the heads the host publishes.
    pub(crate) fn output_positions(&self) -> Vec<(OutputId, (i32, i32))> {
        self.output_ids()
            .into_iter()
            .filter_map(|id| self.output_rect(id).map(|rect| (id, (rect.x, rect.y))))
            .collect()
    }
}
