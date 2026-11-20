//! What a window is asked to do, whoever asks.

/// What a window is asked to do.
///
/// A client's `xdg_toplevel` request, a taskbar's ask and an X11 client's `_NET_WM_STATE` change all become one of these, so
/// the compositor decides them in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// Fill the work area (`true`) or return to the floating rectangle (`false`).
    Maximise(bool),
    /// Cover the output (`true`) or leave fullscreen (`false`).
    Fullscreen(bool),
    /// Hide the window and hand focus on (`true`) or bring it back (`false`).
    Minimise(bool),
    /// Ask the window's client to close.
    Close,
    /// Focus and raise the window. The caller has already decided the activation is allowed.
    Activate,
    /// Put the window on top of its siblings.
    Raise,
    /// Put the window under its siblings.
    Lower,
}
