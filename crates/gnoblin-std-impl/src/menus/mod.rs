//! The window menu as native classes of `gnoblin.std`: the card and its rows, the keyboard navigator, the metrics, and the
//! `window_menus` service that answers a client's `menu` request.
//!
//! Its behaviour is held to the recorded fixtures (the `menus_*` files of `design/native-std-port-tests.md`). The pieces:
//!
//! - `metrics`: the sizes and colours of the card, where it goes, and what a key means to it;
//! - `navigator`: the keyboard selection across a card and its submenu;
//! - `entry`: `menu_entry`, what a row is made from, and the stock actions;
//! - `card`: the layer, its cards and rows, and what they do with the pointer and the keys;
//! - `windows`: the service: requests, opening and closing, the stock entries and what they do, and the pinned windows;
//! - `classes`: the catalog's classes (`menus.toml`) implemented over the above.
//!
//! The card is made of canvases of the toolkit (`ui`), which calls back into this module for what only a menu knows: the request a
//! client made of a window, the pointer and the keys a card takes, the refresh of its rows and what its marks paint.

mod card;
mod classes;
mod entry;
mod metrics;
mod navigator;
mod windows;

use std::rc::Rc;

use gnoblin_object::server::Builder;
use gnoblin_paint::PaintOp;

pub(crate) use card::{ItemPart, MarkPart, MenuPart, PanelPart};
pub(crate) use windows::State;

use crate::ui::{CanvasEvent, Cid, Ui, WidgetKind};

/// Implements the classes of the window menu.
pub(crate) fn register(builder: &mut Builder) {
    classes::register(builder);
}

/// A request a client made of a window: the menu opens for `menu`.
pub(crate) fn on_request(ui: &Rc<Ui>, node: u64, request: &str, x: f64, y: f64) {
    if request == "menu" {
        windows::on_request(ui, node, x, y);
    }
}

/// A click on a frame asks for the window's menu at the pointer.
pub(crate) fn show_at_pointer(ui: &Rc<Ui>, node: u64) {
    windows::show_at_pointer(ui, node);
}

/// The toolkit's frame: the menu closes with its window or the lock, and pinned windows stay above.
pub(crate) fn watch(ui: &Rc<Ui>) {
    windows::watch(ui);
}

/// Whether the toolkit has to run every frame for the menu.
pub(crate) fn polls(ui: &Ui) -> bool {
    windows::polls(ui)
}

/// The pointer on a canvas of a card, before a shell's handler: whether the card used the event up.
pub(crate) fn on_pointer(ui: &Rc<Ui>, cid: Cid, kind: u32, raw: &CanvasEvent) -> bool {
    card::on_pointer(ui, cid, kind, raw)
}

/// A key for the menu layer: whether it used the key up.
pub(crate) fn on_key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    card::on_key(ui, cid, raw)
}

/// Brings a canvas of a card up to date from live state.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid, kind: WidgetKind) {
    card::refresh(ui, cid, kind);
}

/// What a mark or a separator paints.
pub(crate) fn ops(ui: &Ui, cid: Cid, kind: WidgetKind) -> Result<Vec<PaintOp>, String> {
    card::ops(ui, cid, kind)
}
