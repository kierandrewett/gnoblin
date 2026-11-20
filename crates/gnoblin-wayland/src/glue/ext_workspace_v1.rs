//! `gnoblin-wayland-ext-workspace-v1`: the workspaces the configuration declares and the one the shell shows.

use gnoblin_wayland_protocols::ext_workspace_v1::{self, Snapshot, Workspaces, workspaces_of};
use smithay::reexports::wayland_server::DisplayHandle;

use crate::State;

impl ext_workspace_v1::Host for State {
    fn workspace_snapshot(&self) -> Snapshot {
        Snapshot {
            workspaces: workspaces_of(&self.workspace_names, self.composer.active_workspace().as_deref()),
            outputs: self.outputs.iter().map(|entry| entry.output.clone()).collect(),
        }
    }

    fn workspace_display(&self) -> DisplayHandle {
        self.display_handle.clone()
    }

    fn workspaces(&mut self) -> &mut Workspaces {
        &mut self.workspace_protocol
    }
}
