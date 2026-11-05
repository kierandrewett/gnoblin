//! `zwlr_output_power_manager_v1` (version 1): a client switches an output off and on.
//!
//! Idle and lock tools use the wlroots protocol to blank a screen. A client asks for a
//! power object for one output; the object reports the output's mode at once and again
//! after every change, and `set_mode` asks the host to switch the output. The hardware
//! side is the host's [`OutputControl`](crate::output_control::OutputControl).
//!
//! ## Holding an output
//!
//! One client holds an output's power object at a time, as the protocol requires. A second
//! `get_output_power` for the same output is answered with `failed` and the first holder
//! keeps it. Releasing the object does not change the output's mode: the power state a
//! client set stays until something sets it again.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/gamma_power.rs`.
//!
//! 1. **Two clients fight over one output.** The second object is answered `failed`:
//!    `a_second_power_object_for_an_output_fails`.
//! 2. **A mode change goes unreported.** Every change, whether the client asked or the
//!    host did, is sent as a `mode` event to the holder: `a_mode_change_is_reported`.
//! 3. **A mode the protocol does not name is accepted.** An unknown value is a protocol
//!    error, `invalid_mode`: `an_invalid_mode_is_a_protocol_error`.
//! 4. **An output that disappears leaves its holder waiting.** Its holder hears `failed`:
//!    `removing_an_output_fails_its_power_object`.
//! 5. **An unsupported backend is offered a power control it cannot use.** Without host
//!    support the global is not offered: `an_unsupported_backend_does_not_offer_power_control`.

use std::sync::Arc;

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_server::backend::{ClientId as ClientKey, GlobalId};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::{debug, trace};
use wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1::{self as manager, ZwlrOutputPowerManagerV1},
    zwlr_output_power_v1::{self as power, Mode, ZwlrOutputPowerV1},
};

use crate::State;
use crate::access::{Access, Global};

/// The version of `zwlr_output_power_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// Creates the `zwlr_output_power_manager_v1` global.
pub(crate) fn global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ZwlrOutputPowerManagerV1, ManagerGlobalData>(
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

/// The user data of one output power object.
pub(crate) struct PowerUserData {
    /// The output the object holds, or [`None`] when the object was answered `failed`.
    output: Option<gnoblin_types::OutputId>,
}

impl GlobalDispatch2<ZwlrOutputPowerManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerUserData);
        debug!(client = ?client.id(), "[power] a client bound the output power manager");
    }

    fn can_view(&self, client: &Client) -> bool {
        self.access.allows(&client.id(), Global::OutputPower)
    }
}

impl Dispatch2<ZwlrOutputPowerManagerV1, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &ZwlrOutputPowerManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::GetOutputPower { id, output } => {
                let output = SmithayOutput::from_resource(&output).and_then(|output| state.output_id_of(&output));
                match output {
                    Some(output)
                        if state.output_controls.power_supported() && state.output_controls.power_free(output) =>
                    {
                        let object = data_init.init(id, PowerUserData { output: Some(output) });
                        state.output_controls.hold_power(output, object.clone());
                        let mode = if state.output_controls.power(output) {
                            Mode::On
                        } else {
                            Mode::Off
                        };
                        object.mode(mode);
                        debug!(%output, "[power] a client took output power control");
                    }
                    _ => {
                        let object = data_init.init(id, PowerUserData { output: None });
                        object.failed();
                        debug!("[power] power control was refused: no such output, no support, or already held");
                    }
                }
            }
            manager::Request::Destroy => {}
            _ => trace!("[power] an unknown manager request"),
        }
    }
}

impl Dispatch2<ZwlrOutputPowerV1, State> for PowerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        object: &ZwlrOutputPowerV1,
        request: power::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            power::Request::SetMode { mode } => {
                let Some(output) = self.output else {
                    return;
                };
                if !state.output_controls.power_holder_is(output, object) {
                    return;
                }
                match mode {
                    WEnum::Value(Mode::On) => state.output_controls.queue_power(output, true),
                    WEnum::Value(Mode::Off) => state.output_controls.queue_power(output, false),
                    _ => object.post_error(power::Error::InvalidMode, "the output power mode is neither off nor on"),
                }
            }
            power::Request::Destroy => {}
            _ => trace!("[power] an unknown power request"),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientKey, object: &ZwlrOutputPowerV1) {
        state.output_controls.release_power(object);
    }
}
