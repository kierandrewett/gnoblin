//! What the compositor has to see of a request before and after Smithay handles it.
//!
//! [`crate::dispatch`] hands every request of every protocol object to Smithay's handler. A few protocols need the
//! compositor's own record of requests Smithay keeps out of reach: it discards a text input's requests while no input
//! method is bound, and it keeps a toplevel's decoration object where nothing else can read it. The tap sees such a
//! request with the state in hand, before Smithay does, and may keep it ([`before`]); and it runs once more after the
//! request has been handled ([`after`]). A request no tap is for pays a few type comparisons and nothing else.
//!
//! # Failure modes
//!
//! 1. **An input method that binds late never hears of an enabled text field.** `zwp_text_input_v3` requests are held
//!    while there is no input method, and [`after`] gives them to Smithay once one is bound:
//!    `a_text_field_enabled_before_the_input_method_binds_is_activated` in `tests/protocols.rs`.
//! 2. **A toplevel destroyed before its decoration goes unanswered.** The tap records which toplevels have a live
//!    decoration, so the destruction of one is answered with `orphaned`:
//!    `destroying_a_toplevel_before_its_decoration_is_an_error` in `tests/decoration.rs`.
//! 3. **A tablet seat never learns of the pads.** Smithay creates the seat object out of reach, so the tap notes the number
//!    the client chose and finds the object once the request is handled: `a_client_that_binds_late_sees_the_pad` in
//!    `tests/tablet.rs`.

use std::any::Any;

use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_manager_v2::{self, ZwpTabletManagerV2};
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{self, ZwpTextInputV3};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_decoration_manager_v1::{
    self, ZxdgDecorationManagerV1,
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::{
    self, ZxdgToplevelDecorationV1,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{self, XdgToplevel};
use smithay::reexports::wayland_server::{DataInit, DisplayHandle, Resource};

use crate::State;
use crate::protocols::{zwp_tablet_v2 as tablet, zxdg_decoration_v1 as decoration};
use gnoblin_wayland_protocols::zwp_text_input_v3 as text_input;

/// Moves a value between two names for one type. The tap picks the request type from the resource's, so the two are
/// the same type and the cast cannot fail.
fn same<A: 'static, B: 'static>(value: A) -> B {
    *(Box::new(value) as Box<dyn Any>)
        .downcast::<B>()
        .unwrap_or_else(|_| unreachable!("a resource's request type follows from the resource's type"))
}

/// Lets the tap see `request` before Smithay does. It comes back when Smithay is to handle it, and is `None` when the
/// tap kept it.
pub(crate) fn before<I>(
    state: &mut State,
    resource: &I,
    request: I::Request,
    dhandle: &DisplayHandle,
    data_init: &mut DataInit<'_, State>,
) -> Option<I::Request>
where
    I: Resource + 'static,
    I::Request: 'static,
{
    let resource: &dyn Any = resource;
    if let Some(input) = resource.downcast_ref::<ZwpTextInputV3>() {
        let request = same::<_, zwp_text_input_v3::Request>(request);
        if matches!(request, zwp_text_input_v3::Request::Destroy) {
            text_input::end_if_active(state, input, dhandle, data_init);
        }
        let held = text_input::hold(state, input, request);
        return held.map(same);
    }
    let seen: &dyn Any = &request;
    if let Some(toplevel) = resource.downcast_ref::<XdgToplevel>() {
        if matches!(
            seen.downcast_ref::<xdg_toplevel::Request>(),
            Some(xdg_toplevel::Request::Destroy)
        ) {
            decoration::toplevel_destroying(state, toplevel);
        }
    } else if let Some(manager) = resource.downcast_ref::<ZxdgDecorationManagerV1>() {
        if let Some(zxdg_decoration_manager_v1::Request::GetToplevelDecoration { toplevel, .. }) =
            seen.downcast_ref::<zxdg_decoration_manager_v1::Request>()
        {
            decoration::decoration_requested(state, manager, toplevel);
        }
    } else if let Some(manager) = resource.downcast_ref::<ZwpTabletManagerV2>() {
        // Smithay makes the tablet seat inside its own handler and keeps it out of reach; the number the client chose
        // for it is noted here, so the object can be found afterwards and given the pads that exist.
        if let Some(zwp_tablet_manager_v2::Request::GetTabletSeat { tablet_seat, .. }) =
            seen.downcast_ref::<zwp_tablet_manager_v2::Request>()
        {
            tablet::seat_requested(
                state,
                dhandle.get_client(manager.id()).ok(),
                tablet::protocol_id_of(tablet_seat),
            );
        }
    } else if let Some(object) = resource.downcast_ref::<ZxdgToplevelDecorationV1>()
        && matches!(
            seen.downcast_ref::<zxdg_toplevel_decoration_v1::Request>(),
            Some(zxdg_toplevel_decoration_v1::Request::Destroy)
        )
    {
        decoration::decoration_destroyed(state, object);
    }
    Some(request)
}

/// Runs after Smithay handled a request. The request that binds an input method is the only signal that what was held
/// for it can go, and every request is a chance to see it, so the check is the cheap one first.
pub(crate) fn after(state: &mut State, dhandle: &DisplayHandle, data_init: &mut DataInit<'_, State>) {
    text_input::replay(state, dhandle, data_init);
    tablet::resolve_new_seats(state, dhandle);
}
