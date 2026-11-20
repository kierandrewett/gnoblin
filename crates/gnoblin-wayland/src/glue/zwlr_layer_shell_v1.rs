//! `gnoblin-wayland-zwlr-layer-shell-v1`: Smithay's handler forwards into the compositor's layer engine.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface, LayerSurfaceConfigure, WlrLayerShellHandler, WlrLayerShellState,
};

use crate::State;
use crate::log;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: LayerSurface, output: Option<WlOutput>, layer: Layer, namespace: String) {
        self.layer_created(surface, output, layer, namespace);
    }

    fn new_popup(&mut self, parent: LayerSurface, popup: smithay::wayland::shell::xdg::PopupSurface) {
        self.layer_popup_created(&parent, &popup);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: LayerSurfaceConfigure) {
        // Like a toplevel, a layer surface's acknowledgement is its own business:
        // the next commit carries the state and nothing waits on it here.
        log::trace!(area = "layer", serial = ?configure.serial,
            size = ?configure.state.size,
            surface = ?surface.id(),
            "configure acknowledged"
        );
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        self.layer_destroyed(&surface);
    }
}
