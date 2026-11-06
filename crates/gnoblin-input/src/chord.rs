//! The one chord grammar for every shortcut mechanism.
//!
//! A chord is zero or more modifiers in angle brackets, then one key:
//! `<Super><Alt>F9`, `<Control>q`, `XF86AudioMute`, and the bare word `Super`.
//! [`docs/guides/shortcuts.md`](../../../docs/guides/shortcuts.md) publishes the
//! syntax and `src/control/gnoblin-chord.h` is the C implementation it replaces;
//! this module keeps the same accepted language, so a chord written for a Mutter
//! build means the same thing to the Rust compositor.
//!
//! # Accepted forms
//!
//! Modifiers, in angle brackets, any order, any case, each at most once:
//!
//! | Written | Means | Notes |
//! | --- | --- | --- |
//! | `<Shift>`, `<Control>`, `<Alt>`, `<Meta>`, `<Super>`, `<Hyper>` | that modifier | the six Gnoblin accepts |
//! | `<Shft>` | `<Shift>` | legacy GTK abbreviation |
//! | `<Ctrl>`, `<Ctl>`, `<Primary>` | `<Control>` | legacy GTK abbreviations |
//! | `<Mod1>` | `<Alt>` | the only X11 mask name with a portable meaning |
//! | `<Mod2>`, `<Mod3>`, `<Mod4>`, `<Mod5>` | nothing | refused: the key behind the mask depends on the keymap |
//!
//! The key after the modifiers is one of:
//!
//! | Form | Example | Notes |
//! | --- | --- | --- |
//! | an XKB keysym name | `Return`, `F9`, `q`, `XF86AudioMute` | resolved through [`Keysyms`] |
//! | a keysym name without the `XF86` prefix | `AudioMute` | same identity as `XF86AudioMute` |
//! | the key above Tab | `Above_Tab` | layout independent |
//! | a hardware key code | `0x3f` | exactly two hex digits, `0x01` to `0xff` |
//! | a pointer button | `Button1`, `Button2`, `Button3` | [`ChordKind::Pointer`] only, needs a modifier |
//!
//! The bare word `Super`, with no angle brackets, is a chord only for
//! [`ChordKind::Key`]: Super pressed and released with no other key between. A
//! mechanism that does not implement that gesture must not accept the chord, which
//! is why the grammar refuses it for pointer chords.
//!
//! # Refused forms
//!
//! Everything else is an error that names the form to write. Notably: plus signs
//! and spaces (`Super+Alt+F9`), an unknown `<Windows>` modifier, a modifier after
//! the key, two keys, an empty `<>`, a duplicate modifier, a trailing modifier with
//! no key, a key code outside `0x01` to `0xff`, and a pointer chord with no
//! modifier.
//!
//! **There is no sequence syntax.** The published syntax says a chord is one key,
//! so `<Super>a<Super>b` is refused as two keys rather than read as two chords.
//! Sequences would need their own grammar (a separator, a timeout, a partial-match
//! state machine) and their own documented semantics; inventing one here would
//! make the written form ambiguous with the modifiers-plus-key form.
//!
//! # Identity and written form
//!
//! A chord has one identity and one written form, and both come from this module:
//!
//! - Modifiers compare as a set, so `<Alt><Super>` and `<Super><Alt>` are one chord.
//! - Aliases compare as the modifier they name, so `<Ctrl>` and `<Control>` are one
//!   chord.
//! - Key names compare by keysym, so `AudioMute` and `XF86AudioMute` are one chord
//!   and `q` written as `Q` is the chord whose keysym the table resolves it to.
//! - The written form orders modifiers `<Shift><Control><Alt><Meta><Super><Hyper>`
//!   and uses the table's canonical key name, so writing a chord back and parsing
//!   it again yields the same chord and the same text.
//!
//! Two spellings therefore never become two bindings that shadow each other: the
//! configuration can only ever compare canonical chords.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// The longest accepted chord, in bytes.
pub const MAX_CHORD_LENGTH: usize = 128;

/// The longest accepted key name, in bytes. Matches the C `key[64]` field.
pub const MAX_KEY_NAME_LENGTH: usize = 64;

/// The example every error message ends with, for keyboard chords.
pub const CHORD_EXAMPLE: &str = "<Super><Alt>F9";

/// The example every error message ends with, for pointer chords.
pub const POINTER_CHORD_EXAMPLE: &str = "<Super>Button1";

/// How a chord is read.
///
/// The two kinds are separate languages on purpose: a keyboard chord that ended in
/// `Button1` and a pointer chord that ended in `q` are both mistakes, and each gets
/// a message that names the form to write instead of a silently different binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChordKind {
    /// Modifiers and one key.
    Key,
    /// Modifiers and one pointer button.
    Pointer,
}

/// Per-kind wording, so a keyboard error and a pointer error each name their own example.
///
impl ChordKind {
    /// The example chord written in error messages for this kind.
    pub const fn example(self) -> &'static str {
        match self {
            Self::Key => CHORD_EXAMPLE,
            Self::Pointer => POINTER_CHORD_EXAMPLE,
        }
    }
}

/// A set of modifier keys, in the one layout `gnoblin-types` documents.
///
/// Re-exported from `gnoblin-types` rather than declared here: an event mask the
/// router compares a chord against is built by the mechanism layer from the same
/// bit layout, so a chord and an event cannot disagree about which bit Alt is.
pub use gnoblin_types::Modifiers;

/// The order every written chord uses, and the order the error suggestions use.
const WRITTEN_ORDER: [Modifiers; 6] = [
    Modifiers::SHIFT,
    Modifiers::CONTROL,
    Modifiers::ALT,
    Modifiers::META,
    Modifiers::SUPER,
    Modifiers::HYPER,
];

/// The name a single-modifier set is written as, or an empty string otherwise.
fn written_name(modifier: Modifiers) -> &'static str {
    match modifier {
        Modifiers::SHIFT => "Shift",
        Modifiers::CONTROL => "Control",
        Modifiers::ALT => "Alt",
        Modifiers::META => "Meta",
        Modifiers::SUPER => "Super",
        Modifiers::HYPER => "Hyper",
        _ => "",
    }
}

/// An XKB keysym value, from `gnoblin-types`.
///
/// Shared with the accessibility machines: a key a chord names and a key the
/// screen reader grabs are the same symbol, so they are the same type.
pub use gnoblin_types::Keysym;

/// A keysym together with the name Gnoblin writes it as.
///
/// Equality, ordering and hashing use the keysym only, so a table that writes one
/// key two ways cannot split a chord into two. The name is presentation.
#[derive(Clone, Debug)]
pub struct Symbol {
    symbol: Keysym,
    name: String,
}

/// Constructors and accessors. The name is kept only for display and never compared.
///
impl Symbol {
    /// Build a symbol from a keysym and its canonical name.
    pub fn new(symbol: Keysym, name: &str) -> Self {
        Self {
            symbol,
            name: name.to_owned(),
        }
    }

    /// The keysym.
    pub const fn symbol(&self) -> Keysym {
        self.symbol
    }

    /// The canonical name, as written in a chord.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Identity is the keysym alone, so a name written two ways is one symbol.
impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
    }
}

impl Eq for Symbol {}

/// Hashes the keysym only, matching [`PartialEq`] so equal symbols hash equally.
impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
    }
}

/// Orders by keysym only, consistent with [`PartialEq`] and [`Ord`].
impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        self.symbol.cmp(&other.symbol)
    }
}

/// A hardware key code, from `gnoblin-types`.
///
/// The grammar accepts the `0x01` to `0xff` range only (a written chord names a key
/// a person can write in two hex digits), while the type holds libinput's 32-bit
/// codes so the router carries a key past 255 unchanged.
pub use gnoblin_types::KeyCode;

/// The key part of a chord.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A keysym and its canonical name.
    Symbol(Symbol),
    /// A hardware key code, from `0x3f`.
    KeyCode(KeyCode),
    /// The key above Tab, whatever the layout calls it.
    AboveTab,
    /// A pointer button, 1 to 3.
    Button(u8),
    /// The bare word `Super`: Super pressed and released alone.
    BareSuper,
}

/// The written form of a key, in the form a chord would spell it.
impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symbol(symbol) => formatter.write_str(symbol.name()),
            Self::KeyCode(code) => write!(formatter, "0x{:02x}", code.get()),
            Self::AboveTab => formatter.write_str("Above_Tab"),
            Self::Button(button) => write!(formatter, "Button{button}"),
            Self::BareSuper => formatter.write_str("Super"),
        }
    }
}

/// One parsed chord.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    modifiers: Modifiers,
    key: Key,
}

/// Parsing, and the accessors that matching uses.
///
impl Chord {
    /// Parse `text` as a chord of `kind`.
    ///
    /// Key names are resolved through `keysyms`. Everything else is decided here,
    /// so two calls with two tables that agree on a key always agree on the chord.
    pub fn parse(text: &str, kind: ChordKind, keysyms: &dyn Keysyms) -> Result<Self, ParseError> {
        let fail = |reason: Reason| ParseError {
            text: text.to_owned(),
            kind,
            reason,
        };

        if text.is_empty() {
            return Err(fail(Reason::Empty));
        }
        if text.len() > MAX_CHORD_LENGTH {
            return Err(fail(Reason::TooLong { length: text.len() }));
        }
        if text
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(fail(Reason::Whitespace));
        }
        // `Super` alone has no angle brackets, so it is recognised before the bracket loop
        // would look for a modifier that is not there.
        if kind == ChordKind::Key && text == "Super" {
            return Ok(Self {
                modifiers: Modifiers::SUPER,
                key: Key::BareSuper,
            });
        }

        let mut modifiers = Modifiers::NONE;
        let mut rest = text;
        while let Some(stripped) = rest.strip_prefix('<') {
            let Some(end) = stripped.find('>') else {
                return Err(fail(Reason::UnclosedModifier));
            };
            let written = &stripped[..end];
            if written.is_empty() {
                return Err(fail(Reason::EmptyModifier));
            }
            let lowered = written.to_ascii_lowercase();
            let Some((modifier, hint)) = modifier_named(&lowered) else {
                return Err(fail(Reason::UnknownModifier {
                    name: written.to_owned(),
                }));
            };
            if let Some(hint) = hint {
                return Err(fail(Reason::UnportableModifier {
                    name: written.to_owned(),
                    hint,
                }));
            }
            if modifiers.contains(modifier) {
                return Err(fail(Reason::DuplicateModifier {
                    name: written.to_owned(),
                }));
            }
            modifiers = modifiers.union(modifier);
            rest = &stripped[end + 1..];
        }

        if rest.is_empty() {
            return Err(fail(Reason::MissingKey));
        }
        if rest.contains('<') || rest.contains('>') {
            return Err(fail(Reason::MoreThanOneKey));
        }

        // Pointer chords use their own button table, so a key name never reaches the
        // pointer path and a button name never reaches the keyboard path.
        if kind == ChordKind::Pointer {
            let Some(button) = button_named(rest) else {
                return Err(fail(Reason::NotAButton));
            };
            if modifiers.is_empty() {
                return Err(fail(Reason::PointerNeedsModifier));
            }
            return Ok(Self {
                modifiers,
                key: Key::Button(button),
            });
        }

        // Above_Tab is layout independent, so it is matched by name before keysym
        // resolution. A table lookup would make its meaning depend on the keymap.
        if rest.eq_ignore_ascii_case("above_tab") {
            return Ok(Self {
                modifiers,
                key: Key::AboveTab,
            });
        }
        // Key codes are tried before keysyms: `0x3f` is not a keysym name, and the table
        // should never be asked to resolve hexadecimal text.
        if let Some(code) = key_code(rest) {
            if code == 0 {
                return Err(fail(Reason::ZeroKeyCode));
            }
            return Ok(Self {
                modifiers,
                key: Key::KeyCode(KeyCode::new(u32::from(code))),
            });
        }
        // Length is checked after the cheap forms and before the table, so an over-long
        // name never reaches the keysym table.
        if rest.len() > MAX_KEY_NAME_LENGTH {
            return Err(fail(Reason::KeyNameTooLong { length: rest.len() }));
        }
        let Some(symbol) = resolve(keysyms, rest) else {
            return Err(fail(Reason::UnknownKey { name: rest.to_owned() }));
        };
        Ok(Self {
            modifiers,
            key: Key::Symbol(symbol),
        })
    }

    /// The modifiers held by this chord.
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The key part.
    pub const fn key(&self) -> &Key {
        &self.key
    }

    /// Whether this is a pointer-button chord.
    pub const fn is_pointer(&self) -> bool {
        matches!(self.key, Key::Button(_))
    }

    /// Whether this chord is the bare `Super` chord.
    ///
    /// Such a chord is never matched by a key or button edge: it fires when Super
    /// is pressed and released with nothing in between, which is shortcut-session
    /// state rather than a single event.
    pub const fn is_bare_super(&self) -> bool {
        matches!(self.key, Key::BareSuper)
    }

    /// Whether a key event with this keysym and exactly these modifiers is this chord.
    ///
    /// The modifier set matches exactly, as a GTK accelerator does: `<Super>q` fires
    /// while Super is held and not while Super and Shift are.
    pub fn matches_key(&self, symbol: Keysym, modifiers: Modifiers) -> bool {
        match &self.key {
            Key::Symbol(expected) => expected.symbol() == symbol && self.modifiers == modifiers,
            _ => false,
        }
    }

    /// Whether a button event with this number and exactly these modifiers is this chord.
    pub fn matches_button(&self, button: u8, modifiers: Modifiers) -> bool {
        match self.key {
            Key::Button(expected) => expected == button && self.modifiers == modifiers,
            _ => false,
        }
    }
}

/// The written form: the bare `Super`, or the modifiers in [`WRITTEN_ORDER`] then the key.
impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_bare_super() {
            return formatter.write_str("Super");
        }
        for modifier in WRITTEN_ORDER {
            if self.modifiers.contains(modifier) {
                write!(formatter, "<{}>", written_name(modifier))?;
            }
        }
        write!(formatter, "{}", self.key)
    }
}

/// The key-name table the grammar resolves written key names against.
///
/// The grammar knows the syntax, the modifier aliases, the key-code and button
/// forms and the canonical written order, but not which keysyms exist: that depends
/// on xkbcommon. The mechanism layer implements this trait over
/// `xkb_keysym_from_name(name, XKB_KEYSYM_CASE_INSENSITIVE)`, which is what makes a
/// chord resolve to the same key as the C tree and as Mutter.
///
/// An implementation must be case-insensitive, as xkbcommon is, and must return one
/// canonical name per keysym: two writings of one key that returned two names would
/// give the chord two written forms.
pub trait Keysyms {
    /// The keysym `name` means, with the name to write it as, or `None`.
    fn resolve(&self, name: &str) -> Option<Symbol>;
}

/// A refused chord, with the text that was refused and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    text: String,
    kind: ChordKind,
    reason: Reason,
}

impl ParseError {
    /// The refused text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The chord kind the text was read as.
    pub const fn kind(&self) -> ChordKind {
        self.kind
    }

    /// Why it was refused.
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// The message to show the user: what is wrong and the form to write instead.
    pub fn message(&self) -> String {
        match suggest(&self.text, self.kind) {
            Some(suggestion) => format!(
                "\"{}\" is not a chord: put each modifier in angle brackets before the key. Write {suggestion}",
                self.text
            ),
            None => format!(
                "\"{}\" is not a chord: {}. Write modifiers in angle brackets before one key, for example {}",
                self.text,
                self.reason.explain(),
                self.kind.example()
            ),
        }
    }
}

/// Displays the same message as [`ParseError::message`].
impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ParseError {}

/// Why a chord was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The text is empty.
    Empty,
    /// The text is longer than [`MAX_CHORD_LENGTH`] bytes.
    TooLong {
        /// The length of the refused text.
        length: usize,
    },
    /// The text contains a space or control character. A chord has no spaces.
    Whitespace,
    /// A modifier opened with `<` and never closed.
    UnclosedModifier,
    /// An empty `<>`.
    EmptyModifier,
    /// A name in angle brackets that is not a modifier.
    UnknownModifier {
        /// The name as written, without the brackets.
        name: String,
    },
    /// An X11 mask name whose key depends on the keymap.
    UnportableModifier {
        /// The name as written, without the brackets.
        name: String,
        /// What to write instead.
        hint: &'static str,
    },
    /// The same modifier twice.
    DuplicateModifier {
        /// The name as written, without the brackets.
        name: String,
    },
    /// Modifiers and no key.
    MissingKey,
    /// More than one key, or a modifier after the key.
    MoreThanOneKey,
    /// A pointer chord that does not end in `Button1`, `Button2` or `Button3`.
    NotAButton,
    /// A pointer chord with no modifier, which would swallow plain clicks.
    PointerNeedsModifier,
    /// A key name the table does not know.
    UnknownKey {
        /// The name as written.
        name: String,
    },
    /// A key name longer than [`MAX_KEY_NAME_LENGTH`] bytes.
    KeyNameTooLong {
        /// The length of the refused name.
        length: usize,
    },
    /// The key code `0x00`, which no key has.
    ZeroKeyCode,
}

/// One clause per refusal, used in the long message.
///
impl Reason {
    /// One clause describing the refusal.
    fn explain(&self) -> String {
        match self {
            Self::Empty => "it is empty".to_owned(),
            Self::TooLong { length } => {
                format!("it is {length} bytes; the limit is {MAX_CHORD_LENGTH}")
            }
            Self::Whitespace => "it has a space or control character".to_owned(),
            Self::UnclosedModifier => "a modifier has no closing >".to_owned(),
            Self::EmptyModifier => "it has an empty <> modifier".to_owned(),
            Self::UnknownModifier { name } => format!(
                "<{name}> is not a modifier; the modifiers are <Shift>, <Control>, <Alt>, <Super>, <Hyper> and <Meta>"
            ),
            Self::UnportableModifier { hint, .. } => (*hint).to_owned(),
            Self::DuplicateModifier { name } => format!("<{name}> appears twice"),
            Self::MissingKey => "it has no key after the modifiers".to_owned(),
            Self::MoreThanOneKey => "it has more than one key, or a modifier after the key".to_owned(),
            Self::NotAButton => "a pointer chord ends with Button1, Button2 or Button3".to_owned(),
            Self::PointerNeedsModifier => "a pointer chord needs at least one modifier before its button".to_owned(),
            Self::UnknownKey { name } => {
                format!("\"{name}\" is not a key name; use an XKB keysym name such as Return, F9, q or XF86AudioMute")
            }
            Self::KeyNameTooLong { length } => {
                format!("its key name is {length} bytes; the limit is {MAX_KEY_NAME_LENGTH}")
            }
            Self::ZeroKeyCode => "0x00 is not a key code; key codes are 0x01 to 0xff".to_owned(),
        }
    }
}

/// The modifier a written name means, with the hint to give when it has none.
///
/// One table for the parser and for the suggestions: an alias cannot be accepted in
/// one place and unknown in the other.
fn modifier_named(lowered: &str) -> Option<(Modifiers, Option<&'static str>)> {
    let found = match lowered {
        "shift" | "shft" => (Modifiers::SHIFT, None),
        "control" | "ctrl" | "ctl" | "primary" => (Modifiers::CONTROL, None),
        "alt" | "mod1" => (Modifiers::ALT, None),
        "meta" => (Modifiers::META, None),
        "super" => (Modifiers::SUPER, None),
        "hyper" => (Modifiers::HYPER, None),
        "mod4" => (
            Modifiers::NONE,
            Some("<Mod4> is an X11 mask name whose key depends on the keymap; write <Super>"),
        ),
        "mod2" | "mod3" | "mod5" => (
            Modifiers::NONE,
            Some("<Mod2> to <Mod5> are X11 mask names whose key depends on the keymap; there is no portable form"),
        ),
        _ => return None,
    };
    Some(found)
}

/// The modifier a word outside angle brackets means, for the suggestion path.
fn modifier_word(lowered: &str) -> Option<Modifiers> {
    Some(match lowered {
        "shift" | "shft" => Modifiers::SHIFT,
        "control" | "ctrl" | "ctl" | "primary" => Modifiers::CONTROL,
        "alt" | "mod1" | "option" => Modifiers::ALT,
        "meta" => Modifiers::META,
        "super" | "win" | "windows" | "logo" | "cmd" | "command" => Modifiers::SUPER,
        "hyper" => Modifiers::HYPER,
        _ => return None,
    })
}

/// The button a written name means, for pointer chords.
fn button_named(text: &str) -> Option<u8> {
    let digits = text.strip_prefix("Button")?;
    if digits.len() != 1 {
        return None;
    }
    match digits.as_bytes()[0] {
        b'1'..=b'3' => Some(digits.as_bytes()[0] - b'0'),
        _ => None,
    }
}

/// The key code `0x3f` means, accepting either case of the `x`.
fn key_code(text: &str) -> Option<u8> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    if digits.len() != 2 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

/// The keysym a name means, retrying with the `XF86` prefix.
fn resolve(keysyms: &dyn Keysyms, name: &str) -> Option<Symbol> {
    if let Some(symbol) = keysyms.resolve(name) {
        return Some(symbol);
    }
    keysyms.resolve(&format!("XF86{name}"))
}

/// The chord a copy-pasted `Super+Alt+F9`, `ctrl-q` or `ctrl alt q` means, if any.
///
/// This exists only to turn a common mistake into a message that shows the form to
/// write. It never succeeds for text the parser would accept, because a chord with a
/// separator is not a chord at all.
fn suggest(text: &str, kind: ChordKind) -> Option<String> {
    let separator = if text.contains('+') {
        '+'
    } else if text.contains(' ') {
        ' '
    } else if text.contains('-') && text.len() > 1 {
        '-'
    } else {
        return None;
    };
    let parts: Vec<&str> = text.split(separator).map(str::trim).collect();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let (key, words) = parts.split_last()?;
    let mut modifiers = Modifiers::NONE;
    for word in words {
        modifiers = modifiers.union(modifier_word(&word.to_ascii_lowercase())?);
    }
    let key = if kind == ChordKind::Pointer {
        Key::Button(button_named(key)?)
    } else if key.eq_ignore_ascii_case("above_tab") {
        Key::AboveTab
    } else if let Some(code) = key_code(key) {
        if code == 0 {
            return None;
        }
        Key::KeyCode(KeyCode::new(u32::from(code)))
    } else {
        Key::Symbol(PLACEHOLDER_KEYSYMS.resolve(key)?)
    };
    Some(Chord { modifiers, key }.to_string())
}

/// A table that can name a key but not resolve one.
///
/// Suggestions only need the name to appear in the message, so this parses a
/// suggestion with no keysym table: the keysym it reports is unused because the
/// resulting chord is only ever formatted.
struct PlaceholderKeysyms;

/// Accepts any printable name, so suggestions can be formatted without xkbcommon.
///
impl Keysyms for PlaceholderKeysyms {
    fn resolve(&self, name: &str) -> Option<Symbol> {
        if name.is_empty() || name.len() > MAX_KEY_NAME_LENGTH {
            return None;
        }
        if !name.chars().all(|character| character.is_ascii_graphic()) {
            return None;
        }
        Some(Symbol::new(Keysym::new(0), name))
    }
}

/// The placeholder table, as a value.
static PLACEHOLDER_KEYSYMS: PlaceholderKeysyms = PlaceholderKeysyms;

#[cfg(test)]
mod tests {
    use super::{Chord, ChordKind, Keysym, Keysyms, Modifiers, Reason, Symbol};

    struct Table;

    impl Keysyms for Table {
        fn resolve(&self, name: &str) -> Option<Symbol> {
            let lowered = name.to_ascii_lowercase();
            let (symbol, canonical) = match lowered.as_str() {
                "return" => (0xff0d, "Return"),
                "f9" => (0xffc6, "F9"),
                "q" => (0x71, "q"),
                "super_l" => (0xffeb, "Super_L"),
                "xf86audiomute" => (0x1008_ff13, "XF86AudioMute"),
                _ => return None,
            };
            Some(Symbol::new(Keysym::new(symbol), canonical))
        }
    }

    fn parse(text: &str) -> Result<Chord, super::ParseError> {
        Chord::parse(text, ChordKind::Key, &Table)
    }

    fn pointer(text: &str) -> Result<Chord, super::ParseError> {
        Chord::parse(text, ChordKind::Pointer, &Table)
    }

    #[test]
    fn every_accepted_form_parses_to_its_written_form() {
        for (written, canonical) in [
            ("<Super>Return", "<Super>Return"),
            ("<super>return", "<Super>Return"),
            ("<Super><Shift>q", "<Shift><Super>q"),
            ("<Shift><Super>q", "<Shift><Super>q"),
            ("<Primary>q", "<Control>q"),
            ("<Ctrl>q", "<Control>q"),
            ("<Ctl>q", "<Control>q"),
            ("<Shft>q", "<Shift>q"),
            ("<Mod1>q", "<Alt>q"),
            ("AudioMute", "XF86AudioMute"),
            ("XF86AudioMute", "XF86AudioMute"),
            ("Above_Tab", "Above_Tab"),
            ("above_tab", "Above_Tab"),
            ("0x3f", "0x3f"),
            ("0X3F", "0x3f"),
            ("Super", "Super"),
            ("q", "q"),
        ] {
            assert_eq!(parse(written).expect(written).to_string(), canonical, "{written}");
        }
        assert_eq!(
            pointer("<Super>Button1").expect("pointer").to_string(),
            "<Super>Button1"
        );
        assert_eq!(
            pointer("<ctrl><alt>Button3").expect("pointer").to_string(),
            "<Control><Alt>Button3"
        );
    }

    #[test]
    fn every_refusal_names_the_reason() {
        for (written, reason) in [
            ("", Reason::Empty),
            ("<Super>", Reason::MissingKey),
            ("<Super", Reason::UnclosedModifier),
            ("<>q", Reason::EmptyModifier),
            (
                "<Windows>q",
                Reason::UnknownModifier {
                    name: "Windows".to_owned(),
                },
            ),
            (
                "<Super><super>q",
                Reason::DuplicateModifier {
                    name: "super".to_owned(),
                },
            ),
            ("q<Super>", Reason::MoreThanOneKey),
            ("<Super>a<Super>b", Reason::MoreThanOneKey),
            ("Super q", Reason::Whitespace),
            ("0x00", Reason::ZeroKeyCode),
            (
                "nonsense",
                Reason::UnknownKey {
                    name: "nonsense".to_owned(),
                },
            ),
        ] {
            assert_eq!(parse(written).expect_err(written).reason(), &reason, "{written}");
        }
        assert!(matches!(
            parse("<Mod4>q").expect_err("mod4").reason(),
            Reason::UnportableModifier { .. }
        ));
        assert!(matches!(
            parse("<Mod2>q").expect_err("mod2").reason(),
            Reason::UnportableModifier { .. }
        ));
        assert_eq!(
            pointer("Button1").expect_err("no modifier").reason(),
            &Reason::PointerNeedsModifier
        );
        assert_eq!(pointer("q").expect_err("not a button").reason(), &Reason::NotAButton);
        assert_eq!(
            pointer("Super").expect_err("not a button").reason(),
            &Reason::NotAButton
        );
        assert_eq!(
            parse(&format!("<Super>{}", "k".repeat(65))).expect_err("long").reason(),
            &Reason::KeyNameTooLong { length: 65 }
        );
        assert_eq!(
            parse(&"q".repeat(129)).expect_err("long").reason(),
            &Reason::TooLong { length: 129 }
        );
    }

    #[test]
    fn the_message_shows_the_form_to_write() {
        let error = parse("Super+Alt+F9").expect_err("plus signs");
        assert_eq!(
            error.message(),
            "\"Super+Alt+F9\" is not a chord: put each modifier in angle brackets before the key. Write <Alt><Super>F9"
        );
        let error = parse("ctrl-q").expect_err("dashes");
        assert_eq!(
            error.message(),
            "\"ctrl-q\" is not a chord: put each modifier in angle brackets before the key. Write <Control>q"
        );
        let error = parse("windows alt f9").expect_err("spaces");
        assert_eq!(
            error.message(),
            "\"windows alt f9\" is not a chord: put each modifier in angle brackets before the key. Write <Alt><Super>f9"
        );
        let error = parse("<Foo>q").expect_err("unknown modifier");
        assert_eq!(
            error.message(),
            "\"<Foo>q\" is not a chord: <Foo> is not a modifier; the modifiers are <Shift>, <Control>, <Alt>, <Super>, <Hyper> and <Meta>. Write modifiers in angle brackets before one key, for example <Super><Alt>F9"
        );
    }

    #[test]
    fn one_key_has_one_identity_however_it_is_written() {
        assert_eq!(parse("q").expect("q"), parse("Q").expect("Q"));
        assert_eq!(parse("AudioMute").expect("a"), parse("XF86AudioMute").expect("b"));
        assert_eq!(parse("<Alt><Super>q").expect("a"), parse("<Super><Alt>q").expect("b"));
        assert_eq!(parse("<Ctrl>q").expect("a"), parse("<Control>q").expect("b"));
        assert_eq!(parse("0x3F").expect("a"), parse("0x3f").expect("b"));
        assert_ne!(parse("q").expect("q"), parse("<Shift>q").expect("shift"));
        assert_ne!(parse("Super").expect("bare"), parse("<Super>Super_L").expect("keysym"));
    }

    #[test]
    fn exact_modifiers_and_keysym_identify_an_event() {
        let chord = parse("<Super>q").expect("q");
        assert!(chord.matches_key(Keysym::new(0x71), Modifiers::SUPER));
        assert!(!chord.matches_key(Keysym::new(0x71), Modifiers::SUPER.union(Modifiers::SHIFT)));
        assert!(!chord.matches_key(Keysym::new(0x71), Modifiers::NONE));
        assert!(!chord.matches_key(Keysym::new(0x51), Modifiers::SUPER));
        assert!(!chord.matches_button(1, Modifiers::SUPER));
        assert!(
            pointer("<Super>Button2")
                .expect("b")
                .matches_button(2, Modifiers::SUPER)
        );
        assert!(
            !pointer("<Super>Button2")
                .expect("b")
                .matches_button(1, Modifiers::SUPER)
        );
    }
}
