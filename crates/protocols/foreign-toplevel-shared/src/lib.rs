//! What both toplevel lists publish about a window, and what changed since it was last sent.
//!
//! `ext_foreign_toplevel_list_v1` and `zwlr_foreign_toplevel_manager_v1` are fed from one record of each window, so a
//! title is announced once to both and an idle iteration announces nothing. The record is plain data: the compositor
//! reads its windows into it, and each protocol crate sends what changed.

use gnoblin_types::{OutputId, WindowId};

/// The state flags of a window, as the window manager states them: one set, not a list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct States(pub u32);

impl States {
    /// The window is maximised.
    pub const MAXIMISED: u32 = 1 << 0;
    /// The window is minimised.
    pub const MINIMISED: u32 = 1 << 1;
    /// The window holds keyboard focus.
    pub const ACTIVATED: u32 = 1 << 2;
    /// The window is fullscreen.
    pub const FULLSCREEN: u32 = 1 << 3;
}

/// What one window's handles have been told, so an event only goes out when a value
/// actually changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// The title last sent.
    pub title: String,
    /// The application id last sent.
    pub app_id: String,
    /// The state flags last sent.
    pub states: States,
    /// The outputs the shell has been told about.
    pub outputs: Vec<OutputId>,
    /// Whether the initial batch has been sent, so a shell gets a `done` even when
    /// every value is the default one.
    pub introduced: bool,
}

/// One window, as the window manager states it for publication.
pub struct ToplevelState {
    /// The window manager's identity for the window.
    pub window: WindowId,
    /// The title as the window manager reports it.
    pub title: String,
    /// The application id as the window manager reports it.
    pub app_id: String,
    /// The state flags as the window manager reports them.
    pub states: States,
    /// The outputs the window is visible on; empty when it is on another workspace.
    pub outputs: Vec<OutputId>,
}

/// Which values of one window changed since they were last sent.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToplevelChanges {
    /// The title changed.
    pub title: bool,
    /// The application id changed.
    pub app_id: bool,
    /// The state flags changed.
    pub states: bool,
    /// The visible outputs changed.
    pub outputs: bool,
    /// The window has not been introduced to the lists yet.
    pub introduce: bool,
}

impl ToplevelChanges {
    /// Whether anything changed, which is when a `done` goes out.
    pub fn any(self) -> bool {
        self.title || self.app_id || self.states || self.outputs || self.introduce
    }
}
