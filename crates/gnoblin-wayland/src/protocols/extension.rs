//! The redirect adapters' seam: how a protocol this crate does not own gets served.
//!
//! Two of the plan's compat crates speak the Wayland transport rather than D-Bus
//! (`design/rust-compositor.md`, "Dependency rules": "the `gnoblin-wayland`
//! extension trait for the Wayland redirects"). The adapter owns the protocol —
//! its XML, its generated bindings, its dispatch and its mapping — and this crate
//! owns the compositor: the display, the globals, the activation authority and the
//! window manager. The seam is the two traits below, and it points one way, so a
//! compat crate can name this one and this one names no adapter.
//!
//! # Why an extension trait and not a feature
//!
//! A `cfg` feature would put the adapter's code in this crate's build, so a build
//! without the feature would have to stub the protocol and a build with it could
//! not be tested without the adapter. An [`Extension`] is a value the session is
//! given: nothing here knows which adapters exist, and the composition root
//! decides. Registration is one call per extension, before the loop runs, which is
//! the only moment a global can be created without a client seeing a race.
//!
//! # The host traits
//!
//! An adapter's dispatch handler holds `&mut State` but has no business reaching
//! into the window manager, the activation authority or the Xwayland window table
//! itself: those are this crate's policy. [`Shell1Host`] and [`X11InteropHost`] are
//! the operations a redirect needs, named in this crate's vocabulary, and
//! implemented here. The adapter's job stays the mapping — a gesture becomes the
//! configured action, an app id becomes a window — and the effect stays the
//! compositor's.
//!
//! # Failure modes first
//!
//! 1. **A dead surface's request still acts.** Every host call resolves the surface
//!    first and answers [`Surface::Gone`] for one the session no longer manages, so
//!    a request that arrives after its window closed changes nothing: the adapter's
//!    tests cover it (`a_dead_surfaces_request_changes_nothing`).
//! 2. **An extension is registered twice.** [`ExtensionError::Duplicate`] is
//!    returned for a second registration of the same name, because two globals with
//!    one name is a session where half the clients see one handler and half the
//!    other: `an_extension_cannot_be_registered_twice`.
//! 3. **A present or focus request grants focus by app id.** Both go through the
//!    activation authority and only a trusted context is allowed to move focus, so
//!    naming an application is never authority: [`Shell1Host::present`].

use std::collections::BTreeSet;
use std::fmt;

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use gnoblin_authority::Denial;
use gnoblin_config::{TitlebarAction, WindowManagement};
use gnoblin_types::WindowId;
use gnoblin_wm::Event;

use crate::State;

use crate::log;

/// One protocol a session serves through an adapter.
///
/// Implemented by the adapter's crate, next to the protocol it serves. The session
/// keeps no handle on it after [`Extension::register`], because everything the
/// handler needs is in the state the display dispatches to.
pub trait Extension: Send + Sync + std::fmt::Debug {
    /// The adapter's short name, used in logs and in a duplicate's error.
    fn name(&self) -> &'static str;

    /// Creates this extension's globals on the session's display.
    ///
    /// Called once, before the loop runs, so no client can observe a session with
    /// half its globals. Returning is the assertion that every global the adapter
    /// serves is live; nothing here holds the global's identity, because a global
    /// the session never removes needs no handle, and an adapter that does remove
    /// one keeps its handle itself.
    ///
    /// # Errors
    ///
    /// [`ExtensionError`] when the globals cannot be created; the session then stops
    /// rather than running with a protocol it advertised and cannot serve.
    fn register(&self, display: &DisplayHandle) -> Result<(), ExtensionError>;
}

/// Why a session could not serve an extension.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExtensionError {
    /// The global for this name already exists.
    Duplicate {
        /// The adapter's name.
        name: &'static str,
    },
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { name } => write!(
                formatter,
                "the `{name}` extension is registered twice, so one of the two would serve nothing"
            ),
        }
    }
}

impl std::error::Error for ExtensionError {}

/// The extensions of one session, remembered so a duplicate is refused.
#[derive(Default)]
pub(crate) struct Extensions {
    names: BTreeSet<&'static str>,
}

impl Extensions {
    /// Registers every extension, in the order the session was given them.
    pub(crate) fn register(
        &mut self,
        extensions: &[std::sync::Arc<dyn Extension>],
        display: &DisplayHandle,
    ) -> Result<(), ExtensionError> {
        for extension in extensions {
            let name = extension.name();
            if !self.names.insert(name) {
                return Err(ExtensionError::Duplicate { name });
            }
            extension.register(display)?;
        }
        Ok(())
    }
}

/// What a redirect's request can learn about the surface it names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    /// A managed window, with the session's identity for it.
    Window(WindowId),
    /// The surface is not (or no longer) a managed window: a request that arrives
    /// after the window closed buys nothing.
    Gone,
}

/// The three gestures `gtk_surface1` hands to the compositor.
///
/// `window_management` declares one action per gesture, and this is the triple as
/// the configuration states it, so an adapter asks once per request instead of
/// reading three keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TitlebarActions {
    /// `action_double_click_titlebar`.
    pub double_click: TitlebarAction,
    /// `action_middle_click_titlebar`.
    pub middle_click: TitlebarAction,
    /// `action_right_click_titlebar`.
    pub right_click: TitlebarAction,
}

impl TitlebarActions {
    /// The actions a configuration section declares.
    #[must_use]
    pub fn of(window_management: &WindowManagement) -> Self {
        let defaults = Self::default();
        Self {
            double_click: window_management
                .action_double_click_titlebar
                .unwrap_or(defaults.double_click),
            middle_click: window_management
                .action_middle_click_titlebar
                .unwrap_or(defaults.middle_click),
            right_click: window_management
                .action_right_click_titlebar
                .unwrap_or(defaults.right_click),
        }
    }
}

impl Default for TitlebarActions {
    /// The configuration's own defaults, which is what a session that omits the
    /// section gets.
    fn default() -> Self {
        Self {
            double_click: TitlebarAction::ToggleMaximize,
            middle_click: TitlebarAction::Lower,
            right_click: TitlebarAction::Menu,
        }
    }
}

/// What performing a titlebar action did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Titlebar {
    /// The action reached the window manager.
    Performed,
    /// The configured action is one `gnoblin-wm` has no event for yet, so nothing
    /// happened and the caller logs `reason` rather than pretending otherwise.
    Unavailable(&'static str),
}

/// What an activation request did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activation {
    /// The window is focused, or already was.
    Focused,
    /// The request was refused and the refusal is published as
    /// `activation-denied` with this reason.
    Refused(&'static str),
}

/// What a `gtk_shell1` handler asks of the compositor.
///
/// Every method resolves the surface first, so an adapter never has to guess
/// whether a window still exists. The authority is this crate's: `present` and
/// `request_focus` are the only ways to move focus, they go through
/// `gnoblin-authority`, and an app id is never accepted as one of them.
// No `Send` bound: the compositor state holds an `Rc` from Smithay's input protocols,
/// and a host is only ever used on the loop's own thread, inside a dispatch.
pub trait Shell1Host {
    /// The managed window a surface belongs to.
    fn surface(&self, surface: &WlSurface) -> Surface;

    /// The configured titlebar actions, for a gesture to be resolved against.
    fn titlebar_actions(&self) -> TitlebarActions;

    /// Applies the application id a client declared for its window.
    ///
    /// The id is what window rules match, so a client that sets it late updates the
    /// same window rather than opening a second one. A surface that is no longer a
    /// window is [`Surface::Gone`] and nothing changes.
    fn set_app_id(&mut self, surface: &WlSurface, app_id: &str) -> Surface;

    /// Performs one configured titlebar action on a window.
    fn titlebar(&mut self, window: WindowId, action: TitlebarAction) -> Titlebar;

    /// Raises a window its own client asked to present.
    ///
    /// The only trusted context `gtk_shell1` can offer is "the user is already
    /// typing into this window": an unfocused window's present is refused as
    /// `no-authority`, which is the plan's rule that an app id never grants focus.
    fn present(&mut self, window: WindowId, time: u32) -> Activation;

    /// Answers a client's focus request with the same rule as [`Shell1Host::present`].
    fn request_focus(&mut self, window: WindowId, startup_id: Option<&str>) -> Activation;

    /// Rings the bell, publishing the `bell` signal for the session.
    fn bell(&mut self, window: Option<WindowId>);

    /// Records a launch the client announced.
    fn launch(&mut self, startup_id: &str);
}

impl Shell1Host for State {
    fn surface(&self, surface: &WlSurface) -> Surface {
        match self.window_of(surface) {
            Some(window) => Surface::Window(window),
            None => Surface::Gone,
        }
    }

    fn titlebar_actions(&self) -> TitlebarActions {
        self.titlebar_actions
    }

    fn set_app_id(&mut self, surface: &WlSurface, app_id: &str) -> Surface {
        let Some(window) = self.window_of(surface) else {
            return Surface::Gone;
        };
        let effects = self.wm.apply(Event::WindowAppIdChanged {
            window,
            app_id: app_id.to_owned(),
        });
        self.apply(effects);
        self.publish();
        Surface::Window(window)
    }

    fn titlebar(&mut self, window: WindowId, action: TitlebarAction) -> Titlebar {
        if self.wm.window(window).is_none() {
            return Titlebar::Unavailable("the window is not managed");
        }
        let event = match action {
            TitlebarAction::ToggleMaximize => Event::ToggleMaximise { window },
            TitlebarAction::Minimize => Event::Minimise { window },
            TitlebarAction::Lower => Event::Lower { window },
            TitlebarAction::None => return Titlebar::Performed,
            // Three configured actions have no event in `gnoblin-wm`: the
            // horizontal and vertical halves of a maximise, and the window menu,
            // which is a shell surface this session does not have yet.
            TitlebarAction::ToggleMaximizeHorizontally => {
                return Titlebar::Unavailable("gnoblin-wm has no horizontal-maximise event");
            }
            TitlebarAction::ToggleMaximizeVertically => {
                return Titlebar::Unavailable("gnoblin-wm has no vertical-maximise event");
            }
            TitlebarAction::Menu => return Titlebar::Unavailable("the window menu is a shell surface"),
        };
        let effects = self.wm.apply(event);
        self.apply(effects);
        self.publish();
        Titlebar::Performed
    }

    fn present(&mut self, window: WindowId, time: u32) -> Activation {
        // The protocol's timestamp is a client's own clock; the session's turn is
        // what the authority reads, so the value is deliberately unused.
        let _ = time;
        if self.wm.window(window).is_none() {
            return Activation::Refused("not-a-window");
        }
        // A trusted context is the one `gtk_shell1` can offer: the user is
        // interacting with this window already. Anything else is a window asking to
        // raise itself, which is exactly what an app id must not buy.
        if self.wm.focused() == Some(window) {
            let effects = self.wm.apply(Event::Raise { window });
            self.apply(effects);
            self.publish();
            return Activation::Focused;
        }
        let reason = Denial::NoAuthority.as_str();
        self.refuse_activation(Some(window), reason);
        Activation::Refused(reason)
    }

    fn request_focus(&mut self, window: WindowId, startup_id: Option<&str>) -> Activation {
        // There is no startup-notification sequence table yet, so the id is
        // recorded for the log and attributes nothing. The focus rule is present's.
        if let Some(startup_id) = startup_id {
            log::debug!(area = "gtk-shell", startup_id, %window, "a focus request named a startup id");
        }
        <Self as Shell1Host>::present(self, window, 0)
    }

    fn bell(&mut self, window: Option<WindowId>) {
        self.ring_bell(window);
    }

    fn launch(&mut self, startup_id: &str) {
        // Mutter adds a startup sequence here; this session has no sequence table,
        // so the launch is recorded and logged and nothing is completed by it.
        log::debug!(area = "gtk-shell", startup_id, "a client announced a launch");
    }
}

/// What `mutter_x11_interop` needs from the compositor.
///
/// Both answers are read-only: the relation itself is the adapter's to record,
/// because applying a transient to a window that is already mapped needs a
/// window-manager event `gnoblin-wm` does not have yet. What only the compositor
/// knows is which windows exist on either side of the X11 boundary.
pub trait X11InteropHost {
    /// The managed Wayland window a surface belongs to.
    fn surface(&self, surface: &WlSurface) -> Surface;

    /// The managed window an X11 window id names, when Xwayland's window table has
    /// one.
    ///
    /// `None` covers both "no such X11 window" and "no Xwayland session", which are
    /// the same answer to a dialog that wanted an X11 parent: nothing to attach to.
    fn window_for_x11(&self, xwindow: u32) -> Option<WindowId>;
}

impl X11InteropHost for State {
    fn surface(&self, surface: &WlSurface) -> Surface {
        match self.window_of(surface) {
            Some(window) => Surface::Window(window),
            None => Surface::Gone,
        }
    }

    fn window_for_x11(&self, xwindow: u32) -> Option<WindowId> {
        self.x11_window(xwindow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use smithay::reexports::wayland_server::Display;

    /// An extension that counts how often it was registered and creates no global,
    /// because the registry is what this test is about.
    #[derive(Debug, Default)]
    struct Counting {
        registrations: Arc<AtomicU32>,
    }

    impl Extension for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }

        fn register(&self, _display: &DisplayHandle) -> Result<(), ExtensionError> {
            self.registrations.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    /// Two extensions of one name is one session where half the clients reach a
    /// different handler, so the second registration is refused.
    #[test]
    fn an_extension_cannot_be_registered_twice() {
        let display: Display<()> = Display::new().expect("a display");
        let handle = display.handle();
        let registrations = Arc::new(AtomicU32::default());
        let extension: Arc<dyn Extension> = Arc::new(Counting {
            registrations: Arc::clone(&registrations),
        });
        let mut extensions = Extensions::default();
        assert_eq!(extensions.register(std::slice::from_ref(&extension), &handle), Ok(()));
        assert_eq!(
            extensions.register(std::slice::from_ref(&extension), &handle),
            Err(ExtensionError::Duplicate { name: "counting" }),
            "the second registration must be refused by the registry"
        );
        assert_eq!(registrations.load(Ordering::Relaxed), 1);
    }

    /// A session with one headless output and no Xwayland: the smallest real state, so
    /// the host methods below run against the compositor rather than a stand-in.
    fn state() -> crate::State {
        use smithay::reexports::wayland_server::Display;

        use gnoblin_backend::headless::Headless;
        use gnoblin_config::Protocols;
        use gnoblin_wm::WmOptions;

        let backend = Headless::new();
        let outputs = gnoblin_backend::Backend::<crate::State>::outputs(&backend).to_vec();
        let display: Display<crate::State> = Display::new().expect("a display");
        crate::State::new(
            display,
            &outputs,
            "seat0",
            WmOptions::default(),
            1,
            None,
            None,
            &Protocols::default(),
            TitlebarActions::default(),
            crate::server::CursorSettings::default(),
            crate::server::VisualBellSettings::default(),
            &[],
        )
        .expect("a state for the default output")
    }

    /// A bell is an event: it is counted once per request and published, so a reader
    /// sees that it happened. Nothing here rings it again, which is the loop a
    /// compositor's own bell handler must not have.
    #[test]
    fn a_bell_is_counted_once_per_request() {
        let mut state = state();
        assert_eq!(state.observation().lock().expect("the snapshot").bells(), 0);
        Shell1Host::bell(&mut state, None);
        let snapshot = state.observation().lock().expect("the snapshot").clone();
        assert_eq!(snapshot.bells(), 1);
        Shell1Host::bell(&mut state, Some(WindowId::new(1)));
        let snapshot = state.observation().lock().expect("the snapshot").clone();
        assert_eq!(snapshot.bells(), 2, "two requests, two bells");
    }

    /// An activation request for a window the session never had focuses nothing and says
    /// so: a request that arrives after a window closed is the ordinary case, and it must
    /// not invent a window or move focus to whatever was focused before.
    #[test]
    fn a_present_for_an_unmanaged_window_focuses_nothing() {
        let mut state = state();
        assert_eq!(
            Shell1Host::present(&mut state, WindowId::new(9_999), 0),
            Activation::Refused("not-a-window")
        );
        assert_eq!(state.observation().lock().expect("the snapshot").focus(), None);
    }

    /// A configured action the window manager has no event for is reported as absent
    /// rather than performed as something else.
    #[test]
    fn an_action_the_engine_lacks_is_reported() {
        let mut state = state();
        for action in [
            TitlebarAction::Menu,
            TitlebarAction::ToggleMaximizeHorizontally,
            TitlebarAction::ToggleMaximizeVertically,
        ] {
            assert!(
                matches!(
                    Shell1Host::titlebar(&mut state, WindowId::new(1), action),
                    Titlebar::Unavailable(_)
                ),
                "{action} is not a window-manager event yet"
            );
        }
        // And a window this session never had is not a window to act on either.
        assert_eq!(
            Shell1Host::titlebar(&mut state, WindowId::new(9_999), TitlebarAction::ToggleMaximize),
            Titlebar::Unavailable("the window is not managed")
        );
    }

    /// The configuration's three action keys are what an adapter resolves a gesture
    /// against, so the default triple is the configuration's own.
    #[test]
    fn the_default_actions_are_the_configurations() {
        let actions = TitlebarActions::default();
        assert_eq!(actions.double_click, TitlebarAction::ToggleMaximize);
        assert_eq!(actions.middle_click, TitlebarAction::Lower);
        assert_eq!(actions.right_click, TitlebarAction::Menu);
    }
}
