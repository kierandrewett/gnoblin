//! The state's dispatch glue: `Dispatch` and `GlobalDispatch` for every protocol object, with
//! the one registry gate that narrows a single-purpose role.
//!
//! Smithay's `delegate_dispatch2!(State)` would write these two impls, and the `Dispatch` one
//! below is what it writes, with one addition: [`crate::request_guard`] sees a request first. The
//! `GlobalDispatch` one differs in `can_view`, which is where the `view` role is kept off every
//! global it has no business with.
//!
//! ## Why here rather than in each global's own filter
//!
//! A privileged global carries a per-client filter its own module wires to the roster
//! ([`crate::access::decide`]). A *core* global does not: `wl_compositor`, `wl_seat`,
//! `xdg_wm_base` and `ext_session_lock_manager_v1` are created by Smithay's own states with no
//! filter, and every client is meant to see them. A role that must see none of them cannot be
//! expressed in a filter that is not there, and wiring a filter into each of fifty globals
//! would leave the next global added to default to visible.
//!
//! Every global the state serves, Smithay's and this crate's and a compat extension's, is
//! bound through `State: GlobalDispatch<I, U>`, so this is the one place every global passes.
//! The gate asks [`crate::access::ClientRole::sees_interface`], which for the `view` role is
//! an allow-list by interface name: a global added later is hidden from it until someone adds
//! the name on purpose. It runs *in addition to* the global's own filter, never instead of it.
//!
//! The backend asks `can_view` both when it lists the registry and when a client binds, so a
//! hidden global is neither advertised nor bindable by a guessed name: the backend refuses the
//! bind with a protocol error.

use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::State;
use crate::protocols::wl_compositor::WlCompositorClientState;

impl<I, UserData> Dispatch<I, UserData> for State
where
    I: Resource + 'static,
    I::Request: 'static,
    UserData: Dispatch2<I, State>,
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: <I as Resource>::Request,
        data: &UserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // A rule Smithay's handler does not enforce is checked first; a request it refuses has had its
        // protocol error posted and goes no further.
        if !crate::request_guard::admits(resource, &request) {
            return;
        }
        data.request(state, client, resource, request, dhandle, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        data.destroyed(state, client, resource);
    }
}

impl<I, UserData> GlobalDispatch<I, UserData> for State
where
    I: Resource,
    UserData: GlobalDispatch2<I, State>,
{
    fn bind(
        state: &mut Self,
        dhandle: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &UserData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        data.bind(state, dhandle, client, resource, data_init);
    }

    fn can_view(client: Client, data: &UserData) -> bool {
        role_sees_interface(&client, I::interface().name) && data.can_view(&client)
    }
}

/// Whether the connection's role may see the global named `interface` at all.
///
/// The answer comes from the roster through the registration kept in the client's own data,
/// which takes no call back into the backend, so it is safe inside a filter. A connection
/// that did not come through the accept path has no registration and no role to narrow.
fn role_sees_interface(client: &Client, interface: &str) -> bool {
    client
        .get_data::<WlCompositorClientState>()
        .is_none_or(|data| data.registration.allows_interface(interface))
}
