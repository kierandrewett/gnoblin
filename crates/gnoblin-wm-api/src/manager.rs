//! The window-manager provider: the record of windows and outputs, and the decisions the compositor asks of it.

use gnoblin_types::{Insets, OutputId, Point, Rect, Size, WindowId};

use crate::policy::NewWindowFocus;
use crate::window::{Open, OutputRecord, Standing, Window, WindowKind, WmEffect, WmRefusal};

/// What decides where windows go and which one has focus, while no shell holds the tree.
///
/// The compositor keeps exactly one provider per session. It tells the provider about every window and output, and asks
/// it for decisions as [`WmEffect`]s, which it applies. While a shell holds the tree the shell decides: the compositor
/// keeps the provider's record current and applies none of its placement, stacking or focus effects.
///
/// A provider holds no Smithay type and reads no clock. The paint order and the focus come in as [`Standing`], so the
/// scene and the keyboard stay the one record of both. A method that finds no such window changes nothing and returns no
/// effects, or [`WmRefusal::Gone`] where it can fail.
///
/// The compositor calls a provider on its own thread, once per window event and never per frame.
pub trait WindowManager: std::fmt::Debug {
    /// One window.
    fn window(&self, id: WindowId) -> Option<&Window>;

    /// Every window, by identity.
    fn windows(&self) -> Box<dyn Iterator<Item = &Window> + '_>;

    /// One connected output.
    fn output(&self, id: OutputId) -> Option<&OutputRecord>;

    /// The work area of an output.
    fn work_area(&self, id: OutputId) -> Option<Rect> {
        self.output(id).map(OutputRecord::work_area)
    }

    /// Whether the window would be drawn: it is on an output and not minimised.
    fn is_visible(&self, id: WindowId) -> bool {
        self.window(id)
            .is_some_and(|window| window.output.is_some() && !window.state.minimised)
    }

    /// Applies a window's role: its kind and parent, as its protocol says.
    fn set_role(
        &mut self,
        id: WindowId,
        kind: WindowKind,
        transient_for: Option<WindowId>,
        seat: Standing<'_>,
    ) -> Vec<WmEffect>;

    /// Whether a window that has just mapped may take focus.
    fn may_take_focus(&self, id: WindowId, policy: NewWindowFocus, seat: Standing<'_>) -> bool;

    /// Opens a window and places it.
    ///
    /// # Errors
    /// [`WmRefusal::Duplicate`], [`WmRefusal::TooMany`] or [`WmRefusal::NoOutput`]; nothing changes.
    fn open(&mut self, open: Open, seat: Standing<'_>) -> Result<Vec<WmEffect>, WmRefusal>;

    /// Forgets a closed window.
    fn close(&mut self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Focuses a window, and raises it when `raise` is set.
    fn activate(&self, id: WindowId, raise: bool, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Puts a window on top of its layer.
    fn raise(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Puts a window under every other window of its layer.
    fn lower(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Maximises a window, or returns it to its floating rectangle.
    fn set_maximised(&mut self, id: WindowId, on: bool) -> Vec<WmEffect>;

    /// Makes a window cover its output, or returns it to the rectangle it came from.
    fn set_fullscreen(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Hides a window, or brings it back.
    fn set_minimised(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect>;

    /// Moves and resizes a floating window, as a control request asks.
    ///
    /// # Errors
    /// [`WmRefusal::Gone`] for a window that is not open, [`WmRefusal::NotFloating`] for a maximised or fullscreen one.
    fn move_resize(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal>;

    /// Puts a floating window's top left at an exact position and keeps its size.
    ///
    /// # Errors
    /// [`WmRefusal::Gone`] for a window that is not open, [`WmRefusal::NotFloating`] for a maximised or fullscreen one.
    fn place(&mut self, id: WindowId, at: Point) -> Result<Vec<WmEffect>, WmRefusal>;

    /// Sets a floating window's rectangle exactly: what an interactive move or resize ends each pointer step on.
    ///
    /// # Errors
    /// [`WmRefusal::Gone`] for a window that is not open, [`WmRefusal::NotFloating`] for a maximised or fullscreen one.
    fn set_rect(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal>;

    /// Takes the size the client drew a floating window at as the window's size, and keeps its position. Nothing is
    /// sent: the client already has this size. Returns whether the record changed.
    fn adopt_size(&mut self, id: WindowId, size: Size) -> bool;

    /// Moves a window, and its transients, to another output.
    ///
    /// # Errors
    /// [`WmRefusal::Gone`], [`WmRefusal::UnknownOutput`], [`WmRefusal::FollowsParent`] or [`WmRefusal::NotFloating`].
    fn move_to_output(&mut self, id: WindowId, target: OutputId) -> Result<Vec<WmEffect>, WmRefusal>;

    /// An output arrived.
    fn output_added(&mut self, id: OutputId, rect: Rect, seat: Standing<'_>) -> Vec<WmEffect>;

    /// An output went away.
    fn output_removed(&mut self, id: OutputId, seat: Standing<'_>) -> Vec<WmEffect>;

    /// An output's rectangle changed.
    fn output_moved(&mut self, id: OutputId, rect: Rect) -> Vec<WmEffect>;

    /// The insets layer surfaces reserve on an output changed.
    fn reserve(&mut self, id: OutputId, reserved: Insets) -> Vec<WmEffect>;
}
