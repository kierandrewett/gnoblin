//! The UI toolkit as native classes of `gnoblin.std`: canvases that paint a list of drawing operations, boxes that lay their children
//! out, text and icons, the entry, the clock, the bar and the toast.
//!
//! This is what `data/js/gnoblin-std/ui/*` and `widgets.ts` did in TypeScript, held to the same behaviour (the fixtures of
//! `design/native-std-port-tests.md`). The pieces:
//!
//! - `widget`: the canvases of a generation and the handle an object holds on one;
//! - `tree`: making and ending canvases, putting them under each other and under nodes of the compositor;
//! - `update`: the toolkit's turn and the update pass of a canvas (refresh, arrange, children, write what changed);
//! - `layout`: where a box puts its children, a row, a column or a grid;
//! - `text` and `paint`: measuring text in this process and the bytes of a paint list;
//! - `input`: pointer events and keys sent to canvases, bubbling, and the keyboard focus;
//! - `entry`, `clock`, `bar` and `toast`: the widgets the library draws and updates by itself;
//! - `session`: what the toolkit shares with the session, and the hooks a shell runs from its frame;
//! - `classes`: the catalog's classes (`ui.toml`, `widgets.toml`) implemented over the above.
//!
//! Nothing here runs a shell's closure inside a call of the object protocol: a handler runs when the compositor sends an event, and a
//! shell's hook runs from the toolkit's turn.

mod bar;
mod classes;
mod clock;
mod entry;
mod input;
mod layout;
mod paint;
mod session;
mod text;
mod toast;
mod tree;
mod update;
mod widget;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

use gnoblin_object::server::Builder;

/// Implements the classes of the toolkit.
pub(crate) fn register(builder: &mut Builder) {
    classes::register(builder);
    session::register(builder);
}
