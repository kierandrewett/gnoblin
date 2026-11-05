//! Delivery order for the edges that a chord can claim: grab, then binding, then client.
//!
//! The plan's input pipeline is "lock gate, then grabs and bindings, then an optional
//! script policy decision, then seat delivery". The lock gate runs before this type
//! and the policy queue runs after it; this module owns the middle: whether a key or
//! button edge belongs to an active grab, to a chord binding, or to the focused
//! client, and it owns that decision for the whole life of the press.
//!
//! # Why the press owns its release
//!
//! A key edge is half of a pair, and a client that sees one half goes wrong: a press
//! without a release leaves a key stuck down, and a release without its press leaks
//! an event the user never made. So the router records where each press went and
//! sends its release there, whatever the priority order says by then:
//!
//! - A grab that starts while a key is held (the lock screen covering the session,
//!   a shell taking the keyboard for a launcher) does not steal that key's release.
//! - A grab that ends before its own press is released keeps the release, so the
//!   client never receives an orphan release after the grab.
//! - A binding removed by a reload while its press is held still consumes the
//!   release. Firing a callback the user just disabled would be worse than dropping
//!   the edge.
//!
//! A release with no recorded press, which happens when the router is created while
//! keys are already down or when a release is duplicated, is still routed by the
//! current priority order and counted, so a session can report it instead of
//! silently delivering a stuck-key edge.
//!
//! Motion, scroll and touch are not routed here: they cannot be half-delivered, and
//! the seat delivery path handles them directly. Buttons are routed here because a
//! pointer chord binds them, and because a button press is half of a pair for the
//! same reason a key is.

use std::collections::BTreeMap;
use std::fmt;

use crate::binding::{Binding, BindingId, BindingTable, Conflict, Trigger};
use crate::chord::Chord;
use crate::name::Name;
use gnoblin_types::{ButtonState, KeyCode, KeyState, Keysym, Modifiers};

/// Which devices a grab takes.
///
/// The lock cover and exclusive input capture take both; a popup or a keyboard-only
/// shell takes one. A grab takes precedence over every binding for as long as it is
/// active, which is the "grab, then bindings, then client" rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Keys only.
    Keyboard,
    /// Buttons only.
    Pointer,
    /// Keys and buttons.
    Both,
}

impl Scope {
    /// Whether a grab of this scope covers an edge that needs `needed`.
    pub const fn covers(self, needed: Self) -> bool {
        matches!(
            (self, needed),
            (Self::Both, _) | (Self::Keyboard, Self::Keyboard) | (Self::Pointer, Self::Pointer)
        )
    }
}

/// The identity of an active grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrabId(u64);

impl GrabId {
    /// The raw number, for logs and diagnostics.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GrabId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "grab:{}", self.0)
    }
}

/// One active grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grab {
    id: GrabId,
    scope: Scope,
}

impl Grab {
    /// The identity.
    pub const fn id(self) -> GrabId {
        self.id
    }

    /// What it takes.
    pub const fn scope(self) -> Scope {
        self.scope
    }
}

/// The edge to route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key edge.
    Key {
        /// The hardware key code, which does not change with the layout.
        key_code: KeyCode,
        /// The keysym the layout resolves the code to now.
        symbol: Keysym,
        /// The modifiers held now.
        modifiers: Modifiers,
        /// Press or release.
        state: KeyState,
        /// Whether this is an autorepeat, which only follows its press.
        repeat: bool,
    },
    /// A pointer button edge.
    Button {
        /// The button number.
        button: u8,
        /// The modifiers held now.
        modifiers: Modifiers,
        /// Press or release.
        state: ButtonState,
    },
}

/// Where an edge goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// An active grab takes it.
    Grab(GrabId),
    /// A chord binding owns it.
    Binding(BindingId),
    /// Normal seat delivery.
    Client,
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grab(id) => write!(formatter, "{id}"),
            Self::Binding(id) => write!(formatter, "{id}"),
            Self::Client => formatter.write_str("client"),
        }
    }
}

/// Whether the focused client is inhibiting compositor shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutInhibit {
    /// Shortcuts run normally.
    Clear,
    /// The client inhibits shortcuts, so only bindings that opt out still run.
    Inhibited,
}

impl ShortcutInhibit {
    /// Whether shortcuts are inhibited.
    pub const fn is_inhibited(self) -> bool {
        matches!(self, Self::Inhibited)
    }
}

/// Where one edge went, and what it ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Routed {
    /// The owner of the edge.
    pub target: Target,
    /// The binding whose configured edge this is, if this edge runs one.
    ///
    /// A press-triggered binding fires on its press. A release-triggered binding
    /// reserves its press and fires on its release, so its press reports `None`.
    pub fired: Option<BindingId>,
    /// Whether a release followed the target of its press rather than the order in
    /// force now.
    pub followed_press: bool,
}

/// Counters a session can report, and the model test checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Press edges routed.
    pub presses: u64,
    /// Release edges routed.
    pub releases: u64,
    /// Autorepeats routed.
    pub repeats: u64,
    /// Presses for a key or button that was already held, which means a release was
    /// lost somewhere before the router.
    pub duplicate_presses: u64,
    /// Releases with no recorded press.
    pub orphan_releases: u64,
    /// Releases routed to the target of their press.
    pub followed_press: u64,
    /// Edges that ran a binding.
    pub fired: u64,
    /// Edges delivered to a grab.
    pub grab_deliveries: u64,
    /// Edges delivered to a binding.
    pub binding_deliveries: u64,
    /// Edges delivered to the client.
    pub client_deliveries: u64,
    /// Grabs started.
    pub grabs_started: u64,
    /// Grabs ended.
    pub grabs_ended: u64,
}

/// A held key or button, as the router remembers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Pressed {
    Key(KeyCode),
    Button(u8),
}

/// The binding table plus the grabs and held edges that decide delivery.
#[derive(Clone, Debug, Default)]
pub struct Router {
    table: BindingTable,
    grabs: Vec<Grab>,
    owned: BTreeMap<Pressed, Target>,
    next_grab: u64,
    stats: Stats,
}

impl Router {
    /// A router with no bindings and no grabs.
    pub fn new() -> Self {
        Self::default()
    }

    /// A router over an existing table, so a reload can build the new table first
    /// and swap it in only once it has no conflicts.
    pub fn with_bindings(table: BindingTable) -> Self {
        Self {
            table,
            ..Self::default()
        }
    }

    /// The declared bindings.
    pub const fn bindings(&self) -> &BindingTable {
        &self.table
    }

    /// The bindings, for a reload. Held presses keep their ids, so a binding that
    /// survives a reload keeps the releases it already owns.
    pub const fn bindings_mut(&mut self) -> &mut BindingTable {
        &mut self.table
    }

    /// Declare a binding through the router's table.
    pub fn bind(
        &mut self,
        name: &Name,
        chord: Chord,
        trigger: Trigger,
        allow_when_inhibited: bool,
    ) -> Result<crate::binding::Insertion, Conflict> {
        self.table.insert(name, chord, trigger, allow_when_inhibited)
    }

    /// The active grabs, oldest first.
    pub fn grabs(&self) -> impl Iterator<Item = Grab> + '_ {
        self.grabs.iter().copied()
    }

    /// Start a grab and return its id. The newest grab wins for the scopes it takes.
    pub fn begin_grab(&mut self, scope: Scope) -> GrabId {
        let id = GrabId(self.next_grab);
        self.next_grab += 1;
        self.grabs.push(Grab { id, scope });
        self.stats.grabs_started += 1;
        id
    }

    /// End a grab. Edges whose press the grab already owns keep going to it.
    pub fn end_grab(&mut self, id: GrabId) -> bool {
        let Some(index) = self.grabs.iter().position(|grab| grab.id == id) else {
            return false;
        };
        self.grabs.remove(index);
        self.stats.grabs_ended += 1;
        true
    }

    /// How many keys and buttons are held.
    pub fn held(&self) -> usize {
        self.owned.len()
    }

    /// The counters so far.
    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// Route one edge.
    pub fn route(&mut self, event: Event, inhibit: ShortcutInhibit) -> Routed {
        match event {
            Event::Key {
                key_code,
                symbol,
                modifiers,
                state,
                repeat,
            } => self.route_key(key_code, symbol, modifiers, state, repeat, inhibit),
            Event::Button {
                button,
                modifiers,
                state,
            } => self.route_button(button, modifiers, state, inhibit),
        }
    }

    fn route_key(
        &mut self,
        key_code: KeyCode,
        symbol: Keysym,
        modifiers: Modifiers,
        state: KeyState,
        repeat: bool,
        inhibit: ShortcutInhibit,
    ) -> Routed {
        let pressed = Pressed::Key(key_code);
        if repeat {
            self.stats.repeats += 1;
            return match self.owned.get(&pressed).copied() {
                Some(target) => {
                    self.stats.followed_press += 1;
                    self.count(target);
                    Routed {
                        target,
                        fired: None,
                        followed_press: true,
                    }
                }
                None => {
                    self.stats.orphan_releases += 1;
                    let target = self.key_priority(symbol, modifiers, inhibit);
                    self.count(target);
                    Routed {
                        target,
                        fired: None,
                        followed_press: false,
                    }
                }
            };
        }
        match state {
            KeyState::Pressed => {
                self.stats.presses += 1;
                let (target, fired) = self.press_key(symbol, modifiers, inhibit);
                if self.owned.insert(pressed, target).is_some() {
                    self.stats.duplicate_presses += 1;
                }
                self.count(target);
                if fired.is_some() {
                    self.stats.fired += 1;
                }
                Routed {
                    target,
                    fired,
                    followed_press: false,
                }
            }
            KeyState::Released => {
                self.stats.releases += 1;
                match self.owned.remove(&pressed) {
                    Some(target) => {
                        let fired = self.release_fire(target);
                        if fired.is_some() {
                            self.stats.fired += 1;
                        }
                        self.stats.followed_press += 1;
                        self.count(target);
                        Routed {
                            target,
                            fired,
                            followed_press: true,
                        }
                    }
                    None => {
                        self.stats.orphan_releases += 1;
                        let target = self.key_priority(symbol, modifiers, inhibit);
                        self.count(target);
                        Routed {
                            target,
                            fired: None,
                            followed_press: false,
                        }
                    }
                }
            }
        }
    }

    fn route_button(
        &mut self,
        button: u8,
        modifiers: Modifiers,
        state: ButtonState,
        inhibit: ShortcutInhibit,
    ) -> Routed {
        let pressed = Pressed::Button(button);
        match state {
            ButtonState::Pressed => {
                self.stats.presses += 1;
                let (target, fired) = self.press_button(button, modifiers, inhibit);
                if self.owned.insert(pressed, target).is_some() {
                    self.stats.duplicate_presses += 1;
                }
                self.count(target);
                if fired.is_some() {
                    self.stats.fired += 1;
                }
                Routed {
                    target,
                    fired,
                    followed_press: false,
                }
            }
            ButtonState::Released => {
                self.stats.releases += 1;
                match self.owned.remove(&pressed) {
                    Some(target) => {
                        let fired = self.release_fire(target);
                        if fired.is_some() {
                            self.stats.fired += 1;
                        }
                        self.stats.followed_press += 1;
                        self.count(target);
                        Routed {
                            target,
                            fired,
                            followed_press: true,
                        }
                    }
                    None => {
                        self.stats.orphan_releases += 1;
                        let target = self.button_priority(button, modifiers, inhibit);
                        self.count(target);
                        Routed {
                            target,
                            fired: None,
                            followed_press: false,
                        }
                    }
                }
            }
        }
    }

    /// The target the priority order picks for a key press.
    fn key_priority(&self, symbol: Keysym, modifiers: Modifiers, inhibit: ShortcutInhibit) -> Target {
        if let Some(id) = self.top_grab(Scope::Keyboard) {
            return Target::Grab(id);
        }
        if let Some(binding) = self.binding_for_key(symbol, modifiers, inhibit) {
            return Target::Binding(binding.id());
        }
        Target::Client
    }

    /// The target and the binding that fires now, for a key press.
    fn press_key(&self, symbol: Keysym, modifiers: Modifiers, inhibit: ShortcutInhibit) -> (Target, Option<BindingId>) {
        if let Some(id) = self.top_grab(Scope::Keyboard) {
            return (Target::Grab(id), None);
        }
        let Some(binding) = self.binding_for_key(symbol, modifiers, inhibit) else {
            return (Target::Client, None);
        };
        let fired = if binding.fires_on(Trigger::Press) {
            Some(binding.id())
        } else {
            None
        };
        (Target::Binding(binding.id()), fired)
    }

    fn button_priority(&self, button: u8, modifiers: Modifiers, inhibit: ShortcutInhibit) -> Target {
        if let Some(id) = self.top_grab(Scope::Pointer) {
            return Target::Grab(id);
        }
        if let Some(binding) = self.binding_for_button(button, modifiers, inhibit) {
            return Target::Binding(binding.id());
        }
        Target::Client
    }

    fn press_button(&self, button: u8, modifiers: Modifiers, inhibit: ShortcutInhibit) -> (Target, Option<BindingId>) {
        if let Some(id) = self.top_grab(Scope::Pointer) {
            return (Target::Grab(id), None);
        }
        let Some(binding) = self.binding_for_button(button, modifiers, inhibit) else {
            return (Target::Client, None);
        };
        let fired = if binding.fires_on(Trigger::Press) {
            Some(binding.id())
        } else {
            None
        };
        (Target::Binding(binding.id()), fired)
    }

    fn binding_for_key(&self, symbol: Keysym, modifiers: Modifiers, inhibit: ShortcutInhibit) -> Option<&Binding> {
        self.table
            .iter()
            .find(|binding| binding.chord().matches_key(symbol, modifiers) && allowed(binding, inhibit))
    }

    fn binding_for_button(&self, button: u8, modifiers: Modifiers, inhibit: ShortcutInhibit) -> Option<&Binding> {
        self.table
            .iter()
            .find(|binding| binding.chord().matches_button(button, modifiers) && allowed(binding, inhibit))
    }

    /// The binding whose release edge this is, if the binding still exists.
    fn release_fire(&self, target: Target) -> Option<BindingId> {
        let Target::Binding(id) = target else {
            return None;
        };
        let binding = self.table.get(id)?;
        binding.fires_on(Trigger::Release).then_some(id)
    }

    /// The newest grab that covers `needed`.
    fn top_grab(&self, needed: Scope) -> Option<GrabId> {
        self.grabs
            .iter()
            .rev()
            .find(|grab| grab.scope.covers(needed))
            .map(|grab| grab.id)
    }

    fn count(&mut self, target: Target) {
        match target {
            Target::Grab(_) => self.stats.grab_deliveries += 1,
            Target::Binding(_) => self.stats.binding_deliveries += 1,
            Target::Client => self.stats.client_deliveries += 1,
        }
    }
}

/// Whether a binding may run while shortcuts are inhibited.
fn allowed(binding: &Binding, inhibit: ShortcutInhibit) -> bool {
    match inhibit {
        ShortcutInhibit::Clear => true,
        ShortcutInhibit::Inhibited => binding.allows_when_inhibited(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Routed, Router, Scope, ShortcutInhibit, Target};
    use crate::binding::Trigger;
    use crate::chord::{Chord, ChordKind, KeyCode, Keysym, Keysyms, Modifiers, Symbol};
    use crate::name::Name;
    use gnoblin_types::{ButtonState, KeyState};

    const Q: Keysym = Keysym::new(0x71);
    const W: Keysym = Keysym::new(0x77);

    struct Table;

    impl Keysyms for Table {
        fn resolve(&self, name: &str) -> Option<Symbol> {
            let lowered = name.to_ascii_lowercase();
            let (symbol, canonical) = match lowered.as_str() {
                "q" => (0x71, "q"),
                "w" => (0x77, "w"),
                _ => return None,
            };
            Some(Symbol::new(Keysym::new(symbol), canonical))
        }
    }

    fn chord(text: &str) -> Chord {
        Chord::parse(text, ChordKind::Key, &Table).expect(text)
    }

    fn pointer_chord(text: &str) -> Chord {
        Chord::parse(text, ChordKind::Pointer, &Table).expect(text)
    }

    fn name(text: &str) -> Name {
        Name::new(text).expect(text)
    }

    fn code_for(symbol: Keysym) -> KeyCode {
        if symbol == W {
            KeyCode::new(25)
        } else {
            KeyCode::new(24)
        }
    }

    fn press(symbol: Keysym, modifiers: Modifiers) -> Event {
        Event::Key {
            key_code: code_for(symbol),
            symbol,
            modifiers,
            state: KeyState::Pressed,
            repeat: false,
        }
    }

    fn release(symbol: Keysym, modifiers: Modifiers) -> Event {
        Event::Key {
            key_code: code_for(symbol),
            symbol,
            modifiers,
            state: KeyState::Released,
            repeat: false,
        }
    }

    fn route(router: &mut Router, event: Event) -> Routed {
        router.route(event, ShortcutInhibit::Clear)
    }

    #[test]
    fn a_grab_beats_a_binding_which_beats_the_client() {
        let mut router = Router::new();
        assert_eq!(route(&mut router, press(Q, Modifiers::NONE)).target, Target::Client);
        let id = router
            .bind(&name("q"), chord("q"), Trigger::Press, false)
            .expect("q")
            .id();
        let routed = route(&mut router, press(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Binding(id));
        assert_eq!(routed.fired, Some(id));
        let grab = router.begin_grab(Scope::Keyboard);
        let routed = route(&mut router, press(W, Modifiers::NONE));
        assert_eq!(routed.target, Target::Grab(grab));
        assert_eq!(routed.fired, None);
        route(&mut router, release(W, Modifiers::NONE));
        assert!(router.end_grab(grab));
        assert_eq!(
            route(&mut router, release(Q, Modifiers::NONE)).target,
            Target::Binding(id)
        );
    }

    #[test]
    fn a_release_trigger_reserves_its_press_and_fires_once() {
        let mut router = Router::new();
        let id = router
            .bind(&name("r"), chord("q"), Trigger::Release, false)
            .expect("q")
            .id();
        let routed = route(&mut router, press(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Binding(id));
        assert_eq!(routed.fired, None);
        let routed = route(&mut router, release(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Binding(id));
        assert_eq!(routed.fired, Some(id));
    }

    #[test]
    fn an_inhibited_client_skips_bindings_that_do_not_opt_out() {
        let mut router = Router::new();
        router
            .bind(&name("plain"), chord("q"), Trigger::Press, false)
            .expect("q");
        router.bind(&name("loud"), chord("w"), Trigger::Press, true).expect("w");
        let inhibited = ShortcutInhibit::Inhibited;
        assert_eq!(
            router.route(press(Q, Modifiers::NONE), inhibited).target,
            Target::Client
        );
        let routed = router.route(press(W, Modifiers::NONE), inhibited);
        assert!(matches!(routed.target, Target::Binding(_)));
    }

    #[test]
    fn a_pointer_chord_binds_a_button_and_plain_clicks_pass() {
        let mut router = Router::new();
        let id = router
            .bind(&name("resize"), pointer_chord("<Super>Button3"), Trigger::Press, false)
            .expect("pointer")
            .id();
        let routed = router.route(
            Event::Button {
                button: 3,
                modifiers: Modifiers::SUPER,
                state: ButtonState::Pressed,
            },
            ShortcutInhibit::Clear,
        );
        assert_eq!(routed.target, Target::Binding(id));
        let routed = router.route(
            Event::Button {
                button: 1,
                modifiers: Modifiers::NONE,
                state: ButtonState::Pressed,
            },
            ShortcutInhibit::Clear,
        );
        assert_eq!(routed.target, Target::Client);
        router.route(
            Event::Button {
                button: 1,
                modifiers: Modifiers::NONE,
                state: ButtonState::Released,
            },
            ShortcutInhibit::Clear,
        );
    }

    #[test]
    fn a_grab_after_the_press_does_not_take_a_held_keys_release() {
        let mut router = Router::new();
        assert_eq!(route(&mut router, press(Q, Modifiers::NONE)).target, Target::Client);
        let grab = router.begin_grab(Scope::Both);
        assert_eq!(route(&mut router, press(W, Modifiers::NONE)).target, Target::Grab(grab));
        let routed = route(&mut router, release(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Client);
        assert!(routed.followed_press);
        assert_eq!(
            route(&mut router, release(W, Modifiers::NONE)).target,
            Target::Grab(grab)
        );
        assert!(router.end_grab(grab));
        assert_eq!(router.held(), 0);
    }

    #[test]
    fn a_binding_removed_while_held_still_consumes_the_release() {
        let mut router = Router::new();
        let id = router
            .bind(&name("q"), chord("q"), Trigger::Press, false)
            .expect("q")
            .id();
        assert_eq!(
            route(&mut router, press(Q, Modifiers::NONE)).target,
            Target::Binding(id)
        );
        assert!(router.bindings_mut().remove(id).is_some());
        let routed = route(&mut router, release(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Binding(id));
        assert_eq!(routed.fired, None);
        assert_eq!(router.stats().client_deliveries, 0);
    }

    #[test]
    fn a_repeat_follows_its_press_and_never_fires() {
        let mut router = Router::new();
        let id = router
            .bind(&name("q"), chord("q"), Trigger::Press, false)
            .expect("q")
            .id();
        assert_eq!(route(&mut router, press(Q, Modifiers::NONE)).fired, Some(id));
        let repeat = Event::Key {
            key_code: code_for(Q),
            symbol: Q,
            modifiers: Modifiers::NONE,
            state: KeyState::Pressed,
            repeat: true,
        };
        let routed = route(&mut router, repeat);
        assert_eq!(routed.target, Target::Binding(id));
        assert_eq!(routed.fired, None);
        route(&mut router, release(Q, Modifiers::NONE));
    }

    #[test]
    fn an_orphan_release_follows_the_priority_order_and_is_counted() {
        let mut router = Router::new();
        let routed = route(&mut router, release(Q, Modifiers::NONE));
        assert_eq!(routed.target, Target::Client);
        assert!(!routed.followed_press);
        assert_eq!(router.stats().orphan_releases, 1);
        assert_eq!(router.stats().presses, 0);
    }
}
