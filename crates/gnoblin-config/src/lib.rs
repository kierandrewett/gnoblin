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
//! Seven sections are carried but not interpreted: `extensions`, `keybindings`,
//! `portals`, `shortcuts`, `autostart`, `window-rules` and `animations`.

pub mod document;
pub mod engine;
pub mod extensions;
pub mod hooks;
pub mod last_good;
pub mod locate;
mod logging;
pub mod merge;
pub mod model;
pub mod path;
pub mod problem;
pub mod value;

/// The document, generated from `schema/config.toml` by `build.rs`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use document::{CARRIED_SECTIONS, Config, Salvaged, TOP_LEVEL_KEYS, salvage};
pub use generated::{
    ALIASES, AccelProfile, Accessibility, Auth, Blur, BounceKeys, ButtonLayout, ClickMethod, Compat, CompatCapture,
    Compositor, Cursor, CursorProvider, DeviceAccelProfile, ENUM_SPECS, FocusMode, FocusNewWindows, FrameRenderers,
    Gesture, GestureAction, GestureDirection, GestureKind, GesturePoint, GestureWhen, Handedness, Input, InputSource,
    InputSourceType, InputSources, Keyboard, LayerShell, Layout, LayoutOutput, LayoutWorkspace, Location,
    LocationAccuracy, LogConsole, Logging, Monitors, Mouse, MouseKeys, NewWindowPlacement, PadButton, PadButtonAction,
    PointingStick, Prompts, Protocol, Protocols, RendererChoice, ScrollMethod, SlowKeys, StickyKeys, Stylus,
    StylusAction, StylusEraserMode, Tablet, TabletMapping, TapButtonMap, TitlebarAction, TitlebarButton, ToggleKeys,
    Touchpad, Trackball, VisualBellType, WindowManagement, Workspace, Xwayland, XwaylandExtension,
};
pub use last_good::{FORMAT, Resolution, Status, Store, StoreError, StoredDocument};
pub use locate::{ConfigPath, LocateError, Origin, SearchPaths, locate};
pub use logging::logging_from_environment;
pub use merge::{merge, merge_layers};
pub use model::{Default, Deprecation, EnumParse, EnumSpec, Inherit, Kind, Setting, SettingEnum};
pub use path::{Path, Segment};
pub use problem::{Fault, Problem, Report};
pub use value::{MAX_DEPTH, Value};
