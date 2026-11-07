//! The X11 window table: one X11 window in, `gnoblin-wm` events and X11 requests out.
//!
//! This is the window management a floating desktop needs: X11 clients position
//! themselves and expect their position honoured, their windows are managed when they
//! map rather than when they are created, and an override-redirect window (a menu, a
//! tooltip, a drag icon) is never a window at all. The table below is the whole
//! policy, and it is pure: no X connection, no Smithay type, no clock. The compositor
//! asks it what an event means and performs the answer.
//!
//! # Failure modes first
//!
//! Each entry names the test in this module that covers it.
//!
//! 1. **A menu steals focus.** An override-redirect window has no managed identity at
//!    all, so a map request is answered with nothing and an activation request for one
//!    is refused: `an_override_redirect_popup_is_never_managed`.
//! 2. **A window outlives its client.** A destroyed window leaves the manager whether
//!    or not the client sent an unmap first, so a killed client cannot leave a
//!    focusable ghost: `a_destroyed_window_leaves_the_manager`.
//! 3. **A late event resurrects a closed window.** Every arm ignores an id the table
//!    does not hold, so a configure or title event that arrives after the destroy does
//!    not open a window: `an_event_for_an_unknown_window_changes_nothing`.
//! 4. **A client resize is refused.** A configure request is fed back through the
//!    window manager as a move/resize, so the manager's constraint answers it and the
//!    client hears the geometry it will actually get:
//!    `a_configure_request_goes_through_the_manager`.
//! 5. **A dialog is placed away from its parent.** A transient window names its parent
//!    and the window manager keeps the two together: `a_transient_window_follows_its_parent`.
//! 6. **Two windows share an identity.** The X11 window id is the identity, mapped into
//!    a band the Wayland counter cannot reach: `an_x11_identity_is_in_its_own_band`.

use std::collections::BTreeMap;

use gnoblin_layers::{Band, LayerProvider, LayerScope, LayerSpec, LocalName};
use gnoblin_types::{Rect, WindowId};
use gnoblin_wm::{ActivationDecision, Event, WindowKind, WindowProtocol, WindowSpec, Wm};

use gnoblin_backend::x11::{EventOutcome, WindowEvent, WindowMeta, Windows, X11Request};

/// The bit that separates an X11 window identity from a Wayland one.
///
/// The Wayland server numbers its windows from zero in a `u32`, so setting the top bit
/// of the token keeps the two tables apart for as long as the session can run. Doing it
/// here rather than sharing one counter is what lets the X11 policy decide identities
/// without talking to the Wayland engine at all.
const X11_BAND: u64 = 1 << 63;

/// The managed identity of an X11 window.
#[must_use]
pub fn window_id(x11: u32) -> WindowId {
    WindowId::new(X11_BAND | u64::from(x11))
}

/// One X11 window the table knows about.
#[derive(Clone, PartialEq, Eq, Debug)]
struct KnownWindow {
    /// The class the `WM_CLASS` property spells.
    class: String,
    /// The instance name.
    instance: String,
    /// The current title.
    title: String,
    /// The window this one is transient for.
    transient_for: Option<u32>,
    /// Whether the window asked to be unmanaged.
    override_redirect: bool,
    /// The window's rectangle as the server has it.
    geometry: Rect,
    /// The managed identity, for a window the manager took in.
    managed: Option<WindowId>,
}

impl KnownWindow {
    /// The application id the window manager matches a window by.
    ///
    /// The class is what an X11 window's desktop entry matches; the instance name is
    /// the fallback for a client that set only that half of `WM_CLASS`.
    fn app_id(&self) -> &str {
        if self.class.is_empty() {
            &self.instance
        } else {
            &self.class
        }
    }
}

/// The X11 window policy.
///
/// One table, one rule set: an X11 window is managed when it maps, and it leaves the
/// manager when it unmaps or is destroyed. An override-redirect window is recorded so
/// a later event can be answered about it, but it is never managed and never focused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Xwm {
    /// Every window the table holds, managed or not.
    windows: BTreeMap<u32, KnownWindow>,
    /// Whether X11 clients may take keyboard grabs.
    allow_grabs: bool,
    /// The classes or instances allowed to take a grab.
    grab_access_rules: Vec<String>,
}

/// Registers the `gnoblin.surface_x11:<window id>` record for a managed X11 window.
///
/// Xwayland owns the record because only this crate knows the window is an X11 one. The
/// record is scoped to the window, so `WindowClosed` ends it with the window and the close
/// path needs no extra step.
fn register_surface_record(wm: &mut Wm, window: WindowId) {
    if wm.window(window).is_none() {
        // The manager refused the window, and that refusal is already in the effects.
        return;
    }
    let Ok(local) = LocalName::keyed("surface_x11", &window.get().to_string()) else {
        debug_assert!(false, "a numeric window id always forms a valid local name");
        return;
    };
    let spec = LayerSpec::new(local, LayerScope::Window(Some(window)), Band::Windows);
    let result = wm.register_surface(&LayerProvider::Gnoblin("gnoblin-xwayland"), spec);
    debug_assert!(result.is_ok(), "surface record for {window} refused: {result:?}");
}

impl Xwm {
    /// An empty table with the granted grab rules.
    #[must_use]
    pub fn new(allow_grabs: bool, grab_access_rules: Vec<String>) -> Self {
        Self {
            windows: BTreeMap::new(),
            allow_grabs,
            grab_access_rules,
        }
    }

    /// The managed identity of an X11 window, when the manager holds it.
    #[must_use]
    pub fn managed(&self, x11: u32) -> Option<WindowId> {
        self.windows.get(&x11).and_then(|entry| entry.managed)
    }

    /// Whether the table holds the window at all.
    #[must_use]
    pub fn knows(&self, x11: u32) -> bool {
        self.windows.contains_key(&x11)
    }

    /// Records what an event says about the window it is about.
    fn remember(&mut self, meta: &WindowMeta<'_>) {
        let entry = self.windows.entry(meta.x11).or_insert_with(|| KnownWindow {
            class: String::new(),
            instance: String::new(),
            title: String::new(),
            transient_for: None,
            override_redirect: meta.override_redirect,
            geometry: meta.geometry,
            managed: None,
        });
        // The override-redirect flag never changes for a window's lifetime, and a
        // window cannot turn from managed back into an unmanaged one, so it is only
        // ever set when the entry is created.
        entry.class.clear();
        entry.class.push_str(meta.class);
        entry.instance.clear();
        entry.instance.push_str(meta.instance);
        entry.title.clear();
        entry.title.push_str(meta.title);
        entry.transient_for = meta.transient_for;
        entry.geometry = meta.geometry;
    }

    /// The window spec the manager is asked to take a window in with.
    fn spec(&self, x11: u32) -> Option<WindowSpec> {
        let entry = self.windows.get(&x11)?;
        let id = window_id(x11);
        // A transient names its parent only when the manager holds that parent; an
        // unmanaged parent (an override-redirect window, or one the manager never saw)
        // cannot be anchored to, so the window is placed on its own instead of being
        // pinned to an identity nothing will ever close.
        let spec = match entry.transient_for.and_then(|parent| self.managed(parent)) {
            Some(parent) => WindowSpec {
                kind: WindowKind::Dialog,
                transient_for: Some(parent),
                ..WindowSpec::normal(id, entry.app_id())
            },
            None => WindowSpec::normal(id, entry.app_id()),
        };
        // The window is an X11 window, so the manager records it as `surface_x11`
        // rather than a Wayland toplevel.
        Some(WindowSpec {
            protocol: WindowProtocol::X11,
            ..spec.with_title(&entry.title).with_requested(entry.geometry)
        })
    }

    /// A client asked for its window to become visible.
    fn map_requested(&mut self, meta: &WindowMeta<'_>, wm: &mut Wm) -> EventOutcome {
        self.remember(meta);
        if meta.override_redirect {
            // Unmanaged by definition: the client maps it itself and the manager never
            // grants, configures or focuses it.
            return EventOutcome::nothing();
        }
        if let Some(window) = self.managed(meta.x11) {
            // A window the manager still holds is being shown again, which is a
            // request for attention rather than a new window.
            return EventOutcome {
                window: Some(window),
                effects: wm.apply(Event::ActivationRequested {
                    window,
                    decision: ActivationDecision::Allow,
                }),
                commands: vec![X11Request::SetMapped { window, mapped: true }],
            };
        }
        let Some(spec) = self.spec(meta.x11) else {
            return EventOutcome::nothing();
        };
        let window = spec.id;
        if let Some(entry) = self.windows.get_mut(&meta.x11) {
            entry.managed = Some(window);
        }
        let mut effects = wm.apply(Event::WindowOpened(spec));
        register_surface_record(wm, window);
        // Mapping is the X11 equivalent of a toplevel's first committed buffer: the
        // window is now a candidate for focus, and whether it takes it is the window
        // manager's answer.
        effects.extend(wm.apply(Event::ActivationRequested {
            window,
            decision: ActivationDecision::Allow,
        }));
        EventOutcome {
            window: Some(window),
            effects,
            commands: vec![X11Request::SetMapped { window, mapped: true }],
        }
    }

    /// A window left the screen, or went away with its client.
    fn closed(&mut self, x11: u32, wm: &mut Wm) -> EventOutcome {
        let Some(entry) = self.windows.remove(&x11) else {
            return EventOutcome::nothing();
        };
        let Some(window) = entry.managed else {
            return EventOutcome::nothing();
        };
        EventOutcome {
            window: Some(window),
            effects: wm.apply(Event::WindowClosed { window }),
            commands: Vec::new(),
        }
    }
}

impl Windows for Xwm {
    fn managed_window(&self, xwindow: u32) -> Option<WindowId> {
        self.managed(xwindow)
    }

    fn handle(&mut self, event: WindowEvent<'_>, wm: &mut Wm) -> EventOutcome {
        match event {
            WindowEvent::Created(meta) => {
                self.remember(&meta);
                EventOutcome::nothing()
            }
            WindowEvent::MapRequested(meta) => self.map_requested(&meta, wm),
            WindowEvent::Mapped(x11) => EventOutcome::of(self.managed(x11)),
            WindowEvent::Unmapped(x11) | WindowEvent::Destroyed(x11) => self.closed(x11, wm),
            WindowEvent::ConfigureRequested {
                x11,
                geometry,
                position,
                size,
            } => {
                let Some(entry) = self.windows.get(&x11) else {
                    return EventOutcome::nothing();
                };
                let current = entry.geometry;
                let candidate = Rect::new(
                    if position { geometry.x } else { current.x },
                    if position { geometry.y } else { current.y },
                    if size { geometry.width } else { current.width },
                    if size { geometry.height } else { current.height },
                );
                let Some(window) = entry.managed else {
                    // An unmanaged window configures itself. The request is recorded so
                    // the next comparison starts from the truth, and no request is sent
                    // back: an override-redirect menu is positioned by its client.
                    if let Some(entry) = self.windows.get_mut(&x11) {
                        entry.geometry = candidate;
                    }
                    return EventOutcome::nothing();
                };
                EventOutcome {
                    window: Some(window),
                    effects: wm.apply(Event::MoveResize {
                        window,
                        geometry: candidate,
                    }),
                    commands: Vec::new(),
                }
            }
            WindowEvent::Configured { x11, geometry } => {
                let Some(entry) = self.windows.get_mut(&x11) else {
                    return EventOutcome::nothing();
                };
                entry.geometry = geometry;
                let Some(window) = entry.managed else {
                    return EventOutcome::nothing();
                };
                // A client that moved itself is honoured: the manager clamps the
                // rectangle and the client hears the answer, which is what makes an
                // X11 application's own positioning work on a floating desktop.
                EventOutcome {
                    window: Some(window),
                    effects: wm.apply(Event::MoveResize { window, geometry }),
                    commands: Vec::new(),
                }
            }
            WindowEvent::TitleChanged { x11, title } => {
                let Some(entry) = self.windows.get_mut(&x11) else {
                    return EventOutcome::nothing();
                };
                entry.title.clear();
                entry.title.push_str(title);
                let Some(window) = entry.managed else {
                    return EventOutcome::nothing();
                };
                EventOutcome {
                    window: Some(window),
                    effects: wm.apply(Event::WindowTitleChanged {
                        window,
                        title: title.to_owned(),
                    }),
                    commands: Vec::new(),
                }
            }
            WindowEvent::ClassChanged(meta) => {
                self.remember(&meta);
                let Some(window) = self.managed(meta.x11) else {
                    return EventOutcome::nothing();
                };
                let app_id = self
                    .windows
                    .get(&meta.x11)
                    .map(|entry| entry.app_id().to_owned())
                    .unwrap_or_default();
                EventOutcome {
                    window: Some(window),
                    effects: wm.apply(Event::WindowAppIdChanged { window, app_id }),
                    commands: Vec::new(),
                }
            }
            WindowEvent::ActivationRequested(x11) => {
                let Some(window) = self.managed(x11) else {
                    // The request came from a window the manager does not hold, which
                    // includes every override-redirect window: a menu cannot take focus.
                    return EventOutcome::nothing();
                };
                EventOutcome {
                    window: Some(window),
                    effects: wm.apply(Event::ActivationRequested {
                        window,
                        decision: ActivationDecision::Allow,
                    }),
                    commands: Vec::new(),
                }
            }
        }
    }

    fn close(&mut self, window: WindowId) -> EventOutcome {
        let known = self.windows.values().any(|entry| entry.managed == Some(window));
        if !known {
            return EventOutcome::nothing();
        }
        EventOutcome {
            window: Some(window),
            effects: Vec::new(),
            commands: vec![X11Request::Close { window }],
        }
    }

    fn allow_grabs(&self) -> bool {
        self.allow_grabs
    }

    fn grab_allowed(&self, class: &str, instance: &str) -> bool {
        crate::settings::grab_permitted(self.allow_grabs, &self.grab_access_rules, class, instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gnoblin_types::{OutputId, WorkspaceId};
    use gnoblin_wm::{Effect, WmOptions};

    fn session() -> Wm {
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(1),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        wm
    }

    fn meta<'a>(x11: u32, class: &'a str, title: &'a str) -> WindowMeta<'a> {
        WindowMeta {
            x11,
            class,
            instance: "instance",
            title,
            transient_for: None,
            override_redirect: false,
            geometry: Rect::new(0, 0, 300, 200),
        }
    }

    /// The rectangle the manager told a window to use, if it told it anything.
    fn configured(effects: &[Effect], window: WindowId) -> Option<Rect> {
        effects.iter().find_map(|effect| match effect {
            Effect::Configure {
                window: configured,
                geometry,
                ..
            } if *configured == window => Some(*geometry),
            _ => None,
        })
    }

    /// The two identity spaces cannot meet: every X11 token has the band bit, and the
    /// Wayland counter numbers from zero in a `u32`.
    #[test]
    fn an_x11_identity_is_in_its_own_band() {
        for x11 in [0, 1, 42, u32::MAX] {
            let id = window_id(x11);
            assert_eq!(id.get() & X11_BAND, X11_BAND);
            assert_eq!(id.get() & !X11_BAND, u64::from(x11));
            assert!(u64::from(u32::MAX) < X11_BAND);
        }
        assert_ne!(window_id(7), WindowId::new(7));
    }

    /// An override-redirect window is never managed, never activated, and never asked
    /// to map: it is a menu, and a menu that took focus would steal the keyboard from
    /// the window the person was typing in.
    #[test]
    fn an_override_redirect_popup_is_never_managed() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        let mut popup = meta(9, "Gimp", "menu");
        popup.override_redirect = true;

        assert_eq!(
            xwm.handle(WindowEvent::Created(popup.clone()), &mut wm),
            EventOutcome::nothing()
        );
        assert_eq!(
            xwm.handle(WindowEvent::MapRequested(popup.clone()), &mut wm),
            EventOutcome::nothing()
        );
        assert_eq!(
            xwm.handle(WindowEvent::ActivationRequested(9), &mut wm),
            EventOutcome::nothing()
        );

        assert!(wm.windows().next().is_none(), "no window may enter the manager");
        assert_eq!(xwm.managed(9), None);
        assert!(xwm.knows(9), "the table still answers about the window");
    }

    /// Mapping is what takes a window into management, and the manager's answer is the
    /// geometry the client is told to adopt.
    #[test]
    fn a_mapped_window_is_managed_with_its_title_and_class() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        let window = window_id(3);

        let outcome = xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "kieran@fsociety")), &mut wm);

        assert_eq!(outcome.window, Some(window));
        let managed = wm.window(window).expect("the window is managed");
        assert_eq!(managed.app_id, "XTerm");
        assert_eq!(managed.title, "kieran@fsociety");
        let geometry = configured(&outcome.effects, window).expect("the manager configures the window");
        assert_eq!(geometry, managed.geometry);
        assert_eq!(outcome.commands, vec![X11Request::SetMapped { window, mapped: true }]);
    }

    /// A managed X11 window is a surface record, so layer tooling sees it beside the Wayland
    /// toplevels. Its record is scoped to the window, so unmapping it ends the record too.
    #[test]
    fn a_mapped_x11_window_registers_a_surface_record_that_unmapping_ends() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        let window = window_id(3);
        let name = format!("gnoblin.surface_x11:{}", window.get());

        xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "kieran@fsociety")), &mut wm);

        let record = wm
            .layers()
            .records()
            .find(|record| record.name.to_string() == name)
            .expect("the mapped window has a surface record");
        assert_eq!(record.spec.band, Band::Windows);
        assert_eq!(record.spec.scope, LayerScope::Window(Some(window)));

        xwm.handle(WindowEvent::Unmapped(3), &mut wm);
        assert!(wm.layers().records().all(|record| record.name.to_string() != name));
    }

    /// A client that exits sends no unmap, so a destroy leaves the manager the same way
    /// an unmap does: no window, no focus, no ghost.
    #[test]
    fn a_destroyed_window_leaves_the_manager() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        xwm.handle(WindowEvent::MapRequested(meta(4, "XTerm", "t")), &mut wm);
        let window = window_id(4);
        assert!(wm.window(window).is_some());

        let outcome = xwm.handle(WindowEvent::Destroyed(4), &mut wm);

        assert_eq!(outcome.window, Some(window));
        assert!(wm.window(window).is_none(), "the manager holds no window");
        assert!(!xwm.knows(4), "the table forgets the window");
    }

    /// An event about a window the table never saw is not an open: a stale title or
    /// configure from a closing client must not create anything.
    #[test]
    fn an_event_for_an_unknown_window_changes_nothing() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();

        for event in [
            WindowEvent::Unmapped(77),
            WindowEvent::Destroyed(77),
            WindowEvent::Mapped(77),
            WindowEvent::ActivationRequested(77),
            WindowEvent::TitleChanged { x11: 77, title: "late" },
            WindowEvent::Configured {
                x11: 77,
                geometry: Rect::new(1, 1, 10, 10),
            },
            WindowEvent::ConfigureRequested {
                x11: 77,
                geometry: Rect::new(1, 1, 10, 10),
                position: true,
                size: true,
            },
        ] {
            assert_eq!(xwm.handle(event, &mut wm), EventOutcome::nothing());
        }
        assert!(wm.windows().next().is_none());
    }

    /// A configure request reaches the window manager, so the answer is the geometry
    /// the work area allows rather than the geometry the client asked for.
    #[test]
    fn a_configure_request_goes_through_the_manager() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        xwm.handle(WindowEvent::MapRequested(meta(5, "XTerm", "t")), &mut wm);
        let window = window_id(5);
        let before = wm.window(window).expect("managed").geometry;

        let outcome = xwm.handle(
            WindowEvent::ConfigureRequested {
                x11: 5,
                // Far outside the output, so the manager's clamp is what decides.
                geometry: Rect::new(9_000, 9_000, 600, 400),
                position: true,
                size: true,
            },
            &mut wm,
        );

        let managed = wm.window(window).expect("still managed");
        assert_ne!(managed.geometry, before, "the request moved the window");
        assert_eq!(managed.geometry.size().width, 600);
        assert_eq!(managed.geometry.size().height, 400);
        assert_eq!(
            configured(&outcome.effects, window),
            Some(managed.geometry),
            "the client hears the geometry the manager allowed"
        );
    }

    /// A dialog follows the window it is transient for instead of being placed on its
    /// own, which is what keeps a file chooser over the application that opened it.
    #[test]
    fn a_transient_window_follows_its_parent() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        xwm.handle(WindowEvent::MapRequested(meta(6, "App", "main")), &mut wm);
        let mut dialog = meta(7, "App", "open file");
        dialog.transient_for = Some(6);

        let outcome = xwm.handle(WindowEvent::MapRequested(dialog), &mut wm);

        let child = wm.window(window_id(7)).expect("the dialog is managed");
        assert_eq!(outcome.window, Some(window_id(7)));
        assert_eq!(child.transient_for, Some(window_id(6)));
        assert_eq!(child.kind, WindowKind::Dialog);
        let parent = wm.window(window_id(6)).expect("the parent is managed");
        assert_eq!(child.workspace, parent.workspace);
        assert_eq!(child.output, parent.output);
    }

    /// Closing a managed window is a `WM_DELETE_WINDOW` request, so a client with
    /// unsaved work can refuse it; an unmanaged window has nothing to close.
    #[test]
    fn a_close_request_is_a_delete_window() {
        let mut xwm = Xwm::new(false, Vec::new());
        let mut wm = session();
        xwm.handle(WindowEvent::MapRequested(meta(8, "App", "main")), &mut wm);

        assert_eq!(
            xwm.close(window_id(8)),
            EventOutcome {
                window: Some(window_id(8)),
                effects: Vec::new(),
                commands: vec![X11Request::Close { window: window_id(8) }],
            }
        );
        assert_eq!(xwm.close(window_id(99)), EventOutcome::nothing());
    }

    /// Grabs follow the configuration: off by default, and a rule list only narrows.
    #[test]
    fn grabs_follow_the_configuration() {
        let off = Xwm::new(false, Vec::new());
        assert!(!off.allow_grabs());
        assert!(!off.grab_allowed("XTerm", "xterm"));

        let all = Xwm::new(true, Vec::new());
        assert!(all.grab_allowed("XTerm", "xterm"));

        let some = Xwm::new(true, vec!["XTerm".to_owned()]);
        assert!(some.grab_allowed("XTerm", "xterm"));
        assert!(!some.grab_allowed("Gimp", "gimp"));
    }
}
