//! Managed windows and the states the window manager derives geometry from.

use gnoblin_types::{OutputId, Rect, WindowId, WorkspaceId};

use crate::snapping::SnapZone;

/// What kind of toplevel a window is.
///
/// The kind decides placement: dialogs and modals stay with their parent instead
/// of going through smart placement, and window rules that place a window on a
/// workspace never move a transient away from its parent.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum WindowKind {
    /// An application toplevel.
    #[default]
    Normal,
    /// A dialog with a parent window.
    Dialog,
    /// A modal dialog, which blocks its parent.
    Modal,
}

impl WindowKind {
    /// Whether the window follows its parent instead of being placed freely.
    #[must_use]
    pub const fn is_transient(self) -> bool {
        !matches!(self, Self::Normal)
    }
}

/// The states a window can be in, excluding focus.
///
/// These are flags rather than one enum because they compose: a window can be
/// minimised while fullscreen, and a fullscreen window returns to its previous
/// maximised or snapped state when it leaves fullscreen. Keeping them separate is
/// what stops a fullscreen window from being lost by a workspace switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WindowState {
    /// The window fills its output's work area.
    pub maximised: bool,
    /// The window fills its whole output, over panels.
    pub fullscreen: bool,
    /// The window is hidden and out of the focus rotation.
    pub minimised: bool,
    /// The snapping zone the window is occupying, if any.
    pub snapped: Option<SnapZone>,
}

/// One window the manager is responsible for.
///
/// The fields are readable from outside the crate but the window manager owns them:
/// readers get a shared reference and can never mutate a window behind the state
/// machine's back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    /// The caller's identity for this window.
    pub id: WindowId,
    /// The application id used by window rules, or the WM class when the client
    /// has none.
    pub app_id: String,
    /// The current title, used by window rules at map time.
    pub title: String,
    /// What kind of toplevel this is.
    pub kind: WindowKind,
    /// The window this transient follows, when it has one.
    pub transient_for: Option<WindowId>,
    /// The workspace the window belongs to, which never changes by itself.
    pub workspace: WorkspaceId,
    /// The output the window is on; `None` only while no output is connected.
    pub output: Option<OutputId>,
    /// The window's frame rectangle in global logical coordinates.
    pub geometry: Rect,
    /// The floating rectangle to return to when leaving maximise, fullscreen or a
    /// snap.
    pub restore: Rect,
    /// Maximise, fullscreen, minimise and snap flags.
    pub state: WindowState,
    /// How many window rules applied completely to this window.
    ///
    /// Rules run once, when the window is taken into management. The counter makes
    /// that observable, so a test can prove that a title change or a rules reload
    /// did not apply them again, and a rule whose workspace target was missing is
    /// not counted because nothing was applied.
    pub applied_rules: usize,
}

impl Window {
    /// Whether the window would be drawn right now.
    ///
    /// A window is visible when it is not minimised, has an output that can show it
    /// and belongs to the active workspace. Focus eligibility uses the same
    /// question, so an invisible window can never hold focus.
    #[must_use]
    pub fn is_visible(&self, active_workspace: Option<WorkspaceId>) -> bool {
        !self.state.minimised && self.output.is_some() && Some(self.workspace) == active_workspace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        Window {
            id: WindowId::new(1),
            app_id: "org.example.App".to_owned(),
            title: "Document".to_owned(),
            kind: WindowKind::Normal,
            transient_for: None,
            workspace: WorkspaceId::new(1),
            output: Some(OutputId::new(0)),
            geometry: Rect::new(0, 0, 100, 100),
            restore: Rect::new(0, 0, 100, 100),
            state: WindowState::default(),
            applied_rules: 0,
        }
    }

    #[test]
    fn a_window_is_visible_only_on_the_active_workspace() {
        let window = window();
        assert!(window.is_visible(Some(WorkspaceId::new(1))));
        assert!(!window.is_visible(Some(WorkspaceId::new(2))));
        assert!(!window.is_visible(None));
    }

    #[test]
    fn a_minimised_or_outputless_window_is_not_visible() {
        let mut minimised = window();
        minimised.state.minimised = true;
        assert!(!minimised.is_visible(Some(WorkspaceId::new(1))));

        let mut orphaned = window();
        orphaned.output = None;
        assert!(!orphaned.is_visible(Some(WorkspaceId::new(1))));
    }

    #[test]
    fn only_normal_toplevels_are_placed_freely() {
        assert!(!WindowKind::Normal.is_transient());
        assert!(WindowKind::Dialog.is_transient());
        assert!(WindowKind::Modal.is_transient());
    }
}
