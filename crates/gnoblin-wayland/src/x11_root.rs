//! The root-window messages Smithay's X11 window manager does not decode.
//!
//! An X11 client talks to its window manager by sending client messages to the root window. Smithay's `X11Wm` decodes
//! `_NET_WM_STATE`, `WM_CHANGE_STATE`, `_NET_ACTIVE_WINDOW` and `_NET_WM_MOVERESIZE`, and answers the rest with a
//! debug line. Three the EWMH names are left out, and a pager, `wmctrl`, a toolkit asking to be closed or a client
//! placing its own window with them gets nothing back:
//!
//! - `_NET_CLOSE_WINDOW` asks the window manager to close a window the way its close button does, which is the
//!   window's own `WM_DELETE_WINDOW` and never a kill.
//! - `_NET_MOVERESIZE_WINDOW` asks for a rectangle, with a flag for each of its four fields.
//! - `_NET_REQUEST_FRAME_EXTENTS` asks how wide the frame the window manager will draw round the window is, before the
//!   window is mapped. Gnoblin draws no frame, so the answer is always four zeros.
//!
//! Smithay owns the window manager's X connection and exposes no hook for a message it does not know. A message sent
//! to the root window with the mask the EWMH prescribes (`SubstructureNotify | SubstructureRedirect`) reaches every
//! client that selected either on it, though, and selecting `SubstructureNotify` is not exclusive the way
//! `SubstructureRedirect` is. A second connection to the same display selects it, advertises the three messages in
//! `_NET_SUPPORTED` (which is what makes a toolkit that checks the property send them), and hands what it decodes to
//! the same window policy every other X11 event goes through.
//!
//! # Failure modes first
//!
//! 1. **A message for a window that is not managed does something.** The decoded message names an X11 window and the
//!    policy answers for it: an override-redirect menu or a window that is gone is nothing
//!    (`a_state_ask_from_an_unmanaged_window_changes_nothing` in `gnoblin-xwayland`).
//! 2. **A move flag is read as a value.** A field is a request only when its flag is set, so a client that moves a
//!    window does not also resize it to whatever the unused words hold: `only_the_fields_whose_flag_is_set_are_asked`.
//! 3. **A message of another type is taken for one of ours.** The decoder compares the message type against the
//!    atoms it interned and nothing else: `a_message_of_another_type_is_not_decoded`.
//! 4. **The watcher outlives the display.** The source is removed when the session's attempt ends, and its worker
//!    thread leaves with the connection, so a restart does not leave two watchers on one session.

use std::sync::Arc;

use smithay::reexports::calloop::channel::Event as ChannelEvent;
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};
use smithay::utils::x11rb::X11Source;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::State;
use crate::log;

/// The atoms a message is told apart by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kinds {
    /// `_NET_CLOSE_WINDOW`.
    pub(crate) close: u32,
    /// `_NET_MOVERESIZE_WINDOW`.
    pub(crate) move_resize: u32,
    /// `_NET_REQUEST_FRAME_EXTENTS`.
    pub(crate) frame_extents: u32,
}

/// A decoded root-window message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootMessage {
    /// `_NET_CLOSE_WINDOW`: close the window.
    Close {
        /// The X11 window.
        window: u32,
    },
    /// `_NET_MOVERESIZE_WINDOW`: the window's rectangle, with only the fields the client flagged.
    MoveResize {
        /// The X11 window.
        window: u32,
        /// The wanted x, when the client flagged it.
        x: Option<i32>,
        /// The wanted y, when the client flagged it.
        y: Option<i32>,
        /// The wanted width, when the client flagged it.
        width: Option<u32>,
        /// The wanted height, when the client flagged it.
        height: Option<u32>,
    },
    /// `_NET_REQUEST_FRAME_EXTENTS`: tell the window its frame.
    FrameExtents {
        /// The X11 window.
        window: u32,
    },
}

/// The bits of the first word of `_NET_MOVERESIZE_WINDOW` that say which of the four fields are valid.
const X_VALID: u32 = 1 << 8;
const Y_VALID: u32 = 1 << 9;
const WIDTH_VALID: u32 = 1 << 10;
const HEIGHT_VALID: u32 = 1 << 11;

/// Decodes a 32-bit client message to the root window, or `None` for a type this module does not own.
pub(crate) fn decode(kinds: Kinds, kind: u32, window: u32, data: [u32; 5]) -> Option<RootMessage> {
    if kind == kinds.close {
        return Some(RootMessage::Close { window });
    }
    if kind == kinds.frame_extents {
        return Some(RootMessage::FrameExtents { window });
    }
    if kind != kinds.move_resize {
        return None;
    }
    let flags = data[0];
    let signed = |word: u32| i32::from_ne_bytes(word.to_ne_bytes());
    Some(RootMessage::MoveResize {
        window,
        x: (flags & X_VALID != 0).then(|| signed(data[1])),
        y: (flags & Y_VALID != 0).then(|| signed(data[2])),
        width: (flags & WIDTH_VALID != 0).then_some(data[3]),
        height: (flags & HEIGHT_VALID != 0).then_some(data[4]),
    })
}

/// The second connection to the display, and the event source that reads it.
///
/// Dropping it removes the source, which stops the worker thread that reads the connection.
pub(crate) struct RootWatch {
    handle: LoopHandle<'static, State>,
    token: RegistrationToken,
    connection: Arc<RustConnection>,
    /// The root window's `_NET_FRAME_EXTENTS` for a window is written on this atom.
    frame_extents: u32,
}

impl Drop for RootWatch {
    fn drop(&mut self) {
        self.handle.remove(self.token);
    }
}

impl RootWatch {
    /// Connects to display `display`, advertises the three messages and starts reading them.
    pub(crate) fn start(handle: &LoopHandle<'static, State>, display: u32) -> Result<Self, String> {
        let failed = |error: &dyn std::fmt::Display| format!("the root message watcher could not start: {error}");
        let (connection, screen) = RustConnection::connect(Some(&format!(":{display}"))).map_err(|e| failed(&e))?;
        let connection = Arc::new(connection);
        let root = connection.setup().roots[screen].root;
        let intern = |name: &str| -> Result<u32, String> {
            Ok(connection
                .intern_atom(false, name.as_bytes())
                .map_err(|e| failed(&e))?
                .reply()
                .map_err(|e| failed(&e))?
                .atom)
        };
        let kinds = Kinds {
            close: intern("_NET_CLOSE_WINDOW")?,
            move_resize: intern("_NET_MOVERESIZE_WINDOW")?,
            frame_extents: intern("_NET_REQUEST_FRAME_EXTENTS")?,
        };
        let frame_extents = intern("_NET_FRAME_EXTENTS")?;
        let supported = intern("_NET_SUPPORTED")?;
        // The window the source's worker thread is woken through when the watcher is dropped, and the message type it
        // is woken with.
        let wake = connection.generate_id().map_err(|e| failed(&e))?;
        connection
            .create_window(
                0,
                wake,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new(),
            )
            .map_err(|e| failed(&e))?;
        let wake_kind = intern("_GNOBLIN_ROOT_WATCH_CLOSE")?;
        connection
            .change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
            )
            .map_err(|e| failed(&e))?;
        // A toolkit sends these messages only to a window manager that says it understands them.
        connection
            .change_property32(
                PropMode::APPEND,
                root,
                supported,
                AtomEnum::ATOM,
                &[kinds.close, kinds.move_resize, kinds.frame_extents],
            )
            .map_err(|e| failed(&e))?;
        connection.flush().map_err(|e| failed(&e))?;

        let source = X11Source::new(Arc::clone(&connection), wake, wake_kind);
        let reader = Arc::clone(&connection);
        let token = handle
            .insert_source(source, move |event, _, state| {
                let ChannelEvent::Msg(Event::ClientMessage(message)) = event else {
                    return;
                };
                if message.format != 32 {
                    return;
                }
                let Some(decoded) = decode(kinds, message.type_, message.window, message.data.as_data32()) else {
                    return;
                };
                match decoded {
                    RootMessage::FrameExtents { window } => {
                        write_frame_extents(&reader, frame_extents, window);
                    }
                    RootMessage::Close { .. } | RootMessage::MoveResize { .. } => state.x11_root_message(decoded),
                }
            })
            .map_err(|e| failed(&e))?;
        Ok(Self {
            handle: handle.clone(),
            token,
            connection,
            frame_extents,
        })
    }

    /// Tells a window it has no frame: `_NET_FRAME_EXTENTS` is four zeros, which is what a window manager that draws
    /// no decoration says. Set when the window maps, and again on request.
    pub(crate) fn set_frame_extents(&self, window: u32) {
        write_frame_extents(&self.connection, self.frame_extents, window);
    }
}

/// Writes `_NET_FRAME_EXTENTS` as left, right, top and bottom of zero.
fn write_frame_extents(connection: &RustConnection, atom: u32, window: u32) {
    let written = connection
        .change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &[0, 0, 0, 0])
        .and_then(|_| connection.flush());
    if let Err(error) = written {
        log::debug!(area = "xwayland", %error, window, "the frame extents were not written");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: Kinds = Kinds {
        close: 100,
        move_resize: 101,
        frame_extents: 102,
    };

    // A close message names the window it closes, whatever else its words hold.
    #[test]
    fn a_close_message_names_its_window() {
        assert_eq!(
            decode(KINDS, 100, 7, [1, 0, 0, 0, 0]),
            Some(RootMessage::Close { window: 7 })
        );
        assert_eq!(
            decode(KINDS, 102, 7, [0; 5]),
            Some(RootMessage::FrameExtents { window: 7 })
        );
    }

    // A client that sets only the position flags asks for a position: the size words it left behind are not a size.
    #[test]
    fn only_the_fields_whose_flag_is_set_are_asked() {
        let flags = |bits: u32| bits;
        assert_eq!(
            decode(KINDS, 101, 7, [flags(X_VALID | Y_VALID), 40, 50, 999, 999]),
            Some(RootMessage::MoveResize {
                window: 7,
                x: Some(40),
                y: Some(50),
                width: None,
                height: None,
            })
        );
        assert_eq!(
            decode(KINDS, 101, 7, [flags(WIDTH_VALID | HEIGHT_VALID), 1, 2, 640, 480]),
            Some(RootMessage::MoveResize {
                window: 7,
                x: None,
                y: None,
                width: Some(640),
                height: Some(480),
            })
        );
        assert_eq!(
            decode(KINDS, 101, 7, [0, 1, 2, 3, 4]),
            Some(RootMessage::MoveResize {
                window: 7,
                x: None,
                y: None,
                width: None,
                height: None,
            }),
            "a message that flags nothing asks for nothing"
        );
    }

    // A position left of or above the screen is a negative number in a 32-bit word.
    #[test]
    fn a_position_off_the_screen_is_negative() {
        let word = u32::from_ne_bytes((-30_i32).to_ne_bytes());
        assert_eq!(
            decode(KINDS, 101, 7, [X_VALID | Y_VALID, word, word, 0, 0]),
            Some(RootMessage::MoveResize {
                window: 7,
                x: Some(-30),
                y: Some(-30),
                width: None,
                height: None,
            })
        );
    }

    // Any other message to the root window is somebody else's: Smithay's window manager answers it.
    #[test]
    fn a_message_of_another_type_is_not_decoded() {
        assert_eq!(decode(KINDS, 7, 7, [0; 5]), None);
        assert_eq!(decode(KINDS, 0, 7, [X_VALID, 1, 2, 3, 4]), None);
    }
}
