//! The X11 window table: one X11 window in, node changes and X11 requests out.
//!
//! This is the window management a floating desktop needs: X11 clients position
//! themselves and expect their position honoured, their windows are managed when they
//! map rather than when they are created, and an override-redirect window (a menu, a
//! tooltip, a drag icon) is never a window at all. The table below is the whole
//! policy, and it is pure: no X connection, no Smithay type, no clock. The compositor
//! asks it what an event means and performs the answer, and the answer is the same
//! facts a Wayland toplevel's lifecycle carries (open, title, application id,
//! geometry, activation, close), so an X11 window becomes the same window node.
//!
//! # Failure modes first
//!
//! Each entry names the test in this module that covers it.
//!
//! 1. **A menu steals focus.** An override-redirect window has no managed identity at
//!    all, so a map request is answered with nothing and an activation request for one
//!    is refused: `an_override_redirect_popup_is_never_managed`.
//! 2. **A window outlives its client.** A destroyed window leaves the table whether
//!    or not the client sent an unmap first, so a killed client cannot leave a
//!    focusable ghost: `a_destroyed_window_leaves_the_table`.
//! 3. **A late event resurrects a closed window.** Every arm ignores an id the table
//!    does not hold, so a configure or title event that arrives after the destroy does
//!    not open a window: `an_event_for_an_unknown_window_changes_nothing`.
//! 4. **A client resize is dropped.** A configure request is answered with the
//!    rectangle it asked for, the fields it left alone filled in from the last known
//!    one, so the compositor's placement answers it and the client hears the geometry
//!    it will actually get: `a_configure_request_keeps_the_fields_the_client_left_alone`.
//! 5. **A dialog is placed away from its parent.** A transient window names its parent
//!    when the table holds that parent, and the compositor keeps the two together:
//!    `a_transient_window_names_its_managed_parent`.
//! 6. **Two windows share an identity.** The X11 window id is the identity, mapped into
//!    a band the Wayland counter cannot reach: `an_x11_identity_is_in_its_own_band`.
//! 7. **A rename never reaches the node.** A title or class change on a managed window
//!    is a change to its node, and one on an unmanaged window is nothing:
//!    `a_title_and_a_class_change_reach_the_node`.

use std::collections::BTreeMap;

use gnoblin_types::{Rect, WindowId};

use gnoblin_backend::x11::{EventOutcome, NodeChange, Opened, WindowEvent, WindowMeta, Windows, X11Request};

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
    /// The managed identity, for a window the table took in.
    managed: Option<WindowId>,
}

impl KnownWindow {
    /// The application id a window is matched by.
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
/// table when it unmaps or is destroyed. An override-redirect window is recorded so
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

    /// The managed identity of an X11 window, when the table holds it.
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

    /// What a window is taken in with.
    fn opened(&self, x11: u32) -> Option<Opened> {
        let entry = self.windows.get(&x11)?;
        Some(Opened {
            window: window_id(x11),
            app_id: entry.app_id().to_owned(),
            title: entry.title.clone(),
            // A transient names its parent only when the table holds that parent; an
            // unmanaged parent (an override-redirect window, or one the table never saw)
            // cannot be anchored to, so the window is placed on its own instead of being
            // pinned to an identity nothing will ever close.
            transient_for: entry.transient_for.and_then(|parent| self.managed(parent)),
            requested: entry.geometry,
        })
    }

    /// A client asked for its window to become visible.
    fn map_requested(&mut self, meta: &WindowMeta<'_>) -> EventOutcome {
        self.remember(meta);
        if meta.override_redirect {
            // Unmanaged by definition: the client maps it itself and the compositor
            // never grants, configures or focuses it.
            return EventOutcome::nothing();
        }
        if let Some(window) = self.managed(meta.x11) {
            // A window the table still holds is being shown again, which is a
            // request for attention rather than a new window.
            return EventOutcome {
                window: Some(window),
                changes: vec![NodeChange::Activate { window }],
                commands: vec![X11Request::SetMapped { window, mapped: true }],
            };
        }
        let Some(opened) = self.opened(meta.x11) else {
            return EventOutcome::nothing();
        };
        let window = opened.window;
        if let Some(entry) = self.windows.get_mut(&meta.x11) {
            entry.managed = Some(window);
        }
        // Mapping is the X11 equivalent of a toplevel's first committed buffer: the
        // window is now a candidate for focus, and whether it takes it is the
        // compositor's answer.
        EventOutcome {
            window: Some(window),
            changes: vec![NodeChange::Opened(opened), NodeChange::Activate { window }],
            commands: vec![X11Request::SetMapped { window, mapped: true }],
        }
    }

    /// A window left the screen, or went away with its client.
    fn closed(&mut self, x11: u32) -> EventOutcome {
        let Some(entry) = self.windows.remove(&x11) else {
            return EventOutcome::nothing();
        };
        let Some(window) = entry.managed else {
            return EventOutcome::nothing();
        };
        EventOutcome {
            window: Some(window),
            changes: vec![NodeChange::Closed { window }],
            commands: Vec::new(),
        }
    }
}

impl Windows for Xwm {
    fn managed_window(&self, xwindow: u32) -> Option<WindowId> {
        self.managed(xwindow)
    }

    fn handle(&mut self, event: WindowEvent<'_>) -> EventOutcome {
        match event {
            WindowEvent::Created(meta) => {
                self.remember(&meta);
                EventOutcome::nothing()
            }
            WindowEvent::MapRequested(meta) => self.map_requested(&meta),
            WindowEvent::Mapped(x11) => EventOutcome::of(self.managed(x11)),
            WindowEvent::Unmapped(x11) | WindowEvent::Destroyed(x11) => self.closed(x11),
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
                    changes: vec![NodeChange::Geometry {
                        window,
                        geometry: candidate,
                    }],
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
                // A client that moved itself is honoured: the compositor clamps the
                // rectangle and the client hears the answer, which is what makes an
                // X11 application's own positioning work on a floating desktop.
                EventOutcome {
                    window: Some(window),
                    changes: vec![NodeChange::Geometry { window, geometry }],
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
                    changes: vec![NodeChange::Title {
                        window,
                        title: title.to_owned(),
                    }],
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
                    changes: vec![NodeChange::AppId { window, app_id }],
                    commands: Vec::new(),
                }
            }
            WindowEvent::ActivationRequested(x11) => {
                let Some(window) = self.managed(x11) else {
                    // The request came from a window the table does not hold, which
                    // includes every override-redirect window: a menu cannot take focus.
                    return EventOutcome::nothing();
                };
                EventOutcome {
                    window: Some(window),
                    changes: vec![NodeChange::Activate { window }],
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
            changes: Vec::new(),
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

    /// The node `changes` open, if they open one.
    fn opened(changes: &[NodeChange]) -> Option<&Opened> {
        changes.iter().find_map(|change| match change {
            NodeChange::Opened(opened) => Some(opened),
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
        let mut popup = meta(9, "Gimp", "menu");
        popup.override_redirect = true;

        assert_eq!(xwm.handle(WindowEvent::Created(popup.clone())), EventOutcome::nothing());
        assert_eq!(
            xwm.handle(WindowEvent::MapRequested(popup.clone())),
            EventOutcome::nothing()
        );
        assert_eq!(xwm.handle(WindowEvent::ActivationRequested(9)), EventOutcome::nothing());

        assert_eq!(xwm.managed(9), None);
        assert!(xwm.knows(9), "the table still answers about the window");
    }

    /// Mapping is what takes a window into management: the window opens with its title,
    /// its class as the application id and the rectangle it asked for, asks for focus,
    /// and the server is told it is mapped.
    #[test]
    fn a_mapped_window_opens_with_its_title_and_class() {
        let mut xwm = Xwm::new(false, Vec::new());
        let window = window_id(3);

        let outcome = xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "kieran@fsociety")));

        assert_eq!(outcome.window, Some(window));
        assert_eq!(
            outcome.changes,
            vec![
                NodeChange::Opened(Opened {
                    window,
                    app_id: "XTerm".to_owned(),
                    title: "kieran@fsociety".to_owned(),
                    transient_for: None,
                    requested: Rect::new(0, 0, 300, 200),
                }),
                NodeChange::Activate { window },
            ]
        );
        assert_eq!(outcome.commands, vec![X11Request::SetMapped { window, mapped: true }]);
        assert_eq!(xwm.managed(3), Some(window));
    }

    /// A window with no class still has an application id: the instance name.
    #[test]
    fn a_window_with_no_class_is_matched_by_its_instance() {
        let mut xwm = Xwm::new(false, Vec::new());

        let outcome = xwm.handle(WindowEvent::MapRequested(meta(3, "", "t")));

        assert_eq!(
            opened(&outcome.changes).map(|opened| opened.app_id.as_str()),
            Some("instance")
        );
    }

    /// Mapping a window the table still holds is a request for attention, not a second
    /// window: it opens nothing and asks for focus again.
    #[test]
    fn a_window_mapped_again_is_not_opened_twice() {
        let mut xwm = Xwm::new(false, Vec::new());
        let window = window_id(3);
        xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "t")));

        let outcome = xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "t")));

        assert_eq!(outcome.changes, vec![NodeChange::Activate { window }]);
        assert_eq!(outcome.commands, vec![X11Request::SetMapped { window, mapped: true }]);
    }

    /// A client that exits sends no unmap, so a destroy closes the window the same way
    /// an unmap does: no window, no focus, no ghost.
    #[test]
    fn a_destroyed_window_leaves_the_table() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(4, "XTerm", "t")));
        let window = window_id(4);

        let outcome = xwm.handle(WindowEvent::Destroyed(4));

        assert_eq!(outcome.window, Some(window));
        assert_eq!(outcome.changes, vec![NodeChange::Closed { window }]);
        assert!(!xwm.knows(4), "the table forgets the window");
        assert_eq!(xwm.managed_window(4), None);
    }

    /// An unmap closes the window too, and a window that was never managed closes
    /// nothing.
    #[test]
    fn an_unmapped_window_closes_and_an_unmanaged_one_closes_nothing() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(4, "XTerm", "t")));
        let window = window_id(4);

        assert_eq!(
            xwm.handle(WindowEvent::Unmapped(4)).changes,
            vec![NodeChange::Closed { window }]
        );

        let mut created_only = Xwm::new(false, Vec::new());
        created_only.handle(WindowEvent::Created(meta(5, "XTerm", "t")));
        assert_eq!(created_only.handle(WindowEvent::Destroyed(5)), EventOutcome::nothing());
        assert!(!created_only.knows(5));
    }

    /// An event about a window the table never saw is not an open: a stale title or
    /// configure from a closing client must not create anything.
    #[test]
    fn an_event_for_an_unknown_window_changes_nothing() {
        let mut xwm = Xwm::new(false, Vec::new());

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
            assert_eq!(xwm.handle(event), EventOutcome::nothing());
        }
        assert!(!xwm.knows(77));
    }

    /// A configure request that changes one half of the rectangle keeps the other half
    /// as the table last had it, so the compositor places the whole rectangle.
    #[test]
    fn a_configure_request_keeps_the_fields_the_client_left_alone() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(5, "XTerm", "t")));
        let window = window_id(5);

        let moved = xwm.handle(WindowEvent::ConfigureRequested {
            x11: 5,
            geometry: Rect::new(40, 50, 999, 999),
            position: true,
            size: false,
        });
        assert_eq!(
            moved.changes,
            vec![NodeChange::Geometry {
                window,
                geometry: Rect::new(40, 50, 300, 200),
            }]
        );

        let resized = xwm.handle(WindowEvent::ConfigureRequested {
            x11: 5,
            geometry: Rect::new(9_000, 9_000, 600, 400),
            position: false,
            size: true,
        });
        assert_eq!(
            resized.changes,
            vec![NodeChange::Geometry {
                window,
                geometry: Rect::new(0, 0, 600, 400),
            }],
            "the position is the one the server last reported, not the request's"
        );
    }

    /// A configure the server reports is recorded and handed on as the window's
    /// geometry; an override-redirect window's own configure is recorded and answered
    /// with nothing.
    #[test]
    fn a_configured_window_reports_its_geometry() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(5, "XTerm", "t")));
        let mut popup = meta(6, "Gimp", "menu");
        popup.override_redirect = true;
        xwm.handle(WindowEvent::Created(popup));

        let geometry = Rect::new(10, 20, 30, 40);
        assert_eq!(
            xwm.handle(WindowEvent::Configured { x11: 5, geometry }).changes,
            vec![NodeChange::Geometry {
                window: window_id(5),
                geometry,
            }]
        );
        assert_eq!(
            xwm.handle(WindowEvent::Configured { x11: 6, geometry }),
            EventOutcome::nothing()
        );
    }

    /// A dialog names the window it is transient for when the table holds it, which is
    /// what keeps a file chooser over the application that opened it; a parent the
    /// table does not hold is not named.
    #[test]
    fn a_transient_window_names_its_managed_parent() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(6, "App", "main")));
        let mut dialog = meta(7, "App", "open file");
        dialog.transient_for = Some(6);

        let outcome = xwm.handle(WindowEvent::MapRequested(dialog));

        assert_eq!(outcome.window, Some(window_id(7)));
        assert_eq!(
            opened(&outcome.changes).and_then(|opened| opened.transient_for),
            Some(window_id(6))
        );

        let mut orphan = meta(8, "App", "orphan");
        orphan.transient_for = Some(99);
        let outcome = xwm.handle(WindowEvent::MapRequested(orphan));
        assert_eq!(opened(&outcome.changes).and_then(|opened| opened.transient_for), None);
    }

    /// A title or class change on a managed window is a change to its node; the class
    /// falls back to the instance name as at the open, and an unmanaged window's change
    /// is recorded and reported as nothing.
    #[test]
    fn a_title_and_a_class_change_reach_the_node() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "old")));
        let window = window_id(3);

        assert_eq!(
            xwm.handle(WindowEvent::TitleChanged { x11: 3, title: "new" }).changes,
            vec![NodeChange::Title {
                window,
                title: "new".to_owned(),
            }]
        );
        assert_eq!(
            xwm.handle(WindowEvent::ClassChanged(meta(3, "Renamed", "new"))).changes,
            vec![NodeChange::AppId {
                window,
                app_id: "Renamed".to_owned(),
            }]
        );
        assert_eq!(
            xwm.handle(WindowEvent::ClassChanged(meta(3, "", "new"))).changes,
            vec![NodeChange::AppId {
                window,
                app_id: "instance".to_owned(),
            }]
        );

        xwm.handle(WindowEvent::Created(meta(4, "Menu", "m")));
        assert_eq!(
            xwm.handle(WindowEvent::TitleChanged { x11: 4, title: "n" }),
            EventOutcome::nothing()
        );
        assert_eq!(
            xwm.handle(WindowEvent::ClassChanged(meta(4, "Other", "n"))),
            EventOutcome::nothing()
        );
    }

    /// A managed window that asks for focus changes its node's focus request; an
    /// unmanaged one cannot.
    #[test]
    fn a_managed_window_can_ask_for_activation() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(3, "XTerm", "t")));

        assert_eq!(
            xwm.handle(WindowEvent::ActivationRequested(3)).changes,
            vec![NodeChange::Activate { window: window_id(3) }]
        );
    }

    /// Closing a managed window is a `WM_DELETE_WINDOW` request, so a client with
    /// unsaved work can refuse it; an unmanaged window has nothing to close.
    #[test]
    fn a_close_request_is_a_delete_window() {
        let mut xwm = Xwm::new(false, Vec::new());
        xwm.handle(WindowEvent::MapRequested(meta(8, "App", "main")));

        assert_eq!(
            xwm.close(window_id(8)),
            EventOutcome {
                window: Some(window_id(8)),
                changes: Vec::new(),
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
