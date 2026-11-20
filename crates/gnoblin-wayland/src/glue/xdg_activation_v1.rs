//! `gnoblin-wayland-xdg-activation-v1`: Smithay's handler forwards to the crate's token rules; the state says what a
//! connection has earned, what the clock reads and what an allowed activation does to a window.

use gnoblin_types::{Tick, WindowId};
use gnoblin_wayland_api::access::ClientRole;
use gnoblin_wayland_protocols::xdg_activation_v1::{self, Host, XdgActivationV1};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

use crate::State;
use crate::window_request::WindowRequest;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn token_created(&mut self, token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        xdg_activation_v1::token_created(self, &token, &data)
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        xdg_activation_v1::redeem(self, token.as_str(), &token_data, &surface);
    }
}

impl Host for State {
    fn activation(&mut self) -> &mut XdgActivationV1 {
        &mut self.activation
    }

    fn client_role(&self, client: &ClientId) -> ClientRole {
        self.access.role(client)
    }

    fn now(&self) -> Tick {
        self.turn
    }

    fn window_activated(&mut self, window: WindowId, from_launcher: bool) {
        if from_launcher {
            // The window answers a token the session's own launcher minted for the program it started; the shell reads
            // it as `from_launcher`.
            self.composer.set_window_from_launcher(window);
        }
        self.window_request(window, WindowRequest::Activate);
    }

    fn activation_changed(&mut self) {
        self.publish();
    }
}

impl State {
    /// Tells the authority whether the session is covered, from the lock's state.
    pub(crate) fn sync_authority_lock(&mut self) {
        let locked = self.lock.state() != gnoblin_lock::State::Unlocked;
        let now = self.turn;
        self.activation.lock_changed(now, locked);
    }
}
