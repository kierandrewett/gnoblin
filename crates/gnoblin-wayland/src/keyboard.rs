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

use std::collections::{BTreeMap, BTreeSet};

use smithay::backend::input::{InputTime, KeyState as SmithayKeyState};
use smithay::input::keyboard::Keysym as SmithayKeysym;
use smithay::input::keyboard::{KeyboardSource, Keycode, ModifiersState, XkbConfig, xkb};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;

use gnoblin_backend::{DeviceInfo, InputEvent};
use gnoblin_input::{
    Chord, ChordKind, DeviceKey, DeviceSelector, Event as RouterEvent, KeyState as RouterKeyState, Keysyms, Name,
    Router, ShortcutInhibit, Symbol, Target, Trigger,
};
use gnoblin_lock::State as LockState;
use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{DeviceId, KeyCode, KeyState, Keysym, Modifiers, NodeId, WindowId};

use crate::State;
use crate::devices::{DeviceKeys, LOCKS};
use crate::observation::{MenuAnchor, MenuSource};
use crate::tree_frame::{ChordEvent, ChordState, TreeInput};

use crate::log;

/// One key edge as the seat hands it to the router: the keyboard it came from, what that keyboard alone makes of
/// it, the `evdev` key code and whether it is a press or a release.
#[derive(Clone, Copy)]
struct SeatEdge {
    device: DeviceId,
    own: Option<DeviceKey>,
    keycode: u32,
    state: KeyState,
}

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
    /// The claim whose press the shell was told of, by the keyboard and keycode that pressed it, until the
    /// release is reported. Two keyboards holding one keycode are two presses.
    pub(crate) pressed_claims: BTreeMap<(DeviceId, u32), Claim>,
    /// The input devices the backend announced, in arrival order. `Seat.device` lists them.
    pub(crate) devices: Vec<DeviceInfo>,
    /// Each keyboard's own key state, which a binding scoped to that keyboard matches.
    pub(crate) device_keys: DeviceKeys,
    /// Which keymap the seat holds, counted up each time it changes, so a keyboard's own state knows
    /// when it was built from an older one.
    pub(crate) keymap_generation: u64,
    /// The seat's name for each keyboard, which tells one keyboard's hold of a key from another's.
    pub(crate) key_sources: BTreeMap<DeviceId, KeyboardSource>,
    /// The key codes a client was told are down, so the seat's last release of one is told to it too.
    pub(crate) forwarded_keys: BTreeSet<u32>,
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
            pressed_claims: BTreeMap::new(),
            devices: Vec::new(),
            device_keys: DeviceKeys::new(),
            keymap_generation: 0,
            key_sources: BTreeMap::new(),
            forwarded_keys: BTreeSet::new(),
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
        device: None,
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
        self.keymap_changed();
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
            && let Some(window) = self.focus
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
        self.keymap_changed();
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
            self.keymap_changed();
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
        if let Some(window) = self.keyboard_focus_window() {
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

    /// Applies the seat `claim`, `claim_gesture` and `unclaim` calls the shell's transactions made. A
    /// claim binds its chord in the router, so the router delivers it before any client; a gesture claim
    /// takes its gesture from the touchpad bindings; an unclaim removes either.
    pub(crate) fn apply_claims(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let Some(&id) = call.args.first() else {
                log::warn!(area = "claim", method = %call.method, "a seat claim call has no id");
                continue;
            };
            if call.method == "claim_gesture" {
                self.claim_gesture(&call, id);
                continue;
            }
            if call.method == "unclaim" {
                self.touchpad_gestures.claims_mut().unclaim(call.node.get(), id);
            }
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
            // The text is the chord, or the device a claim is scoped to, a line feed and then the chord.
            let (selector, subject) = crate::devices::split_selector(call.text.expose());
            let device = match selector.map(DeviceSelector::new).transpose() {
                Ok(device) => device,
                Err(error) => {
                    log::warn!(area = "claim", %error, "a seat claim's device selector is refused");
                    continue;
                }
            };
            let chord = match Chord::parse(subject, ChordKind::Key, &SeatKeysyms) {
                Ok(chord) => chord,
                Err(_) => {
                    // The chord is not logged: call text is a generic secret-capable argument.
                    log::warn!(area = "claim", "a seat claim's chord does not parse");
                    continue;
                }
            };
            let text = chord.to_string();
            match self
                .keyboard_tables
                .router
                .bind_for(&bound, chord, Trigger::Press, false, device)
            {
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
                Err(_) => log::warn!(area = "claim", "a seat claim's chord is already bound for that device"),
            }
        }
    }

    /// Releases the claims the destroyed nodes held. A destroyed seat takes its claims with it, so
    /// its chords stop firing and reach the client again.
    pub(crate) fn release_seat_nodes(&mut self, gone: &[NodeId]) {
        let dropped: Vec<String> = self
            .keyboard_tables
            .claims
            .iter()
            .filter(|(_, claim)| gone.contains(&claim.node))
            .map(|(name, _)| name.clone())
            .collect();
        for name in dropped {
            if let Ok(bound) = Name::new(&name) {
                self.keyboard_tables.router.bindings_mut().remove_name(&bound);
            }
            self.keyboard_tables.claims.remove(&name);
        }
        let holders: Vec<u64> = gone.iter().map(|node| node.get()).collect();
        self.touchpad_gestures.claims_mut().release(&holders);
        self.keyboard_tables
            .pressed_claims
            .retain(|_, claim| !gone.contains(&claim.node));
    }

    /// The seat claim behind a router target, when the target is a claim.
    fn claim_of(&self, target: Target) -> Option<Claim> {
        let Target::Binding(bound) = target else {
            return None;
        };
        let name = self
            .keyboard_tables
            .router
            .bindings()
            .get(bound)?
            .name()
            .as_str()
            .to_owned();
        self.keyboard_tables.claims.get(&name).cloned()
    }

    /// Reports one edge of a claimed chord to the shell that claimed it, and says whether the client
    /// receives the edge. `None` means the edge is not a claim, so the caller keeps its own answer.
    ///
    /// A press reported to the shell is remembered by the keyboard and keycode that made it, so its release is
    /// reported even when the lock came down between the two edges, and another keyboard's release of the same
    /// keycode is not taken for it. A press the lock held back went to the client, so its release does too.
    fn route_claim_edge(&mut self, device: DeviceId, keycode: u32, state: KeyState, target: Target) -> Option<bool> {
        match state {
            KeyState::Pressed => {
                let claim = self.claim_of(target)?;
                if self.lock.state() != LockState::Unlocked {
                    return Some(false);
                }
                self.tree.chord(
                    claim.node,
                    ChordEvent {
                        id: claim.id,
                        chord: claim.chord.clone(),
                        state: ChordState::Press,
                    },
                );
                self.keyboard_tables.pressed_claims.insert((device, keycode), claim);
                Some(true)
            }
            KeyState::Released => {
                if let Some(claim) = self.keyboard_tables.pressed_claims.remove(&(device, keycode)) {
                    self.tree.chord(
                        claim.node,
                        ChordEvent {
                            id: claim.id,
                            chord: claim.chord,
                            state: ChordState::Release,
                        },
                    );
                    return Some(true);
                }
                self.claim_of(target).map(|_| false)
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
    /// the router, and only then handed to `wl_keyboard`. `device` is the keyboard the key is on, or
    /// [`DeviceId::VIRTUAL`] for a key no announced keyboard produced.
    ///
    /// Two states are kept apart. The seat holds one key state for every keyboard: a key is down once it is
    /// down on any keyboard and up once it is up on all of them, and only those two transitions reach the
    /// seat's modifiers and a client. The router keeps a press and its release per keyboard and key code.
    /// So every edge reaches the router, including a second keyboard's press of a key the first still holds,
    /// because a binding scoped to the second keyboard must fire on it; and a client is told of the seat's
    /// transitions only, so it never sees a press twice or a release while another keyboard holds the key.
    /// A key the client was told is down is released to it by the seat's last release, whichever keyboard's
    /// and whatever the router made of that release, so a claim on the last keyboard cannot leave it down.
    pub(crate) fn dispatch_key(
        &mut self,
        device: DeviceId,
        keycode: u32,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
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
        let seat_code: Keycode = keycode.saturating_add(SEAT_KEYCODE_OFFSET).into();
        let edge = SeatEdge {
            device,
            own: self.device_key(device, keycode, state),
            keycode,
            state,
        };
        let source = self.key_source(device);
        let seat_edge = keyboard.input_intercept(source, self, seat_code, key_state, |data, modifiers, symbol| {
            data.route_key_edge(edge, modifiers, symbol.modified_sym().raw(), false)
        });
        let Some((consumed, modifiers_changed)) = seat_edge else {
            // Another keyboard holds the key, or still does: the seat's state is what it was. The router still
            // takes the edge, under the seat's modifiers and the symbol the key has in them.
            let modifiers = keyboard.modifier_state();
            // The key is held by the other keyboard, so the seat has a handle for it. One the seat does not hold
            // (a release nothing pressed) has no symbol.
            let symbol = keyboard.with_pressed_keysyms(|held| {
                held.iter()
                    .find(|handle| handle.raw_code() == seat_code)
                    .map_or(0, |handle| handle.modified_sym().raw())
            });
            self.route_key_edge(edge, &modifiers, symbol, true);
            return;
        };
        let forward = match state {
            KeyState::Pressed => !consumed,
            KeyState::Released => self.keyboard_tables.forwarded_keys.remove(&keycode),
        };
        if forward {
            if state == KeyState::Pressed {
                self.keyboard_tables.forwarded_keys.insert(keycode);
            }
            keyboard.input_forward(self, seat_code, key_state, serial, time, modifiers_changed);
        }
    }

    /// The seat's name for the keyboard `device`, so the seat can tell one keyboard's hold of a key from another's.
    fn key_source(&mut self, device: DeviceId) -> KeyboardSource {
        *self
            .keyboard_tables
            .key_sources
            .entry(device)
            .or_insert_with(KeyboardSource::new_auxiliary)
    }

    /// Lets go of every key a keyboard that left still held, so no other keyboard's press of one is taken for
    /// the held key and a client is told of each release the seat itself ends.
    pub(crate) fn release_keyboard(&mut self, device: DeviceId) {
        let Some(source) = self.keyboard_tables.key_sources.remove(&device) else {
            return;
        };
        let keyboard = self.keyboard.clone();
        let before = keyboard.pressed_keys();
        keyboard.release_source(self, source);
        let after = keyboard.pressed_keys();
        for code in before.difference(&after) {
            self.keyboard_tables
                .forwarded_keys
                .remove(&code.raw().saturating_sub(SEAT_KEYCODE_OFFSET));
        }
    }

    /// What the keyboard `device` alone makes of one edge of its own, from its own key state.
    ///
    /// `None` for a key no announced keyboard produced, which has no state of its own to read.
    fn device_key(&mut self, device: DeviceId, keycode: u32, state: KeyState) -> Option<DeviceKey> {
        if !self.keyboard_tables.devices.iter().any(|known| known.id == device) {
            return None;
        }
        let generation = self.keyboard_tables.keymap_generation;
        if !self.keyboard_tables.device_keys.is_current(generation) {
            let keyboard = self.keyboard.clone();
            let text = keyboard.with_xkb_state(self, |context| {
                context
                    .xkb()
                    .lock()
                    .map(|xkb| xkb.keymap_as_string())
                    .unwrap_or_default()
            });
            self.keyboard_tables.device_keys.set_keymap(&text, generation);
        }
        self.keyboard_tables
            .device_keys
            .key(device, keycode, state == KeyState::Pressed)
    }

    /// Marks the seat's keymap as changed, so each keyboard's own state is rebuilt from the new one the next
    /// time it reads a key.
    pub(crate) fn keymap_changed(&mut self) {
        self.keyboard_tables.keymap_generation = self.keyboard_tables.keymap_generation.wrapping_add(1);
    }

    /// Delivers one key edge from a virtual keyboard. A modal that stands takes it, as it
    /// takes a physical key in `input`; otherwise the edge goes through [`Self::dispatch_key`].
    pub(crate) fn deliver_virtual_key(&mut self, keycode: u32, state: KeyState, serial: Serial, time: InputTime) {
        let pressed = state == KeyState::Pressed;
        if self.active_modal().is_some()
            && self.route_to_modal(&InputEvent::Key {
                device: DeviceId::VIRTUAL,
                keycode,
                pressed,
            })
        {
            return;
        }
        self.dispatch_key(DeviceId::VIRTUAL, keycode, state, serial, time);
    }

    /// The router's half of one edge: a binding or a grab consumes it, a client edge is
    /// forwarded.
    ///
    /// `own` is what the keyboard that sent the key says about it, from its own key state: a binding
    /// scoped to that keyboard matches it, and every other binding matches the seat's `modifiers` and `symbol`.
    /// A lock another keyboard toggled still applies to a scoped binding, because a lock is the seat's state
    /// and not a key this keyboard holds.
    ///
    /// `shared` is set when the seat's key state does not change with this edge: another keyboard holds the
    /// key this one pressed, or still holds the key this one released. The router sees the edge all the same,
    /// so each keyboard's press keeps its own release and a binding scoped to this keyboard fires. A binding
    /// for every keyboard is the seat's, and the seat's key was already down or stays down, so such an edge
    /// runs nothing and never reaches the shell's node.
    fn route_key_edge(&mut self, edge: SeatEdge, modifiers: &ModifiersState, symbol: u32, shared: bool) -> bool {
        let SeatEdge {
            device,
            own,
            keycode,
            state,
        } = edge;
        let inhibit = if self.focused_shortcuts_inhibited() {
            ShortcutInhibit::Inhibited
        } else {
            ShortcutInhibit::Clear
        };
        let seat_modifiers = modifiers_of(modifiers);
        let own = own.map(|own| DeviceKey {
            symbol: own.symbol,
            modifiers: own.modifiers.remove(LOCKS).union(seat_modifiers.intersect(LOCKS)),
        });
        let routed = self.keyboard_tables.router.route(
            RouterEvent::Key {
                device,
                key_code: KeyCode::new(keycode),
                symbol: Keysym::new(symbol),
                modifiers: seat_modifiers,
                own,
                state: match state {
                    KeyState::Pressed => RouterKeyState::Pressed,
                    KeyState::Released => RouterKeyState::Released,
                },
                repeat: false,
            },
            inhibit,
        );
        let mut consumed = routed.target != Target::Client;
        if consumed {
            log::debug!(area = "key", keycode, target = %routed.target, fired = ?routed.fired, "consumed");
        }
        // A shared edge of a binding for every keyboard is not the seat's. A release is still reported when the
        // press it ends was, so the shell is never left holding a chord.
        let acts = !shared || self.is_scoped_binding(routed.target);
        // A claimed chord reports both of its edges to the shell that claimed it, while the session
        // is unlocked. A locked session's cover owns every key, so the client gets the chord instead.
        if (acts || state == KeyState::Released)
            && let Some(answer) = self.route_claim_edge(device, keycode, state, routed.target)
        {
            consumed = answer;
        }
        // A key the router leaves to the client goes to the shell's node instead, when the shell holds
        // the seat focus on one that takes input. A locked session's cover owns every key, so none reach it.
        if !shared
            && !consumed
            && self.lock.state() == LockState::Unlocked
            && let Some(node) = self.composer.shell_key_target()
        {
            self.tree.input(
                node,
                TreeInput::Key {
                    pressed: state == KeyState::Pressed,
                    keysym: Keysym::new(symbol),
                },
            );
            consumed = true;
        }
        // A press that fires a configured shortcut runs its command. A fired binding is
        // reported only on the edge that runs it, so the release of the same chord does nothing.
        if acts
            && let Some(fired) = routed.fired
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

    /// Whether a router target is a binding that names one keyboard.
    fn is_scoped_binding(&self, target: Target) -> bool {
        let Target::Binding(id) = target else {
            return false;
        };
        self.keyboard_tables
            .router
            .bindings()
            .get(id)
            .is_some_and(|binding| binding.device().is_some())
    }

    /// Opens the window menu for the focused window from the keyboard.
    ///
    /// With no focused window there is nothing to anchor to, so nothing is emitted. The
    /// anchor is the window's top-left in output coordinates, which is where the keyboard
    /// default places the menu.
    fn open_focused_window_menu(&mut self) {
        let Some(window) = self.focus else {
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
