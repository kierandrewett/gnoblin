//! XML protocol `xdg_output_unstable_v1` (`<protocol name="xdg_output_unstable_v1">`); interfaces served: `zxdg_output_manager_v1`.
//!
//! `zxdg_output_manager_v1` (version 3): the size and place of an output in the desktop.
//!
//! Smithay serves this protocol too, but derives the logical size from the output's current mode divided by its
//! scale, with no way to say otherwise. A supersampled output is told a scale of `u * s` while its mode stays the
//! real one, so Smithay's arithmetic would report `mode / (u * s)` and shrink the desktop. This module reads the
//! logical rectangle from the layout instead, which is what the protocol describes ("the size of the output in the
//! global compositor space") and which does not depend on the supersample factor. See `design/supersample.md`.
//!
//! The event order is the one the protocol and Smithay's own handler use: position and size, then name and description
//! (version 2 and later), then `done` for versions before 3, then `wl_output.done` so a client applies everything at
//! once. A client's own scale (Xwayland's) multiplies the values, as it does in Smithay's handler.
//!
//! # Failure modes
//!
//! Each is a test in `tests/supersample.rs`.
//!
//! 1. **A supersampled output reports a smaller desktop.** The size is the layout rectangle's:
//!    `a_supersampled_output_keeps_its_logical_size_everywhere`.
//! 2. **An object for an output that is gone is silent.** It is created and told nothing, so the client is not sent
//!    a size of nothing.

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_protocols::xdg::xdg_output::zv1::server::{
    zxdg_output_manager_v1::{self as manager, ZxdgOutputManagerV1},
    zxdg_output_v1::{self as xdg_output, ZxdgOutputV1},
};
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, Weak};
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_types::{OutputId, Rect};

use crate::State;

/// The version of `zxdg_output_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 3;

/// Creates the `zxdg_output_manager_v1` global. It is offered to every client: the extent of an output is what
/// `wl_output` already tells anyone.
pub(crate) fn global(display_handle: &DisplayHandle) -> smithay::reexports::wayland_server::backend::GlobalId {
    display_handle.create_global::<State, ZxdgOutputManagerV1, ZxdgOutputManagerGlobalData>(
        MANAGER_VERSION,
        ZxdgOutputManagerGlobalData,
    )
}

/// The global data of the manager.
pub(crate) struct ZxdgOutputManagerGlobalData;

/// The user data of a bound manager object.
pub(crate) struct ZxdgOutputManagerUserData;

/// The user data of one `zxdg_output_v1`. Which output it describes is in [`XdgOutputs`], where a change of the
/// output's rectangle finds it.
pub(crate) struct ZxdgOutputUserData;

/// The `zxdg_output_v1` objects clients hold, so a change of an output's rectangle reaches them.
#[derive(Default)]
pub(crate) struct XdgOutputs {
    instances: Vec<(Weak<ZxdgOutputV1>, OutputId)>,
}

impl XdgOutputs {
    /// Remembers a new object for `output`.
    fn add(&mut self, object: &ZxdgOutputV1, output: OutputId) {
        self.instances.push((object.downgrade(), output));
    }

    /// Forgets `destroyed`, and every other object that is gone with it.
    fn forget(&mut self, destroyed: &ZxdgOutputV1) {
        self.instances
            .retain(|(object, _)| object.id() != destroyed.id() && object.upgrade().is_ok());
    }

    /// The live objects of `output`.
    fn of(&self, output: OutputId) -> Vec<ZxdgOutputV1> {
        self.instances
            .iter()
            .filter(|(_, id)| *id == output)
            .filter_map(|(object, _)| object.upgrade().ok())
            .collect()
    }
}

/// `value` as a client with `scale` counts it, rounded to a whole pixel.
fn to_client(value: i32, scale: f64) -> i32 {
    (f64::from(value) * scale).round() as i32
}

impl State {
    /// Sends `object` the position and size of `rect`, in the units of the client that holds it.
    fn send_xdg_geometry(&self, object: &ZxdgOutputV1, rect: Rect) {
        let scale = object
            .client()
            .map_or(1.0, |client| self.client_compositor_state(&client).client_scale());
        object.logical_position(to_client(rect.x, scale), to_client(rect.y, scale));
        object.logical_size(
            to_client(i32::try_from(rect.width).unwrap_or(i32::MAX), scale),
            to_client(i32::try_from(rect.height).unwrap_or(i32::MAX), scale),
        );
        // `done` is deprecated from version 3: `wl_output.done` ends the update there.
        if object.version() < 3 {
            object.done();
        }
    }

    /// Tells every `zxdg_output_v1` of `output` where it is and how large, after the layout changed its rectangle.
    ///
    /// It runs before the `wl_output` state changes, because that change sends `wl_output.done`, which has to be the
    /// last event of an update.
    pub(crate) fn update_xdg_outputs(&mut self, output: OutputId, rect: Rect) {
        for object in self.xdg_outputs.of(output) {
            self.send_xdg_geometry(&object, rect);
        }
    }
}

impl GlobalDispatch2<ZxdgOutputManagerV1, State> for ZxdgOutputManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgOutputManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ZxdgOutputManagerUserData);
    }
}

impl Dispatch2<ZxdgOutputManagerV1, State> for ZxdgOutputManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &ZxdgOutputManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::GetXdgOutput { id, output } => {
                let known = SmithayOutput::from_resource(&output).and_then(|found| state.output_id_of(&found));
                let object = data_init.init(id, ZxdgOutputUserData);
                let Some(backend) = known else {
                    return;
                };
                let Some(rect) = state.output_rect(backend) else {
                    return;
                };
                state.xdg_outputs.add(&object, backend);
                state.send_xdg_geometry(&object, rect);
                if object.version() >= 2
                    && let Some(found) = SmithayOutput::from_resource(&output)
                {
                    object.name(found.name());
                    object.description(found.description());
                    // Names and descriptions come with their own `done` on the versions that still have one.
                    if object.version() < 3 {
                        object.done();
                    }
                }
                if output.version() >= wl_output::EVT_DONE_SINCE {
                    output.done();
                }
            }
            manager::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch2<ZxdgOutputV1, State> for ZxdgOutputUserData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _object: &ZxdgOutputV1,
        request: xdg_output::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // The only request is `destroy`, which needs no work: the table drops the object in `destroyed`.
        let _ = request;
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        object: &ZxdgOutputV1,
    ) {
        state.xdg_outputs.forget(object);
    }
}
