//! The Gnoblin configuration document: what a setting means, and what to do when
//! one is wrong.
//!
//! The Lua host evaluates `init.lua` and hands over a neutral tree
//! ([`Value`]); this crate validates that tree into a typed [`Config`], merges
//! layered documents, keeps what it can when a section is broken, and stores the
//! accepted document so a broken edit cannot cost the session its settings. It
//! never runs Lua and never reads the config file: the one exception is
//! [`locate`], which names the file, and [`Store`], which keeps the last accepted
//! document. `cargo xtask deps` enforces that boundary: this is a foundation
//! crate, so it depends on no Smithay, Wayland, D-Bus or `mlua` code.
//!
//! # Failure modes first
//!
//! The list below was written before the logic, and every line is covered by a
//! test.
//!
//! 1. **An invalid section poisons the whole file.** One bad gesture must not
//!    cost the user their cursor theme, and the recovery notice must say which
//!    section was dropped. [`salvage`] drops the sections that hold a problem,
//!    keeps the rest and reports every problem it dropped.
//! 2. **A torn write of the last-good document.** A crash mid-save must leave the
//!    previous document readable rather than half of the new one. [`Store`]
//!    writes a temporary file, syncs it, renames it and syncs the directory, so
//!    the store holds the old document or the new one; a truncated file is
//!    refused with a reason instead of being applied.
//! 3. **Two files disagree about precedence.** The merged result must depend on
//!    the order the caller supplies and on nothing else. [`merge_layers`] takes
//!    layers in load order and the last one wins: tables merge by key, lists and
//!    scalars are replaced whole.
//! 4. **An alias is accepted silently.** A deprecated spelling must be accepted,
//!    canonicalised in the stored document, and reported. [`ALIASES`] is the one
//!    table; parsing rewrites the value and records a [`Deprecation`] for it, so
//!    the document that is stored and published holds only current spellings.
//! 5. **A second problem hides behind the first.** One reload must report every
//!    problem, not one problem per reload: [`Config::from_value`] collects
//!    problems and returns one [`Report`].
//! 6. **A last-good document from an older schema.** A stored document can hold a
//!    key this version no longer accepts; it is re-validated on load and refused
//!    with the problem, so the session starts from its defaults rather than on
//!    settings nobody meant.
//! 7. **A last-good document belongs to another config path.** Two sessions or a
//!    `GNOBLIN_CONFIG` switch must not publish one file's settings for another;
//!    the envelope records its source path and a mismatch is refused.
//! 8. **An unknown key is reported without a usable hint.** Every problem names
//!    its dotted path and the accepted values, so the recovery panel and the log
//!    line are the same text.
//! 9. **The tree cannot be walked.** A script can nest tables deeper than the
//!    stack allows, and JSON cannot store infinity; both are refused at the root
//!    before any section is read.
//! 10. **The stored document is readable by another user.** The state directory
//!     and the file are created `0700` and `0600`, owned by this user, and a load
//!     refuses anything else.
//! 11. **The config path depends on who asked.** One [`locate`] answers with one
//!     order — `GNOBLIN_CONFIG`, then the XDG config home, then the home
//!     directory — and one file name, so `gnoblinctl` and the compositor cannot
//!     disagree.
//! 12. **A removed setting reads as a typo.** A key that used to exist is refused
//!     with what replaced it, which an unknown-key hint cannot say.
//!
//! # What this layer does not validate
//!
//! Five sections are carried but not interpreted: `keybindings`, `shortcuts`,
//! `autostart`, `window-rules` and `animations`. They hold Lua callbacks and rule
//! declarations whose meaning belongs to the Lua host and to window management.
//! The document keeps them, rewrites nothing inside them, and leaves their
//! refusal to the consumer that knows what they mean.

pub mod document;
pub mod enums;
pub mod last_good;
pub mod locate;
pub mod merge;
pub mod path;
pub mod problem;
pub mod sections;
pub mod value;

mod fields;

pub use document::{CARRIED_SECTIONS, Config, Salvaged, TOP_LEVEL_KEYS, salvage};
pub use enums::{
    ALIASES, AccelProfile, Alias, Blur, ClickMethod, Deprecation, DeviceAccelProfile, ENUM_SPECS, EnumParse, EnumSpec,
    FocusMode, FocusNewWindows, GestureAction, GestureDirection, GestureKind, GestureWhen, Handedness, InputSourceType,
    LocationAccuracy, NewWindowPlacement, PadButtonAction, Protocol, ScrollMethod, SettingEnum, StylusAction,
    StylusEraserMode, TabletMapping, TapButtonMap, TitlebarAction, TitlebarButton, VisualBellType, XwaylandExtension,
};
pub use last_good::{FORMAT, Resolution, Status, Store, StoreError, StoredDocument};
pub use locate::{ConfigPath, LocateError, Origin, SearchPaths, locate};
pub use merge::{merge, merge_layers};
pub use path::{Path, Segment};
pub use problem::{Fault, Problem, Report};
pub use sections::Inherit;
pub use sections::general::{Auth, Compositor, Cursor, LayerShell, Location, Monitors, Prompts, Xwayland};
pub use sections::input::{
    AccelCurve, Accessibility, BounceKeys, Input, Keyboard, Mouse, MouseKeys, PadButton, PointingStick, SlowKeys,
    StickyKeys, Stylus, Tablet, ToggleKeys, Touchpad, Trackball,
};
pub use sections::protocols::{FrameRenderers, InputSource, InputSources, Protocols};
pub use sections::window::{ButtonLayout, Gesture, GesturePoint, WindowManagement, Workspace};
pub use value::{MAX_DEPTH, Value};
