//! `libgnoblin-std`: the optional desktop object model above the core.
//!
//! This crate is built as a `cdylib` whose SONAME is `libgnoblin-std.so.1` (set in `build.rs`; Cargo names the file
//! `libgnoblin_std_impl.so`, and dist installs it under the SONAME). It exports three symbols (`exports.rs`) and one versioned function
//! table (`table.rs`); everything a shell does with it goes through the table. `gnoblin-std-abi` is the
//! contract, and the TypeScript library in `data/js/gnoblin-std` is the reference for what it does.
//!
//! What it holds, in the order a request meets it:
//!
//! - `providers`: what std registers with the core as the desktop's policy (the window manager, the portal dialogs and the
//!   configuration observer) and `gnoblin_std_init`, which does it.
//! - `table`: the C functions, each contained against a panic, each reading the caller's structs by their own `size`.
//! - `state`: the object model of one generation (clients, outputs, workspaces, desks, collections, listeners) and the one rule
//!   about callbacks: the library never runs a shell callback inside a table call or while it holds a borrow of its state.
//! - `events`: `open`, and what the library hears from the core (a window appearing, going, changing; an output the same).
//! - `collection`: live collections and their selections (`where`), with tracked views that announce members that stop matching.
//! - `workspace`: workspaces and the membership of clients in them, dialogs, the keyboard when a window closes.
//! - `desks`: which output shows which workspace; the output policies (separate, duplicate, extend) and the workspace policy
//!   (manual, per output); showing a workspace, with a slide.
//! - `layout` and `floating`: the layout objects (the library's floating layout, and a layout the shell implements through a C
//!   table) and the floating placement; `fit` refits windows to a new work area.
//! - `turn`: the pass that runs once after the listeners of a dispatch turn: it tells the listeners what changed, computes the dirty
//!   workspaces that are shown, writes their placements, and shows and hides windows.
//! - `animation`: the five easing curves, the staging of a transition's writes, and the engine that sends them as timed writes joined
//!   to one `tween` node and follows the compositor's word on how each ended.
//! - `style`: scopes of style (every client, a workspace's, a selection's, one client's), the cascade, and what a resolved style
//!   becomes on a node: effects (shadows, borders, the backdrop blur), opacity and corner radius.
//! - `scene` and `camera`: nodes, views, `Camera3D` and the flat `Camera2D`, with the checks of the TypeScript scene.
//!
//! The library reaches the compositor only through the installed core, as the shell's guest: it holds the same handle on the
//! generation that the shell does, made by `gnoblin_core::private::gnoblin_for`. It names the core SDK and the two ABI crates and
//! nothing of the core's implementation, so it runs on any ABI-compatible core. Nothing outside this crate's own tests may depend on
//! it as a Rust library (`impl-sealed`): a shell that does carries a second copy of the object model, which is the failure
//! `cargo xtask dist-check` exists to catch.

mod animation;
mod augment;
mod bindings;
mod builtin;
mod builtin_api;
mod client_state;
mod collection;
mod desks;
mod display_api;
mod display_objects;
mod displays;
mod error;
mod events;
mod exports;
mod fit;
mod floating;
mod floating_api;
mod frame;
mod layout;
mod lock;
mod math;
mod menus;
mod nodes;
mod object;
mod objects;
mod policy;
mod providers;
mod scene;
mod shell;
mod splits;
mod standalone;
mod state;
mod style;
mod table;
mod turn;
mod ui;
mod workspace;
mod workspace_objects;

pub use exports::{gnoblin_std_abi_version, gnoblin_std_get_api, gnoblin_std_last_error};

/// What a test of the toolkit may change (the measurer of text). Not part of the library's interface: it is built for this crate's own
/// tests, which enable the feature through a development dependency on the crate itself.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use ui::testing as ui_testing;
