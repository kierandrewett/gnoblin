//! The seat's keyboard: the keymap, the repeat, the layout memory and the delivery
//! order.
//!
//! This is the "seat" half of the plan's input path. `gnoblin-input` decides *where* an
//! edge goes and `gnoblin-wayland` decides *what it means to a client*, and this module is
//! the join between them:
//!
//! ```text
//! backend key -> xkb keysym (the keymap this module installed)
//!             -> gnoblin-input's Router: grab, then binding, then client
//!             -> wl_keyboard.key for a client edge, nothing for a consumed one
//! ```
//!
//! ## The keymap
//!
//! Layouts come from the configuration document's `input-sources`
//! (`docs/config/configure/input_sources.md`): the entries of type `xkb` are the layouts
//! the session can switch between, and their `layout+variant` ids are split on `+` because
//! that is how the document writes them. The active source is compiled into a keymap and
//! handed to the seat, which is what a client sees in `wl_keyboard.keymap`.
//!
//! One keymap holds one source, so switching rebuilds it. Mutter instead compiles every
//! source into one keymap's groups and switches groups, which costs nothing per switch;
//! rebuilding is the smaller change and produces the same observable behaviour for a
//! client, and the cost is bounded by how often a user switches layout.
//!
//! ## Per-window layout memory
//!
//! With `per_window = true`, a window remembers the source that was active while it had
//! focus, and gets it back when it is focused again. A window focused for the first time
//! takes the source that is active at that moment, which is the rule the configuration
//! reference states. The memory is keyed by window identity, so it is dropped with the
//! window.
//!
//! ## Delivery order
//!
//! [`gnoblin_input::Router`] is the authority on where an edge goes and it owns half of
//! each pair: a press that went to a binding keeps its release whatever the priority order
//! says later. That is why a binding "never reaches the client" is true even when the
//! binding is removed, the grab changes or the layout changes while the key is held: the
//! release is looked up by hardware key code, which no layout change moves.
//!
//! # Failure modes
//!
//! 1. **A bound key's release reaches the client.** The router owns both edges of a bound
//!    press, so the client sees neither:
//!    `a_bound_key_is_consumed_and_its_release_is_not_leaked` in `tests/input.rs`.
//! 2. **A layout change under a held key delivers an orphan release.** Ownership is
//!    recorded by hardware key code, so the release follows its press even though the
//!    keymap — and therefore the keysym — changed in between:
//!    `a_keymap_change_under_a_held_key_keeps_its_target`.
//! 3. **A keymap that cannot be compiled leaves the seat with no keymap.** The previous
//!    keymap stays installed and the failure is logged, because a session with a stale
//!    layout is usable and a session with no keymap is not.
//! 4. **A window keeps a layout after it closed.** The memory is dropped with the window,
//!    so a recycled window identity cannot inherit another window's source.

use std::collections::BTreeMap;

use smithay::backend::input::{InputTime, KeyState as SmithayKeyState};
use smithay::input::keyboard::Keysym as SmithayKeysym;
use smithay::input::keyboard::{FilterResult, KeysymHandle, ModifiersState, XkbConfig, xkb};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;
use tracing::{debug, warn};

use gnoblin_input::{
    Chord, ChordKind, Event as RouterEvent, KeyState as RouterKeyState, Keysyms, Name, Router, ShortcutInhibit, Symbol,
    Target, Trigger,
};
use gnoblin_types::{KeyCode, KeyState, Keysym, Modifiers, WindowId};

use crate::State;

/// The offset between an `evdev` key code and the XKB key code of the same key.
///
/// XKB key codes are the `evdev` code plus eight, which is the number a
/// `wl_keyboard.key` event carries; the seat's keymap is an XKB one, so this is the
/// conversion at the boundary between the two.
const SEAT_KEYCODE_OFFSET: u32 = 8;

/// One layout the session can switch between, as the configuration document writes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeySource {
    /// The XKB layout, such as `us`.
    pub(crate) layout: String,
    /// The XKB variant, such as `intl`. Empty means the layout's base variant.
    pub(crate) variant: String,
}

impl KeySource {
    /// Parses a document id: `us`, `gb`, `us+intl`.
    fn from_id(id: &str) -> Self {
        match id.split_once('+') {
            Some((layout, variant)) => Self {
                layout: layout.to_owned(),
                variant: variant.to_owned(),
            },
            None => Self {
                layout: id.to_owned(),
                variant: String::new(),
            },
        }
    }
}

/// The keyboard's bookkeeping: the sources, the memory and the router.
#[derive(Debug)]
pub(crate) struct KeyboardTables {
    /// The switchable layouts, in the document's order.
    pub(crate) sources: Vec<KeySource>,
    /// Which source is active for a window that has no memory of its own.
    pub(crate) active: usize,
    /// Whether a window remembers its own source.
    pub(crate) per_window: bool,
    /// The source each window last had.
    pub(crate) remembered: BTreeMap<WindowId, usize>,
    /// The XKB options the document asks for, joined as xkbcommon writes them.
    pub(crate) options: Option<String>,
    /// The delivery order: grabs, bindings and the clients' edges.
    pub(crate) router: Router,
}

impl Default for KeyboardTables {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            active: 0,
            per_window: false,
            remembered: BTreeMap::new(),
            options: None,
            router: Router::new(),
        }
    }
}

impl KeyboardTables {
    /// The source that is active for a window.
    fn source_of(&self, window: Option<WindowId>) -> usize {
        match window {
            Some(window) if self.per_window => self.remembered.get(&window).copied().unwrap_or(self.active),
            _ => self.active,
        }
    }
}

/// The keysym table a chord is parsed against.
///
/// `gnoblin-input` decides the grammar and this supplies the one table it cannot know: the
/// name-to-keysym mapping, which belongs to xkbcommon.
struct SeatKeysyms;

impl Keysyms for SeatKeysyms {
    fn resolve(&self, name: &str) -> Option<Symbol> {
        let symbol = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        if symbol == SmithayKeysym::NoSymbol {
            return None;
        }
        Some(Symbol::new(Keysym::new(symbol.raw()), name))
    }
}

/// The modifier bits the router matches against, from Smithay's own view of the state.
fn modifiers_of(state: &ModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    for (flag, on) in [
        (Modifiers::SHIFT, state.shift),
        (Modifiers::LOCK, state.caps_lock),
        (Modifiers::CONTROL, state.ctrl),
        (Modifiers::MOD1, state.alt),
        (Modifiers::MOD2, state.num_lock),
        (Modifiers::MOD3, state.iso_level3_shift),
        (Modifiers::MOD4, state.logo),
        (Modifiers::SUPER, state.logo),
    ] {
        if on {
            modifiers = modifiers.union(flag);
        }
    }
    modifiers
}

impl State {
    /// Applies the configuration document's `input-sources` section.
    ///
    /// A session with no section, or with no `xkb` source in it, keeps the keymap the seat
    /// already has: the configuration reference is explicit that a document without the
    /// section leaves the current keymap in place.
    pub(crate) fn apply_input_sources(&mut self, sources: &gnoblin_config::InputSources) {
        let layouts: Vec<KeySource> = sources
            .sources
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|source| source.r#type == Some(gnoblin_config::InputSourceType::Xkb))
            .filter_map(|source| source.id.as_deref())
            .filter(|id| !id.is_empty())
            .map(KeySource::from_id)
            .collect();
        if layouts.is_empty() {
            return;
        }
        self.keyboard_tables.sources = layouts;
        self.keyboard_tables.per_window = sources.per_window.unwrap_or(false);
        self.keyboard_tables.active = 0;
        self.keyboard_tables.remembered.clear();
        self.rebuild_keymap();
    }

    /// Applies the document's keyboard repeat settings and XKB options.
    pub(crate) fn apply_keyboard_settings(&mut self, keyboard: &gnoblin_config::Keyboard) {
        use gnoblin_config::Inherit;
        let milliseconds = |setting: &Option<Inherit<i64>>, default: i32| match setting {
            Some(Inherit::Value(value)) => i32::try_from(*value).unwrap_or(default),
            _ => default,
        };
        let delay = milliseconds(&keyboard.delay, crate::state::KEY_REPEAT_DELAY);
        let interval = milliseconds(&keyboard.repeat_interval, crate::state::KEY_REPEAT_INTERVAL);
        let enabled = !matches!(keyboard.repeat, Some(Inherit::Value(false)));
        let rate = if enabled && interval > 0 { 1000 / interval } else { 0 };
        self.keyboard_tables.options = match &keyboard.xkb_options {
            Some(Inherit::Value(options)) if !options.is_empty() => Some(options.join(",")),
            _ => None,
        };
        let handle = self.keyboard.clone();
        handle.change_repeat_info(rate, delay);
        self.rebuild_keymap();
    }

    /// The keymap for the active source, compiled into the seat.
    fn rebuild_keymap(&mut self) {
        let index = self.keyboard_tables.active;
        let Some(source) = self.keyboard_tables.sources.get(index).cloned() else {
            return;
        };
        let config = XkbConfig {
            rules: "",
            model: "",
            layout: &source.layout,
            variant: &source.variant,
            options: self.keyboard_tables.options.clone(),
        };
        let handle = self.keyboard.clone();
        match handle.set_xkb_config(self, config) {
            Ok(()) => debug!(
                layout = %source.layout,
                variant = %source.variant,
                "[keyboard] the keymap changed"
            ),
            // The previous keymap stays installed: a session with a stale layout is usable
            // and one with no keymap is not.
            Err(error) => warn!(?error, layout = %source.layout, "[keyboard] the keymap was refused"),
        }
    }

    /// Selects a source by index for the window that has focus, and rebuilds the keymap.
    ///
    /// `false` means the index names no configured source, which is what a caller gets for
    /// a document that changed under it.
    pub fn select_input_source(&mut self, index: usize) -> bool {
        if index >= self.keyboard_tables.sources.len() {
            return false;
        }
        self.keyboard_tables.active = index;
        if self.keyboard_tables.per_window
            && let Some(window) = self.wm().focused()
        {
            self.keyboard_tables.remembered.insert(window, index);
        }
        self.rebuild_keymap();
        true
    }

    /// How many switchable layouts the session has.
    #[must_use]
    pub fn input_sources(&self) -> usize {
        self.keyboard_tables.sources.len()
    }

    /// Drops a closed window's layout memory.
    pub(crate) fn forget_layout(&mut self, window: WindowId) {
        self.keyboard_tables.remembered.remove(&window);
    }

    /// Records the layout of the window focus is leaving.
    ///
    /// The window is read from the seat's own keyboard focus, not from the window manager's:
    /// [`State::set_keyboard_focus`] runs *after* the manager has already applied the focus
    /// change, so asking the manager at this point names the window that is arriving and a
    /// window would overwrite its own memory with the layout it is about to be given.
    pub(crate) fn remember_layout(&mut self) {
        if !self.keyboard_tables.per_window {
            return;
        }
        let leaving = self.keyboard.current_focus();
        if let Some(window) = leaving.as_ref().and_then(|surface| self.window_of(surface)) {
            self.keyboard_tables
                .remembered
                .insert(window, self.keyboard_tables.active);
        }
    }

    /// Restores the layout the newly focused surface's window remembers.
    pub(crate) fn restore_layout(&mut self, target: Option<&WlSurface>) {
        let window = target.and_then(|surface| self.window_of(surface));
        let wanted = self.keyboard_tables.source_of(window);
        if wanted == self.keyboard_tables.active || wanted >= self.keyboard_tables.sources.len() {
            return;
        }
        self.keyboard_tables.active = wanted;
        self.rebuild_keymap();
    }

    /// Adds a binding to the session's router.
    ///
    /// It is the seam a configuration reload and `gnoblinctl` use: the chord is parsed
    /// against xkbcommon's name table, and a chord or a name the grammar refuses is
    /// reported rather than silently dropped.
    pub fn add_binding(&mut self, name: &str, chord: &str) -> bool {
        let Ok(name) = Name::new(name) else {
            return false;
        };
        let Ok(chord) = Chord::parse(chord, ChordKind::Key, &SeatKeysyms) else {
            return false;
        };
        self.keyboard_tables
            .router
            .bind(&name, chord, Trigger::Press, false)
            .is_ok()
    }

    /// Routes one key edge, and whether the client receives it.
    ///
    /// This is the whole of the delivery rule, and the reason it is here rather than in a
    /// protocol handler: the edge is looked up in the keymap the seat installed, decided by
    /// the router, and only then handed to `wl_keyboard`.
    pub(crate) fn dispatch_key(&mut self, keycode: u32, state: KeyState, serial: Serial, time: InputTime) {
        let keyboard = self.keyboard.clone();
        let key_state = match state {
            KeyState::Pressed => SmithayKeyState::Pressed,
            KeyState::Released => SmithayKeyState::Released,
        };
        // A backend and the configuration both name a key by its `evdev` code, and the
        // seat's keymap is an XKB one, whose key codes are the `evdev` code plus eight.
        // Smithay subtracts the eight again when it sends `wl_keyboard.key`, so a client
        // is told the code the Wayland protocol defines; passing the raw code here would
        // deliver every key eight too low.
        let seat_code = keycode.saturating_add(SEAT_KEYCODE_OFFSET);
        keyboard.input::<(), _>(
            self,
            seat_code.into(),
            key_state,
            serial,
            time,
            |data, modifiers, symbol| {
                if data.route_key_edge(keycode, state, modifiers, &symbol) {
                    FilterResult::Intercept(())
                } else {
                    FilterResult::Forward
                }
            },
        );
    }

    /// The router's half of one edge: a binding or a grab consumes it, a client edge is
    /// forwarded.
    fn route_key_edge(
        &mut self,
        keycode: u32,
        state: KeyState,
        modifiers: &ModifiersState,
        symbol: &KeysymHandle<'_>,
    ) -> bool {
        let inhibit = if self.focused_shortcuts_inhibited() {
            ShortcutInhibit::Inhibited
        } else {
            ShortcutInhibit::Clear
        };
        let routed = self.keyboard_tables.router.route(
            RouterEvent::Key {
                key_code: KeyCode::new(keycode),
                symbol: Keysym::new(symbol.modified_sym().raw()),
                modifiers: modifiers_of(modifiers),
                state: match state {
                    KeyState::Pressed => RouterKeyState::Pressed,
                    KeyState::Released => RouterKeyState::Released,
                },
                repeat: false,
            },
            inhibit,
        );
        let consumed = routed.target != Target::Client;
        if consumed {
            debug!(keycode, target = %routed.target, fired = ?routed.fired, "[key] consumed");
        }
        consumed
    }

    /// Whether the focused client holds a shortcut inhibitor.
    fn focused_shortcuts_inhibited(&mut self) -> bool {
        let seat = self.seat.clone();
        self.shortcuts_inhibited(&seat)
    }
}
