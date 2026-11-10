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

use gnoblin_backend::InputEvent;
use gnoblin_input::{
    Chord, ChordKind, Event as RouterEvent, KeyState as RouterKeyState, Keysyms, Name, Router, ShortcutInhibit, Symbol,
    Target, Trigger,
};
use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{KeyCode, KeyState, Keysym, Modifiers, NodeId, WindowId};

use crate::State;
use crate::observation::{MenuAnchor, MenuSource};
use crate::tree_frame::{ChordEvent, ChordState};

use crate::log;

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

/// The name of the binding that opens the window menu for the focused window.
///
/// The name is what a configured binding must use to replace the default, so it is part
/// of the configuration contract rather than an internal detail.
pub(crate) const WINDOW_MENU_BINDING: &str = "window-menu";

/// The default chord for the window-menu binding, Alt+Space: the conventional key for a
/// window's system menu on the desktops this compositor follows.
pub(crate) const WINDOW_MENU_CHORD: &str = "<Alt>space";

/// Which keymap the seat holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeymapOwner {
    /// The layout compiled from the configuration's `input-sources`.
    Layout,
    /// The keymap a virtual keyboard sent, by that keyboard's id.
    Virtual(u64),
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
    /// The command each configured shortcut runs, by name. A fired binding with no entry
    /// here is the window menu, or nothing.
    pub(crate) shortcut_commands: BTreeMap<String, Vec<String>>,
    /// Which keymap the seat holds: the layout's, or a virtual keyboard's.
    pub(crate) keymap_owner: KeymapOwner,
    /// The keymap text each virtual keyboard sent, by its id, while it holds one.
    pub(crate) virtual_keymaps: BTreeMap<u64, String>,
    /// The id the next virtual keyboard takes.
    pub(crate) next_virtual_id: u64,
    /// The seat claims the shell holds, by their router binding's name.
    pub(crate) claims: BTreeMap<String, Claim>,
}

impl Default for KeyboardTables {
    fn default() -> Self {
        let mut router = Router::new();
        // The default window-menu binding, Alt+Space. It is declared here, in the same
        // table as every other binding, so a configured binding of the same name replaces
        // it in place rather than adding a second one. A default that does not parse is a
        // bug in this file, not in the user's configuration, so it fails loudly.
        let name = Name::new(WINDOW_MENU_BINDING).expect("the default window-menu name is valid ASCII");
        let chord = Chord::parse(WINDOW_MENU_CHORD, ChordKind::Key, &SeatKeysyms)
            .expect("the default window-menu chord parses against the seat's keysyms");
        let inserted = router.bind(&name, chord, Trigger::Press, false);
        debug_assert!(
            inserted.is_ok(),
            "the default window-menu binding conflicts with nothing"
        );
        Self {
            sources: Vec::new(),
            active: 0,
            per_window: false,
            remembered: BTreeMap::new(),
            options: None,
            router,
            shortcut_commands: BTreeMap::new(),
            keymap_owner: KeymapOwner::Layout,
            virtual_keymaps: BTreeMap::new(),
            next_virtual_id: 0,
            claims: BTreeMap::new(),
        }
    }
}

/// A chord the shell claimed on a seat node: the claim's id and the chord as the router reports it.
#[derive(Clone, Debug)]
pub(crate) struct Claim {
    pub(crate) node: NodeId,
    pub(crate) id: i64,
    pub(crate) chord: String,
}

/// The router binding name for a claim. The name grammar allows only ASCII alphanumerics, `-` and
/// `_`, and a node's digits never contain `-`, so the name is unambiguous.
fn claim_name(node: NodeId, id: i64) -> String {
    format!("claim-{}-{}", node.get(), id)
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

/// Runs a configured shortcut's command directly, with no shell, as the document says.
///
/// The child is waited on from a helper thread so that it is reaped when it exits, rather
/// than staying a zombie for the rest of the session.
pub(crate) fn spawn_shortcut_command(name: &str, argv: &[String]) {
    let Some((program, arguments)) = argv.split_first() else {
        return;
    };
    match std::process::Command::new(program).args(arguments).spawn() {
        Ok(mut child) => {
            let waiter = std::thread::Builder::new()
                .name(format!("shortcut-{name}"))
                .spawn(move || {
                    let _ = child.wait();
                });
            if waiter.is_err() {
                log::warn!(
                    area = "keyboard",
                    shortcut = name,
                    "the shortcut's waiter thread did not start"
                );
            }
        }
        Err(error) => log::warn!(
            area = "keyboard",
            shortcut = name,
            ?error,
            "the shortcut's command did not start"
        ),
    }
}

/// Builds one declaration from a configured name, chord and edge. The chord is parsed here so
/// a bad one is refused with the shortcut named, rather than left to fail later.
fn declare(name: &str, binding: &str, trigger: Trigger) -> Result<gnoblin_input::Declaration, String> {
    let name_value = Name::new(name).map_err(|error| format!("the name {name:?} is not valid: {error}"))?;
    let chord = Chord::parse(binding, ChordKind::Key, &SeatKeysyms).map_err(|error| error.message())?;
    Ok(gnoblin_input::Declaration {
        name: name_value,
        chord,
        trigger,
        allow_when_inhibited: false,
    })
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

    /// Applies the configuration document's `shortcuts` section as one set.
    ///
    /// The set is built in full before the table changes, so one bad shortcut, or a chord
    /// that another shortcut already owns, refuses the lot and the bindings that worked stay
    /// in force. A configured `window-menu` entry replaces the default window-menu binding,
    /// and a disabled one leaves that binding unbound.
    pub(crate) fn apply_shortcuts(&mut self, shortcuts: &gnoblin_config::Shortcuts) {
        let mut declared = Vec::new();
        let mut commands = BTreeMap::new();
        if !shortcuts.shortcuts.contains_key(WINDOW_MENU_BINDING) {
            declared.push(
                declare(WINDOW_MENU_BINDING, WINDOW_MENU_CHORD, Trigger::Press)
                    .expect("the default window-menu binding parses against the seat's keysyms"),
            );
        }
        for (name, shortcut) in &shortcuts.shortcuts {
            if shortcut.enable == Some(false) {
                continue;
            }
            let trigger = match shortcut.trigger {
                Some(gnoblin_config::ShortcutTrigger::Release) => Trigger::Release,
                _ => Trigger::Press,
            };
            let binding = shortcut.binding.as_deref().unwrap_or_default();
            match declare(name, binding, trigger) {
                Ok(declaration) => declared.push(declaration),
                Err(reason) => {
                    log::warn!(
                        area = "keyboard",
                        shortcut = name.as_str(),
                        reason = %reason,
                        "the shortcuts were refused; the previous bindings stay in force"
                    );
                    return;
                }
            }
            if let Some(argv) = &shortcut.command {
                commands.insert(name.clone(), argv.clone());
            }
        }
        let next = match self.keyboard_tables.router.bindings().reconcile(&declared) {
            Ok(next) => next,
            Err(conflict) => {
                log::warn!(
                    area = "keyboard",
                    code = conflict.code().as_str(),
                    conflict = %conflict,
                    "the shortcuts were refused; the previous bindings stay in force"
                );
                return;
            }
        };
        *self.keyboard_tables.router.bindings_mut() = next;
        self.keyboard_tables.shortcut_commands = commands;
        log::debug!(area = "keyboard", shortcuts = declared.len(), "the shortcuts changed");
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
        // A virtual keyboard's keymap is the seat's until that keyboard releases it.
        if self.keyboard_tables.keymap_owner != KeymapOwner::Layout {
            return;
        }
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
            Ok(()) => log::debug!(area = "keyboard", layout = %source.layout,
                variant = %source.variant,
                "the keymap changed"
            ),
            // The previous keymap stays installed: a session with a stale layout is usable
            // and one with no keymap is not.
            Err(error) => log::warn!(area = "keyboard", ?error, layout = %source.layout, "the keymap was refused"),
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

    /// Makes a new virtual keyboard and returns its id. It holds no keymap until it sends
    /// one, so its keys are refused before then.
    pub(crate) fn new_virtual_keyboard(&mut self) -> u64 {
        let id = self.keyboard_tables.next_virtual_id;
        self.keyboard_tables.next_virtual_id += 1;
        id
    }

    /// Compiles the keymap a virtual keyboard sent into the seat, and makes it the seat's
    /// keymap. A keymap xkbcommon refuses is released instead, see
    /// [`State::release_virtual_keymap`].
    pub(crate) fn set_virtual_keymap(&mut self, id: u64, text: String) {
        let handle = self.keyboard.clone();
        match handle.set_keymap_from_string(self, text.clone()) {
            Ok(()) => {
                self.keyboard_tables.virtual_keymaps.insert(id, text);
                self.keyboard_tables.keymap_owner = KeymapOwner::Virtual(id);
                log::debug!(
                    area = "keyboard",
                    keyboard = id,
                    "a virtual keyboard's keymap is the seat's"
                );
            }
            Err(error) => {
                log::warn!(
                    area = "keyboard",
                    ?error,
                    keyboard = id,
                    "a virtual keyboard's keymap was refused"
                );
                self.release_virtual_keymap(id);
            }
        }
    }

    /// Forgets a virtual keyboard's keymap. If it was the seat's, the seat gets its layout
    /// keymap back, so the keys the keyboard leaves behind are not read with its map.
    pub(crate) fn release_virtual_keymap(&mut self, id: u64) {
        self.keyboard_tables.virtual_keymaps.remove(&id);
        if self.keyboard_tables.keymap_owner != KeymapOwner::Virtual(id) {
            return;
        }
        self.keyboard_tables.keymap_owner = KeymapOwner::Layout;
        if self.keyboard_tables.sources.is_empty() {
            let handle = self.keyboard.clone();
            if let Err(error) = handle.set_xkb_config(self, XkbConfig::default()) {
                log::warn!(area = "keyboard", ?error, "the default keymap was refused");
            }
        } else {
            self.rebuild_keymap();
        }
    }

    /// Makes a virtual keyboard's keymap the seat's when another keymap holds the seat.
    /// `false` means the keyboard has no keymap, so its key is refused.
    pub(crate) fn use_virtual_keymap(&mut self, id: u64) -> bool {
        if self.keyboard_tables.keymap_owner != KeymapOwner::Virtual(id)
            && let Some(text) = self.keyboard_tables.virtual_keymaps.get(&id).cloned()
        {
            self.set_virtual_keymap(id, text);
        }
        self.keyboard_tables.keymap_owner == KeymapOwner::Virtual(id)
    }

    /// Sets the modifier state a virtual keyboard reports to the seat.
    pub(crate) fn set_modifier_state(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        let handle = self.keyboard.clone();
        handle.with_xkb_state(self, |mut context| {
            context.set_modifier_mask(depressed, latched, locked, group);
        });
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

    /// Applies the seat `claim` and `unclaim` calls the shell's transactions made. A claim binds its
    /// chord in the router, so the router delivers it before any client; an unclaim removes it.
    pub(crate) fn apply_claims(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let Some(&id) = call.args.first() else {
                log::warn!(area = "claim", method = %call.method, "a seat claim call has no id");
                continue;
            };
            let name = claim_name(call.node, id);
            let Ok(bound) = Name::new(&name) else {
                log::warn!(area = "claim", claim = %name, "a seat claim name is not a valid binding name");
                continue;
            };
            // A claim of an id already held replaces that claim.
            self.keyboard_tables.router.bindings_mut().remove_name(&bound);
            self.keyboard_tables.claims.remove(&name);
            if call.method == "unclaim" {
                continue;
            }
            let chord = match Chord::parse(&call.text, ChordKind::Key, &SeatKeysyms) {
                Ok(chord) => chord,
                Err(_) => {
                    log::warn!(area = "claim", chord = %call.text, "a seat claim's chord does not parse");
                    continue;
                }
            };
            let text = chord.to_string();
            match self.keyboard_tables.router.bind(&bound, chord, Trigger::Press, false) {
                Ok(_) => {
                    self.keyboard_tables.claims.insert(
                        name,
                        Claim {
                            node: call.node,
                            id,
                            chord: text,
                        },
                    );
                }
                Err(_) => log::warn!(area = "claim", chord = %call.text, "a seat claim's chord is already bound"),
            }
        }
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

    /// Delivers one key edge from a virtual keyboard. A modal that stands takes it, as it
    /// takes a physical key in `input`; otherwise the edge goes through [`Self::dispatch_key`].
    pub(crate) fn deliver_virtual_key(&mut self, keycode: u32, state: KeyState, serial: Serial, time: InputTime) {
        let pressed = state == KeyState::Pressed;
        if self.active_modal().is_some() && self.route_to_modal(&InputEvent::Key { keycode, pressed }) {
            return;
        }
        self.dispatch_key(keycode, state, serial, time);
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
            log::debug!(area = "key", keycode, target = %routed.target, fired = ?routed.fired, "consumed");
        }
        // A claimed chord reports both of its edges to the shell that claimed it.
        if let Target::Binding(bound) = routed.target
            && let Some(name) = self
                .keyboard_tables
                .router
                .bindings()
                .get(bound)
                .map(|binding| binding.name().as_str().to_owned())
            && let Some(claim) = self.keyboard_tables.claims.get(&name).cloned()
        {
            let state = match state {
                KeyState::Pressed => ChordState::Press,
                KeyState::Released => ChordState::Release,
            };
            self.tree.chord(
                claim.node,
                ChordEvent {
                    id: claim.id,
                    chord: claim.chord,
                    state,
                },
            );
        }
        // A press that fires a configured shortcut runs its command. A fired binding is
        // reported only on the edge that runs it, so the release of the same chord does nothing.
        if let Some(fired) = routed.fired
            && let Some(name) = self
                .keyboard_tables
                .router
                .bindings()
                .get(fired)
                .map(|binding| binding.name().as_str().to_owned())
        {
            if let Some(argv) = self.keyboard_tables.shortcut_commands.get(&name) {
                spawn_shortcut_command(&name, argv);
            } else if name == WINDOW_MENU_BINDING {
                self.open_focused_window_menu();
            }
        }
        consumed
    }

    /// Opens the window menu for the focused window from the keyboard.
    ///
    /// With no focused window there is nothing to anchor to, so nothing is emitted. The
    /// anchor is the window's top-left in output coordinates, which is where the keyboard
    /// default places the menu.
    fn open_focused_window_menu(&mut self) {
        let Some(window) = self.wm.focused() else {
            log::debug!(area = "menu", "the window-menu key was pressed with no focused window");
            return;
        };
        let Some(geometry) = self.geometry_of(window) else {
            log::debug!(
                area = "menu",
                ?window,
                "the focused window has no geometry for its menu"
            );
            return;
        };
        let anchor = MenuAnchor::point(geometry.x, geometry.y);
        self.request_window_menu(window, anchor, MenuSource::Keyboard);
    }

    /// Whether the focused client holds a shortcut inhibitor.
    fn focused_shortcuts_inhibited(&mut self) -> bool {
        let seat = self.seat.clone();
        self.shortcuts_inhibited(&seat)
    }
}
