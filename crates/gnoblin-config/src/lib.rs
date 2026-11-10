//! The Gnoblin configuration document: what a setting means, and what to do when
//! one is wrong.
//!
//! The Lua host evaluates `init.lua` and hands over a neutral tree ([`Value`]);
//! this crate validates that tree into a typed [`Config`], merges layered
//! documents, keeps what it can when a section is broken, and stores the accepted
//! document so a broken edit cannot cost the session its settings.
//!
//! # One declaration per setting
//!
//! The typed document, its enums, the alias table, the per-field validators, the
//! settings catalogue, the reference tables and the Lua stubs all come from
//! `schema/config.toml` at build time: [`generated`] is its `OUT_DIR` output,
//! compiled here with `include!`, and `cargo xtask gen-config` writes the
//! human-readable outputs into a gitignored directory. Nothing generated is
//! committed, so a clean checkout builds with plain `cargo build` and there is no
//! checked-in copy to drift. What the schema cannot express stays hand-written in
//! [`hooks`]: the cross-field rules, which the schema names and the generator
//! calls.
//!
//! # Failure modes first
//!
//! 1. **An invalid section poisons the whole file.** [`salvage`] drops the
//!    sections that hold a problem, keeps the rest and reports every one.
//! 2. **A torn write of the last-good document.** [`Store`] writes a temporary
//!    file, syncs it, renames it and syncs the directory; a truncated file is
//!    refused with a reason.
//! 3. **Two files disagree about precedence.** [`merge_layers`] takes layers in
//!    load order and the last one wins.
//! 4. **An alias is accepted silently.** A deprecated spelling is canonicalised
//!    in the document and reported; [`ALIASES`] is the one generated table.
//! 5. **A second problem hides behind the first.** [`Config::from_value`] collects
//!    problems and returns one [`Report`].
//! 6. **A last-good document from an older schema.** Re-validated on load and
//!    refused with the problem.
//! 7. **A last-good document belongs to another config path.** The envelope
//!    records its source path and a mismatch is refused.
//! 8. **An unknown key is reported without a usable hint.** Every problem names
//!    its dotted path and the accepted values, generated from the schema.
//! 9. **The tree cannot be walked.** A tree nested past the depth limit, or
//!    holding a non-finite number, is refused at the root.
//! 10. **The stored document is readable by another user.** `0700` directory,
//!     `0600` file, owned by this user, or the load refuses.
//! 11. **The config path depends on who asked.** One [`locate`] answers with one
//!     order and one file name.
//! 12. **A removed setting reads as a typo.** A key that used to exist is refused
//!     with what replaced it, before the unknown-key pass.
//! 13. **Generated and hand-written validators disagree.** `tests/generated.rs`
//!     re-reads the catalogue: every default is inside its own range, every alias
//!     names a current spelling, and the hooks the schema lists are the hooks that
//!     exist.
//!
//! Four sections are carried but not interpreted: `extensions`, `keybindings`,
//! `portals` and `animations`.

/// Accent and colour rules for the `appearance` table.
pub mod appearance;
/// The typed configuration document and the salvage that keeps its valid sections.
pub mod document;
/// The validation engine: reads one setting and reports the diagnostic for a wrong one.
pub mod engine;
/// Extensions the host loads, named in `gnoblin.configure`.
pub mod extensions;
/// Cross-field rules that the schema cannot declare.
pub mod hooks;
/// The `keyring` section: whether Gnoblin runs its built-in keyring.
pub mod keyring;
/// The last-good store: the last accepted document, kept on disk.
pub mod last_good;
/// Decides which file is the configuration, and in what search order.
pub mod locate;
/// The `logging` section, read before the log subscriber is installed.
mod logging;
/// Layered documents: how several files become one.
pub mod merge;
/// Shared shapes that the generated and hand-written code both name.
pub mod model;
/// Dotted paths that name a setting in diagnostics.
pub mod path;
/// Validation problems and the report that carries all of them.
pub mod problem;
/// The neutral value tree that the Lua host hands over.
pub mod value;

/// The document, generated from `schema/config.toml` by `build.rs`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

/// The typed document, the sections it carries, and the salvage that keeps valid sections.
pub use document::{CARRIED_SECTIONS, Config, Salvaged, TOP_LEVEL_KEYS, salvage};
/// Generated enums, settings and the alias table, all from `schema/config.toml`.
pub use generated::{
    ALIASES, AccelProfile, Accessibility, Appearance, Auth, Autostart, AutostartEntry, AutostartWhen, Blur, BounceKeys,
    ButtonLayout, CaptureSecureTreatment, CaptureSettings, ClickMethod, ColorScheme, ColourFilter, Compat,
    CompatCapture, Compositor, Cursor, CursorProvider, DeviceAccelProfile, ENUM_SPECS, FocusMode, FocusNewWindows,
    FrameRenderers, Gesture, GestureAction, GestureDirection, GestureKind, GesturePoint, Handedness, Input,
    InputSource, InputSourceType, InputSources, Keyboard, KeyringConfig, LayerOverride, LayerShell, Layers, Layout,
    LayoutOutput, LayoutWorkspace, Location, LocationAccuracy, LogConsole, Logging, Monitors, Mouse, MouseKeys,
    PadButton, PadButtonAction, PointingStick, Protocol, Protocols, RendererChoice, RuleClientType, RuleCorners,
    RuleWorkspace, ScrollMethod, Shortcut, ShortcutTrigger, Shortcuts, SlowKeys, StickyKeys, Stylus, StylusAction,
    StylusEraserMode, Tablet, TabletMapping, TapButtonMap, TitlebarAction, TitlebarButton, ToggleKeys, Touchpad,
    Trackball, VisualAccessibility, VisualBellType, WindowManagement, WindowMatch, WindowRule, WindowRuleType,
    Workspace, WorkspaceSet, WorkspaceSetEntry, Xwayland, XwaylandExtension,
};
/// The layout a setting names: a layout name, or a module reference and its options.
pub use generated::{LayoutOptionValue, LayoutReference, LayoutRoot, LayoutSelector};
/// The built-in keyring rule: from the environment, from a document, or from the last accepted file.
pub use keyring::{
    keyring_enabled_from_document, keyring_enabled_from_environment, keyring_enabled_from_store,
    last_good_keyring_enabled_from_environment,
};
/// The last-good store, its on-disk format version and its load result.
pub use last_good::{FORMAT, Resolution, Status, Store, StoreError, StoredDocument};
/// Config path lookup: the located path, its origin and the search order.
pub use locate::{ConfigPath, LocateError, Origin, SearchPaths, locate};
/// Reads the `logging` section from the environment before the subscriber exists.
pub use logging::logging_from_environment;
/// Merges layered documents into one, last layer wins.
pub use merge::{merge, merge_layers};
/// Shared model types: deprecations, enum parsing, settings and their kinds.
pub use model::{Default, Deprecation, EnumParse, EnumSpec, Inherit, Kind, Setting, SettingEnum};
/// Dotted path and segment types used in diagnostics.
pub use path::{Path, Segment};
/// Validation problems, their fault class and the report that holds them.
pub use problem::{Fault, Problem, Report};
/// The value tree type and the depth limit it enforces.
pub use value::{MAX_DEPTH, Value};
