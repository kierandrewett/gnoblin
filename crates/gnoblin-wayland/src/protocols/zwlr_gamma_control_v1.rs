//! `zwlr_gamma_control_manager_v1` (version 1): a client loads colour ramps onto an output.
//!
//! Night-light and colour-calibration tools use the wlroots protocol. A client asks for
//! control of one output, is told the size of its gamma tables, and then writes three
//! tables of that many `u16` values (red, green, blue) through a file descriptor. The
//! hardware-side work is done by the host through [`OutputControl`](crate::output_control::OutputControl);
//! this module decides who may hold an output and what a request means.
//!
//! ## Holding an output
//!
//! One client holds an output's gamma control at a time. The first `get_gamma_control`
//! takes it; a second one for the same output is answered with `failed` at once and the
//! first holder keeps it. Releasing the control, by `destroy` or by the client going
//! away, restores the output's original ramp if the holder had loaded one.
//!
//! ## Where the requests go
//!
//! A request from a client that does not hold the output is ignored: a `failed` object
//! accepts no work, as the protocol says. A ramp the hardware refuses is reported to its
//! client as `failed` by the apply step in [`crate::output_control`].
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/gamma_power.rs`.
//!
//! 1. **Two clients fight over one output.** The second gets `failed` and the first keeps
//!    the output: `a_second_gamma_control_for_an_output_fails`.
//! 2. **A holder leaves and its ramp stays.** Destroying the control, or disconnecting,
//!    restores the original ramp: `releasing_gamma_control_restores_the_original_ramp`.
//! 3. **A ramp of the wrong length reaches the hardware.** The table length is checked
//!    against the advertised size before anything is queued, and a fd carrying more bytes
//!    than the tables is a protocol error, `invalid_gamma`: `a_ramp_of_the_wrong_size_is_a_protocol_error`.
//! 4. **A descriptor that cannot be read is treated as a failed ramp.** A read error or a
//!    short read sends `failed` and loads nothing: `a_short_ramp_fails_and_loads_nothing`.
//! 5. **An output that disappears takes its control with it.** Its holders hear `failed`:
//!    `removing_an_output_fails_its_gamma_control`.
//! 6. **An unsupported backend advertises nothing it cannot do.** Without host support
//!    the global is not offered; a bind that reaches it anyway gets `failed`:
//!    `an_unsupported_backend_does_not_offer_gamma_control`.
//! 7. **A sandboxed client sees no gamma control.** The access roster hides the global
//!    from a sandboxed connection: `a_sandboxed_client_sees_no_privileged_global` in `tests/access.rs`.

use std::fs::File;
use std::io::Read;
use std::sync::Arc;

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_server::backend::{ClientId as ClientKey, GlobalId};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::{debug, trace, warn};
use wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1::{self as manager, ZwlrGammaControlManagerV1},
    zwlr_gamma_control_v1::{self as control, ZwlrGammaControlV1},
};

use gnoblin_types::OutputId;

use crate::State;
use crate::access::{Access, Global};

/// The version of `zwlr_gamma_control_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// Creates the `zwlr_gamma_control_manager_v1` global.
///
/// The access roster is captured in the global's data, so the answer to "may this
/// connection bind it" is the same answer the other privileged globals give.
pub(crate) fn global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ZwlrGammaControlManagerV1, ManagerGlobalData>(
        MANAGER_VERSION,
        ManagerGlobalData {
            access: Arc::clone(access),
        },
    )
}

/// The global data of the manager: who may bind it.
pub(crate) struct ManagerGlobalData {
    /// The roster that answers which connection may.
    access: Arc<Access>,
}

/// The user data of a bound manager object.
pub(crate) struct ManagerUserData;

/// The user data of one gamma control object.
pub(crate) struct ControlUserData {
    /// The output the object holds, or [`None`] when the object was answered `failed`.
    output: Option<OutputId>,
}

impl GlobalDispatch2<ZwlrGammaControlManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerUserData);
        debug!(client = ?client.id(), "[gamma] a client bound the gamma control manager");
    }

    fn can_view(&self, client: &Client) -> bool {
        self.access.allows(&client.id(), Global::GammaControl)
    }
}

impl Dispatch2<ZwlrGammaControlManagerV1, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &ZwlrGammaControlManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::GetGammaControl { id, output } => {
                let output = SmithayOutput::from_resource(&output).and_then(|output| state.output_id_of(&output));
                let size =
                    output.and_then(|output| state.output_controls.gamma_size(output).map(|size| (output, size)));
                match size {
                    Some((output, size)) if state.output_controls.gamma_free(output) => {
                        let object = data_init.init(id, ControlUserData { output: Some(output) });
                        state.output_controls.hold_gamma(output, object.clone());
                        object.gamma_size(size);
                        debug!(output = %output, size, "[gamma] a client took gamma control");
                    }
                    _ => {
                        let object = data_init.init(id, ControlUserData { output: None });
                        object.failed();
                        debug!("[gamma] gamma control was refused: no such output, no support, or already held");
                    }
                }
            }
            manager::Request::Destroy => {}
            _ => trace!("[gamma] an unknown manager request"),
        }
    }
}

impl Dispatch2<ZwlrGammaControlV1, State> for ControlUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        object: &ZwlrGammaControlV1,
        request: control::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            control::Request::SetGamma { fd } => {
                let Some(output) = self.output else {
                    return;
                };
                if !state.output_controls.gamma_holder_is(output, object) {
                    return;
                }
                let Some(size) = state.output_controls.gamma_size(output) else {
                    object.failed();
                    return;
                };
                match read_ramp(File::from(fd), size) {
                    Ok(tables) => state.output_controls.queue_gamma(output, tables),
                    Err(RampError::Read(error)) => {
                        warn!(%error, "[gamma] the ramp descriptor could not be read");
                        object.failed();
                    }
                    Err(RampError::Short) => {
                        warn!("[gamma] the ramp descriptor was shorter than the table");
                        object.failed();
                    }
                    Err(RampError::Long) => {
                        object.post_error(
                            control::Error::InvalidGamma,
                            "the gamma ramp is longer than three tables of the output's gamma size",
                        );
                    }
                }
            }
            control::Request::Destroy => {}
            _ => trace!("[gamma] an unknown gamma control request"),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientKey, object: &ZwlrGammaControlV1) {
        state.output_controls.release_gamma(object);
    }
}

/// Why a ramp could not be read from its descriptor.
enum RampError {
    /// The descriptor could not be read.
    Read(std::io::Error),
    /// The descriptor held fewer bytes than three tables.
    Short,
    /// The descriptor held more bytes than three tables.
    Long,
}

/// Reads three tables of `size` entries, each entry a native-endian `u16`, from `file`.
///
/// Reads one byte past the expected length, so an oversized ramp is told apart from a
/// correct one without reading the whole of an unbounded descriptor.
fn read_ramp(file: File, size: u32) -> Result<[Vec<u16>; 3], RampError> {
    let table = usize::try_from(size).unwrap_or(usize::MAX);
    let expected = table.saturating_mul(3).saturating_mul(2);
    let mut bytes = Vec::with_capacity(expected.saturating_add(1));
    file.take(u64::try_from(expected).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(RampError::Read)?;
    match bytes.len().cmp(&expected) {
        std::cmp::Ordering::Less => Err(RampError::Short),
        std::cmp::Ordering::Greater => Err(RampError::Long),
        std::cmp::Ordering::Equal => {
            let values: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_ne_bytes(*pair))
                .collect();
            let mut values = values.into_iter();
            let mut channel = || values.by_ref().take(table).collect::<Vec<u16>>();
            Ok([channel(), channel(), channel()])
        }
    }
}
