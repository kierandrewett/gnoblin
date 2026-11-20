//! XML protocol `wlr_output_power_management_unstable_v1` (`<protocol name="wlr_output_power_management_unstable_v1">`); interfaces served: `zwlr_output_power_manager_v1`.
//!
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
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_types::OutputId;
use gnoblin_wayland_api::access::{Access, Global};
use gnoblin_wayland_api::host::OutputHost;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1::{self as manager, ZwlrOutputPowerManagerV1},
    zwlr_output_power_v1::{self as power, Mode, ZwlrOutputPowerV1},
};

gnoblin_log::logger!(log = "wayland");

/// The version of `zwlr_output_power_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwlr-output-power-management-v1",
    interfaces: &[ServedInterface::new("zwlr_output_power_manager_v1", MANAGER_VERSION)],
    privilege: Privilege::Roster(Global::OutputPower),
};

/// What the protocol needs of the compositor: who holds each output's power.
pub trait Host: OutputHost {
    /// Whether the backend can switch outputs off and on.
    fn power_supported(&self) -> bool;

    /// Whether no client holds `output`'s power.
    fn power_free(&self, output: OutputId) -> bool;

    /// Whether `output` is on.
    fn power(&self, output: OutputId) -> bool;

    /// Whether `object` is the client that holds `output`'s power.
    fn power_holder_is(&self, output: OutputId, object: &ZwlrOutputPowerV1) -> bool;

    /// Records `object` as the holder of `output`'s power.
    fn hold_power(&mut self, output: OutputId, object: ZwlrOutputPowerV1);

    /// Queues a change of `output`'s power, applied by the next frame.
    fn queue_power(&mut self, output: OutputId, on: bool);

    /// Releases the output `object` held.
    fn release_power(&mut self, object: &ZwlrOutputPowerV1);
}

/// Creates the `zwlr_output_power_manager_v1` global.
///
/// The access roster is captured in the global's data, so the answer to "may this
/// connection bind it" is the same answer the other privileged globals give.
pub fn register<D>(registrar: &Registrar<'_>) -> GlobalId
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, ZwlrOutputPowerManagementV1ManagerGlobalData> + 'static,
{
    registrar
        .display()
        .create_global::<D, ZwlrOutputPowerManagerV1, ZwlrOutputPowerManagementV1ManagerGlobalData>(
            MANAGER_VERSION,
            ZwlrOutputPowerManagementV1ManagerGlobalData {
                access: Arc::clone(registrar.access()),
            },
        )
}

/// The global data of the manager: who may bind it.
pub struct ZwlrOutputPowerManagementV1ManagerGlobalData {
    /// The roster that answers which connection may.
    access: Arc<Access>,
}

/// The user data of a bound manager object.
pub struct ZwlrOutputPowerManagementV1ManagerUserData;

/// The user data of one output power object.
pub struct ZwlrOutputPowerManagementV1PowerUserData {
    /// The output the object holds, or [`None`] when the object was answered `failed`.
    output: Option<gnoblin_types::OutputId>,
}

impl<D> GlobalDispatch2<ZwlrOutputPowerManagerV1, D> for ZwlrOutputPowerManagementV1ManagerGlobalData
where
    D: Dispatch<ZwlrOutputPowerManagerV1, ZwlrOutputPowerManagementV1ManagerUserData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ZwlrOutputPowerManagementV1ManagerUserData);
        log::debug!(area = "power", client = ?client.id(), "a client bound the output power manager");
    }

    fn can_view(&self, client: &Client) -> bool {
        self.access.allows(&client.id(), Global::OutputPower)
    }
}

impl<D> Dispatch2<ZwlrOutputPowerManagerV1, D> for ZwlrOutputPowerManagementV1ManagerUserData
where
    D: Host + Dispatch<ZwlrOutputPowerV1, ZwlrOutputPowerManagementV1PowerUserData> + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _manager: &ZwlrOutputPowerManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            manager::Request::GetOutputPower { id, output } => {
                let output = SmithayOutput::from_resource(&output).and_then(|output| state.output_id_of(&output));
                match output {
                    Some(output) if state.power_supported() && state.power_free(output) => {
                        let object =
                            data_init.init(id, ZwlrOutputPowerManagementV1PowerUserData { output: Some(output) });
                        state.hold_power(output, object.clone());
                        let mode = if state.power(output) { Mode::On } else { Mode::Off };
                        object.mode(mode);
                        log::debug!(area = "power", %output, "a client took output power control");
                    }
                    _ => {
                        let object = data_init.init(id, ZwlrOutputPowerManagementV1PowerUserData { output: None });
                        object.failed();
                        log::debug!(
                            area = "power",
                            "power control was refused: no such output, no support, or already held"
                        );
                    }
                }
            }
            manager::Request::Destroy => {}
            _ => log::trace!(area = "power", "an unknown manager request"),
        }
    }
}

impl<D: Host> Dispatch2<ZwlrOutputPowerV1, D> for ZwlrOutputPowerManagementV1PowerUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        object: &ZwlrOutputPowerV1,
        request: power::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            power::Request::SetMode { mode } => {
                let Some(output) = self.output else {
                    return;
                };
                if !state.power_holder_is(output, object) {
                    return;
                }
                match mode {
                    WEnum::Value(Mode::On) => state.queue_power(output, true),
                    WEnum::Value(Mode::Off) => state.queue_power(output, false),
                    _ => object.post_error(power::Error::InvalidMode, "the output power mode is neither off nor on"),
                }
            }
            power::Request::Destroy => {}
            _ => log::trace!(area = "power", "an unknown power request"),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientKey, object: &ZwlrOutputPowerV1) {
        state.release_power(object);
    }
}
