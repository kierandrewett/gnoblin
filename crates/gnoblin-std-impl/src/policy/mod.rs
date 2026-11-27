//! The window policy of the native std: what the desktop does with focus, with the requests a window makes, with a window the keyboard moves,
//! with the edges a dragged window snaps to, and with the touchpad gestures and pointer drags that act on windows and workspaces.
//!
//! Each piece that watches the desktop is a service with a `start`, and does nothing before it, so a shell that has a policy of its own never
//! starts this one and two policies never answer one request. The one exception is `focus`: a shell that takes the tree decides every window's
//! focus (the compositor's own window manager stands down), so the library starts the stock focus policy itself when it becomes the
//! generation's desktop (`events::open`), and a shell that wants another calls `focus_policy.stop`:
//!
//! - `focus`: `focus_policy`, the focus modes, raise on click and auto raise, and the timers they need;
//! - `requests`: `window_state`, the router of the requests a window makes (the first listener that handles one answers it, the stock policy
//!   last) and the stock answers (maximise, fullscreen, minimise, activate, raise, lower, close, and the drags a move or resize request starts);
//! - `reposition`: `repositioning`, the keyboard move and resize modes;
//! - `snap`: where a dragged window tiles to;
//! - `drags`: the pointer drags the library runs itself (`move_binding`, `resize_binding`, `swap_binding`, `split_resize_binding`);
//! - `gesture_actions` and `slide`: the touchpad gestures that run actions, and the three-finger workspace slide;
//! - `binding`: what those bindings share, and how one becomes a binding of the core's input matcher;
//! - `stack`: the windows as a stack, and who holds the keyboard.
//!
//! The toolkit (`crate::ui`) is what hears the compositor, so the policy hangs on its frame and its events: the requests of windows, the
//! pointer's edges on windows, the keys of the grab, and the frame that runs the timers.

mod binding;
mod drags;
mod focus;
mod gesture_actions;
mod reposition;
mod requests;
mod slide;
mod snap;
mod stack;

use std::rc::Rc;

use gnoblin_object::server::Builder;

pub(crate) use reposition::GrabPart;

use crate::ui::{CanvasEvent, Cid, Ui};

/// What the policy keeps for one generation.
#[derive(Default)]
pub(crate) struct PolicyWorld {
    focus: focus::Focus,
    requests: requests::WindowRequests,
    reposition: reposition::KeyMode,
}

/// Implements the classes of the policy.
pub(crate) fn register(builder: &mut Builder) {
    focus::register(builder);
    requests::register(builder);
    reposition::register(builder);
    snap::register(builder);
    binding::register_base(builder);
    drags::register(builder);
    gesture_actions::register(builder);
    slide::register(builder);
}

/// Starts the stock focus policy for the toolkit `ui`, once. The library calls it when it becomes a generation's desktop (`events::open`),
/// so a shell that lets the library place its windows also has windows that take the keyboard; `focus_policy.stop` ends it for a shell
/// that has a focus policy of its own.
pub(crate) fn start_focus(ui: &Rc<Ui>) -> gnoblin_object::Res<()> {
    focus::start(ui)
}

/// Stops the stock focus policy for the toolkit `ui`. Only `ui::testing` calls it (the oracle records without stock focus); a shell
/// stops the policy through `focus_policy.stop`.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn stop_focus(ui: &Rc<Ui>) {
    focus::stop(ui);
}

/// A request a window made. Answers whether a listener handled it, which the window menu respects: a `menu` request a listener answered
/// does not open the menu.
pub(crate) fn on_request(ui: &Rc<Ui>, node: u64, request: &str, x: f64, y: f64) -> bool {
    requests::on_request(ui, node, request, x, y)
}

/// A pointer edge on a window (`enter`, `motion`, `leave`, `press`, `release`).
pub(crate) fn on_window_event(ui: &Rc<Ui>, raw: &CanvasEvent) {
    if !matches!(raw.kind.as_str(), "enter" | "motion" | "leave" | "press" | "release") {
        return;
    }
    requests::pointer_edge(ui, &raw.kind);
    focus::on_pointer(ui, raw.node, &raw.kind);
}

/// A key for the canvas that holds the keyboard while a window is moved or resized. Answers whether the key was used up.
pub(crate) fn on_key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    reposition::on_key(ui, cid, raw)
}

/// The toolkit's frame: the timers that are due, the drag a request started follows the pointer, and a mode that ends with its window ends.
pub(crate) fn turn(ui: &Rc<Ui>) {
    reposition::watch(ui);
    focus::turn(ui);
    requests::turn(ui);
}

/// How long until the policy needs a turn, in milliseconds.
pub(crate) fn wake_in(ui: &Ui) -> Option<u64> {
    [focus::wake_in(ui), requests::wake_in(ui)].into_iter().flatten().min()
}

/// Whether the toolkit has to run every frame for the policy.
pub(crate) fn polls(ui: &Ui) -> bool {
    reposition::polls(ui)
}

/// Whether the keyboard modes can be started: the window menu uses them for its Move and Resize.
pub(crate) fn repositions(ui: &Ui) -> bool {
    reposition::started(ui)
}

/// The window menu's Move (`resize` false) and Resize start the keyboard mode.
pub(crate) fn begin_reposition(ui: &Rc<Ui>, client: u64, resize: bool) {
    reposition::begin_from_menu(ui, client, resize);
}

/// `client.moveWithKeys()` (`resize` false) and `client.resizeWithKeys()`: starts the keyboard mode for the core's `Client` `client`.
/// False for a window the mode does not take (fullscreen, maximised, tiled, gone or not showing) and before `Repositioning` starts.
pub(crate) fn reposition_with_keys(
    cx: &mut gnoblin_object::server::Cx<'_>,
    client: &gnoblin_object::Val,
    resize: bool,
) -> gnoblin_object::Res<gnoblin_object::Val> {
    let member = if resize { "resizeWithKeys" } else { "moveWithKeys" };
    let client = crate::object::client::client_node(cx.ctx(), member, client)?;
    let ui = crate::ui::ui_of(cx)?;
    let mode = if resize { 1 } else { 0 };
    reposition::begin(&ui, cx, client, mode).map(gnoblin_object::Val::Bool)
}
