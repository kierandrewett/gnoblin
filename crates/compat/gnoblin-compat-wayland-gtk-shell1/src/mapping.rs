//! The protocol's values, mapped onto the compositor's.
//!
//! Everything here is a pure function of the protocol's own numbers and the three
//! configured titlebar actions, so the mapping is testable without a socket, a seat
//! or a session. The dispatch in [`crate::global`] performs what this module answers;
//! nothing here touches a surface.
//!
//! The one rule the module exists for: a gesture is **not** an action. GTK sends
//! `double_click`, `middle_click` or `right_click`, and which of those maximises,
//! lowers or opens a menu is `window_management`'s decision. A mapping that baked the
//! actions in would make the configuration a lie for exactly the applications that
//! draw their own titlebars.

use gnoblin_config::TitlebarAction;
use gnoblin_wayland::TitlebarActions;

/// The three titlebar gestures the protocol defines.
///
/// The numbers are the protocol's own `gesture` enum, which is not ordered by
/// convention: 1 is a double click, 2 a right click and 3 a middle click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gesture {
    /// Two clicks in quick succession on the titlebar.
    DoubleClick,
    /// A right click on the titlebar.
    RightClick,
    /// A middle click on the titlebar.
    MiddleClick,
}

/// A gesture number the protocol does not define.
///
/// The protocol's answer is the `invalid_gesture` error on that client, so this is a
/// value rather than a default: a caller that treated it as a double click would
/// maximise a window because a client sent a number it invented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UnknownGesture {
    /// The number the client sent.
    pub raw: u32,
}

impl Gesture {
    /// The gesture a protocol number names.
    ///
    /// # Errors
    ///
    /// [`UnknownGesture`] for anything other than 1, 2 or 3.
    pub fn parse(raw: u32) -> Result<Self, UnknownGesture> {
        match raw {
            1 => Ok(Self::DoubleClick),
            2 => Ok(Self::RightClick),
            3 => Ok(Self::MiddleClick),
            raw => Err(UnknownGesture { raw }),
        }
    }
}

/// The configured action a gesture names.
#[must_use]
pub fn action_for(gesture: Gesture, actions: &TitlebarActions) -> TitlebarAction {
    match gesture {
        Gesture::DoubleClick => actions.double_click,
        Gesture::RightClick => actions.right_click,
        Gesture::MiddleClick => actions.middle_click,
    }
}

/// The gesture a client sent, resolved to the action the configuration declares.
///
/// # Errors
///
/// [`UnknownGesture`] for a number the protocol does not define.
pub fn resolve(raw: u32, actions: &TitlebarActions) -> Result<TitlebarAction, UnknownGesture> {
    Gesture::parse(raw).map(|gesture| action_for(gesture, actions))
}

/// One `gtk_surface1` request, in the vocabulary the host traits use.
///
/// The dispatch turns a protocol request into one of these and hands it to the host,
/// so the mapping from wire values to desktop meanings is a plain value a test can
/// compare, and the code that performs it is one `match` in [`crate::global`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Requested {
    /// `set_dbus_properties`: the application id the client declared.
    AppId(String),
    /// `set_modal` or `unset_modal`.
    Modal(bool),
    /// `present`, with the client's own timestamp.
    Present(u32),
    /// `request_focus`, with the startup id the client named.
    Focus(Option<String>),
    /// `titlebar_gesture`, resolved to the configured action.
    Titlebar(TitlebarAction),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The configuration's three actions reach the gesture that names them, which is
    /// what makes the setting apply to GTK's own titlebars.
    #[test]
    fn a_gesture_becomes_the_configured_action() {
        let actions = TitlebarActions {
            double_click: TitlebarAction::ToggleMaximize,
            middle_click: TitlebarAction::Minimize,
            right_click: TitlebarAction::Lower,
        };
        assert_eq!(resolve(1, &actions), Ok(TitlebarAction::ToggleMaximize));
        assert_eq!(resolve(3, &actions), Ok(TitlebarAction::Minimize));
        assert_eq!(resolve(2, &actions), Ok(TitlebarAction::Lower));
        assert_eq!(
            resolve(1, &TitlebarActions::default()),
            Ok(TitlebarAction::ToggleMaximize),
            "the defaults are the configuration's own"
        );
    }

    /// A number the protocol does not define is not a gesture, and is not rounded to
    /// one: the dispatch posts `invalid_gesture` instead.
    #[test]
    fn an_unknown_gesture_is_not_a_click() {
        assert_eq!(Gesture::parse(0), Err(UnknownGesture { raw: 0 }));
        assert_eq!(Gesture::parse(4), Err(UnknownGesture { raw: 4 }));
        assert!(resolve(u32::MAX, &TitlebarActions::default()).is_err());
    }
}
