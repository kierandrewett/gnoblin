//! The seam an Xwayland session meets the compositor through.
//!
//! `gnoblin-xwayland` owns the X11 side: the start parameters the `xwayland`
//! configuration section produces, the supervision policy that decides when a dead
//! server is started again, and the window table that turns X11 windows into the
//! events `gnoblin-wm` already consumes. `gnoblin-wayland` owns the Wayland side:
//! it spawns the process, hands Smithay's `X11Wm` the X connection and performs
//! the X11 requests the policy asks for.
//!
//! Neither crate may name the other, and both may name this one, so the vocabulary
//! they share lives here. The rule shapes the types below:
//!
//! - Everything is plain data. No Smithay type, no `x11rb` connection and no
//!   protocol object crosses the seam, so a policy can be unit-tested with no X
//!   server at all and the Wayland crate stays the only place that knows how an
//!   X11 surface is configured.
//! - The policy decides, the compositor performs. [`Windows::handle`] answers with
//!   the `gnoblin-wm` effects and the handful of X11 requests the compositor has to
//!   send; it never touches a surface itself.
//! - The clock is a parameter. [`Session`] is ticked by its caller, so a restart
//!   policy is a deterministic function of the instants it is handed rather than a
//!   thread with a sleep in it.
//!
//! # Failure modes first
//!
//! The traits here are the contract for the failures `gnoblin-xwayland` covers;
//! each one is a test in that crate (`crates/gnoblin-xwayland/tests/` for the ones
//! that need the real server, `src/*.rs` for the pure ones).
//!
//! 1. **A crash becomes a restart loop.** [`Session::failed`] answers
//!    [`Restart::RetryAfter`] with a growing delay and [`Restart::Stop`] once the
//!    attempts are spent, so the caller never has to invent a policy of its own.
//! 2. **A start that never becomes ready looks healthy forever.** The caller asks
//!    [`Session::startup_deadline`] and reports the attempt as failed when it
//!    passes, so a hung server follows the same path as a crashed one.
//! 3. **An override-redirect window steals focus.** [`WindowEvent::Created`] carries
//!    the flag, and the policy answers with no window at all, so a menu or a
//!    tooltip can never be an activation target.
//! 4. **A window outlives its client.** [`WindowEvent::Destroyed`] is the normal
//!    path for a client that exited, and the policy removes the window from the
//!    manager rather than waiting for an unmap that will never come.
//! 5. **`DISPLAY` leaks into every other child.** [`Launch`] states the display
//!    number instead of setting an environment variable; only the caller that
//!    starts a process on the X display reads it.
//! 6. **A scale below the rule is applied.** [`Launch::scale`] is a
//!    [`gnoblin_types::Scale`], which cannot be zero or below `0.5`: the range is
//!    checked before a launch exists.

use std::time::{Duration, Instant};

use gnoblin_types::{Rect, Scale, WindowId};
use gnoblin_wm::{Effect, Wm};

/// What to do with a start attempt.
#[derive(Clone, PartialEq, Debug)]
pub enum Start {
    /// Start the server with these parameters.
    Spawn(Launch),
    /// Do not start one, and say why.
    ///
    /// The reason is complete enough to be logged as it stands: an operator reads
    /// "the Xwayland binary is not on PATH" rather than an error code, and the
    /// caller is not expected to map a condition to a sentence.
    Disabled(String),
}

/// The parameters of one Xwayland start.
#[derive(Clone, PartialEq, Debug)]
pub struct Launch {
    /// The display number to use, when one is fixed.
    ///
    /// [`None`] lets the server pick the first free number and write it back to the
    /// readiness pipe. The number belongs here rather than in the environment: a
    /// `DISPLAY` set on the compositor leaks into every child it starts, including
    /// the ones that must not talk to the X server at all.
    pub display: Option<u32>,
    /// Extra command-line arguments, in order.
    pub args: Vec<String>,
    /// Extra environment for the Xwayland process alone.
    pub env: Vec<(String, String)>,
    /// Whether to listen on the abstract socket as well as the lock-named one.
    pub open_abstract_socket: bool,
    /// The scale the Xwayland client composes at.
    ///
    /// [`None`] follows the output the X11 windows are on, which is what the
    /// configuration's `0` means. A fixed scale is at least `0.5`: see
    /// [`Start`]'s failure list.
    pub scale: Option<Scale>,
}

impl Launch {
    /// A launch with the defaults the configuration section starts from.
    #[must_use]
    pub fn new() -> Self {
        Self {
            display: None,
            args: Vec::new(),
            env: Vec::new(),
            open_abstract_socket: false,
            scale: None,
        }
    }
}

impl Default for Launch {
    fn default() -> Self {
        Self::new()
    }
}

/// What the caller does after an attempt ended without a running server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Restart {
    /// Start another one after this delay.
    RetryAfter(Duration),
    /// Stop trying, and say why. The reason is logged as it stands.
    Stop(String),
}

/// One X11 window, as the X11 side sees it.
///
/// The fields are the ones the window manager classifies a window by: the class and
/// instance a launch matches, the title a rule matches, the transient parent that
/// makes it a dialog, and the override-redirect flag that makes it unmanaged. The
/// geometry is in global logical coordinates because that is the space
/// `gnoblin-wm` places and configures in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowMeta<'a> {
    /// The X11 window id.
    pub x11: u32,
    /// The window's class, as the WM_CLASS property spells it.
    pub class: &'a str,
    /// The window's instance name.
    pub instance: &'a str,
    /// The window's title.
    pub title: &'a str,
    /// The window this one is transient for, when it has a parent.
    pub transient_for: Option<u32>,
    /// Whether the window asked to be unmanaged.
    pub override_redirect: bool,
    /// The window's rectangle, in global logical coordinates.
    pub geometry: Rect,
}

/// Something that happened to an X11 window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WindowEvent<'a> {
    /// The X server created a window. It is not mapped yet.
    Created(WindowMeta<'a>),
    /// The client asked for its window to become visible.
    MapRequested(WindowMeta<'a>),
    /// A window became mapped.
    Mapped(u32),
    /// A window was unmapped.
    Unmapped(u32),
    /// A window was destroyed, which is also how a killed client's windows go.
    Destroyed(u32),
    /// A window asked for the given rectangle. A field is `None` when the client did
    /// not ask to change it.
    ConfigureRequested {
        /// The X11 window id.
        x11: u32,
        /// The rectangle the client asked for, with the fields it changed.
        geometry: Rect,
        /// Whether the client asked to change its position.
        position: bool,
        /// Whether the client asked to change its size.
        size: bool,
    },
    /// The window's rectangle changed without the manager asking.
    Configured {
        /// The X11 window id.
        x11: u32,
        /// The rectangle the server now uses.
        geometry: Rect,
    },
    /// The window's title changed.
    TitleChanged {
        /// The X11 window id.
        x11: u32,
        /// The new title.
        title: &'a str,
    },
    /// The window's class or instance name changed.
    ClassChanged(WindowMeta<'a>),
    /// The window asked to be active, for example when it was clicked in an X11
    /// application that has no X11 window manager of its own.
    ActivationRequested(u32),
}

/// An X11 request the compositor has to send, and the policy cannot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// Tell the server the window is mapped, or no longer is.
    SetMapped {
        /// The managed window.
        window: WindowId,
        /// Whether the window should be visible.
        mapped: bool,
    },
    /// Ask the client to close its window.
    ///
    /// The request goes through the window's `WM_DELETE_WINDOW` protocol, so a
    /// client that has unsaved work can refuse it; a client that does not support
    /// the protocol is not destroyed behind its back.
    Close {
        /// The managed window.
        window: WindowId,
    },
}

/// What the compositor does with one X11 event.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct EventOutcome {
    /// The window the event is about, when the policy decided it is managed.
    ///
    /// `None` means the window is not managed at all: an override-redirect window,
    /// or an event about a window the policy never took in. The compositor keeps no
    /// window entry for it.
    pub window: Option<WindowId>,
    /// The window manager effects to apply, in order.
    pub effects: Vec<Effect>,
    /// The X11 requests to send, in order.
    pub commands: Vec<Command>,
}

impl EventOutcome {
    /// Nothing happened.
    #[must_use]
    pub fn nothing() -> Self {
        Self::default()
    }

    /// An outcome that keeps the window it is about.
    #[must_use]
    pub fn of(window: Option<WindowId>) -> Self {
        Self {
            window,
            ..Self::default()
        }
    }
}

/// The X11 window policy: one X11 window in, window manager events and X11
/// requests out.
pub trait Windows: Send {
    /// Takes one X11 event and answers what the compositor should do about it.
    fn handle(&mut self, event: WindowEvent<'_>, wm: &mut Wm) -> EventOutcome;

    /// Asks the client to close a window.
    ///
    /// This is the shell's close request, not a protocol event: the compositor calls
    /// it when a person closes the window and the answer is the
    /// [`Command::Close`] to send.
    fn close(&mut self, window: WindowId) -> EventOutcome;

    /// Whether X11 clients may take keyboard grabs at all.
    ///
    /// The configuration's `xwayland.allow-grabs`; a grab is what a menu, a combo
    /// box or a modal dialog takes so that it hears every key.
    fn allow_grabs(&self) -> bool;

    /// Whether this client may take a grab, by the configuration's rules.
    ///
    /// Only consulted when [`Windows::allow_grabs`] is true. An empty rule list
    /// allows every client, which is what the configuration's default means.
    fn grab_allowed(&self, class: &str, instance: &str) -> bool;

    /// The managed identity of an X11 window, when the manager holds it.
    ///
    /// `mutter_x11_interop` is the caller that needs it: a Wayland dialog names its
    /// X11 parent by the *X11* window id, and only this table knows which of the
    /// session's windows that id is. `None` means the window was never managed (or is
    /// already gone), which is the same answer to a dialog that wanted a parent: there
    /// is nothing to attach to.
    fn managed_window(&self, xwindow: u32) -> Option<WindowId>;
}

/// The supervision policy for the Xwayland process.
///
/// The caller owns the process, the event loop and the clock; this trait owns the
/// decisions. Every method is a statement about one attempt, and the caller only has
/// to remember the two answers it cannot derive: when it started the attempt and
/// whether it became ready.
pub trait Session: Send {
    /// Asks what to start, at `now`.
    ///
    /// Called once per attempt, after the previous [`Restart::RetryAfter`] delay has
    /// passed. The answer is [`Start::Disabled`] when no attempt can succeed: the
    /// binary is missing, the configuration refuses it, or the attempts are spent.
    fn start(&mut self, now: Instant) -> Start;

    /// Records that the instance became ready on `display`.
    fn ready(&mut self, display: u32, now: Instant);

    /// Records that the attempt failed: the process died, or it did not become
    /// ready within [`Session::startup_deadline`].
    ///
    /// Called at most once per attempt. The answer says when to try again, or that
    /// the session gives up.
    fn failed(&mut self, now: Instant) -> Restart;

    /// How long a start may take before it counts as a failure.
    fn startup_deadline(&self) -> Duration;

    /// The X settings to publish on the X connection, in name order.
    ///
    /// Empty when no XSETTINGS owner is attached. The caller publishes them once the
    /// X11 window manager is up, and reports the answer with
    /// [`Session::settings_published`], so an owner that is not the compositor's
    /// knows whether its settings reached the clients.
    fn settings(&mut self) -> Vec<(String, Setting)> {
        Vec::new()
    }

    /// Records what publishing the settings did.
    fn settings_published(&mut self, result: Result<(), String>) {
        let _ = result;
    }
}

/// One XSETTINGS value.
///
/// The three kinds the specification defines for a desktop's own settings: an integer
/// for a timeout, text for a theme name and a colour for a palette entry. The
/// serialised form is the compositor's business, because only it owns the connection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Setting {
    /// A signed integer setting, such as an animation duration.
    Integer(i32),
    /// A text setting, such as a theme name.
    Text(String),
    /// A colour, as red, green, blue and alpha bytes.
    Colour([u8; 4]),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are the neutral launch: no fixed display, no arguments, no
    /// scale override.
    #[test]
    fn a_default_launch_fixes_nothing() {
        let launch = Launch::default();
        assert_eq!(launch.display, None);
        assert!(launch.args.is_empty());
        assert!(launch.env.is_empty());
        assert_eq!(launch.scale, None);
        assert!(!launch.open_abstract_socket);
    }

    /// An outcome with no window is the shape a caller short-circuits on, so it
    /// carries no effects and no commands either.
    #[test]
    fn an_outcome_with_no_window_asks_for_nothing() {
        let outcome = EventOutcome::nothing();
        assert_eq!(outcome.window, None);
        assert!(outcome.effects.is_empty());
        assert!(outcome.commands.is_empty());
    }
}
