//! The parts of the stock shell that sit above the toolkit, as native classes of `gnoblin.std`: the not-responding alert, the launcher and
//! the overview. (The toast is a widget of the toolkit.)
//!
//! Their behaviour is held to the recorded fixtures `reference/parts_*.json` (`design/native-std-port-tests.md`). The alert and the launcher
//! are built over the UI toolkit (`crate::ui`): every canvas of theirs is a canvas of the toolkit that carries a [`ShellPart`], so it is
//! updated, painted and written to the compositor by the toolkit's turn. The pieces:
//!
//! - `alerts`: the tracker that decides which windows get an alert, and the service that looks at the windows from the toolkit's turn;
//! - `dialog`: the card, its two buttons, and what the keyboard and the pointer do to them;
//! - `launcher`: the command entry and the one that is open;
//! - `overview`: the dimming of the windows of the workspaces that are not shown;
//! - `classes`: the catalog's classes (`shell.toml`) implemented over the above.
//!
//! A window is named by its node number as text. Nothing here runs a shell's closure inside a call of the object protocol: the alert looks at
//! the windows, arms and opens its cards from the toolkit's turn.

mod alerts;
mod classes;
mod dialog;
mod launcher;
mod overview;

use std::rc::Rc;

use gnoblin_object::Closure;

use crate::ui::CanvasEvent;
use crate::ui::widget::{Cid, Special, Ui, en};

pub(crate) use classes::register;

/// The values of the enums of the catalog (`shell.toml`), which are their places in it counted from one.
pub(crate) mod answer {
    pub(crate) const WAIT: u32 = 1;
    pub(crate) const FORCE: u32 = 2;
    pub(crate) const RECOVERED: u32 = 3;
}

/// The compositor's left button.
const LEFT: u64 = 272;

/// What a launcher adds to an entry.
#[derive(Clone, Default)]
pub(crate) struct LauncherPart {
    /// Starts the typed command line; `None` is the core's `shell`.
    pub(crate) spawn: Option<Closure>,
}

/// What the alert's card adds to a box.
#[derive(Clone)]
pub(crate) struct DialogPart {
    /// What the window is called.
    pub(crate) name: String,
    /// The button the keyboard is on.
    pub(crate) index: usize,
    /// Force quit takes a press.
    pub(crate) armed: bool,
    /// The Wait and Force quit buttons.
    pub(crate) buttons: Vec<Cid>,
    /// The window the card is for, when the service opened it.
    pub(crate) client: Option<u64>,
}

/// What a button of the alert adds to a box.
#[derive(Clone)]
pub(crate) struct ButtonPart {
    pub(crate) dialog: Cid,
    pub(crate) index: usize,
    pub(crate) label: Cid,
    pub(crate) enabled: bool,
    pub(crate) picked: bool,
}

/// What a canvas of the shell parts is.
#[derive(Clone)]
pub(crate) enum ShellPart {
    Dialog(DialogPart),
    Button(ButtonPart),
}

/// The state of the overview.
pub(crate) struct OverviewState {
    pub(crate) on: bool,
    pub(crate) dimmed: f64,
}

impl Default for OverviewState {
    fn default() -> OverviewState {
        OverviewState { on: false, dimmed: 0.4 }
    }
}

/// What the shell parts keep for one generation.
#[derive(Default)]
pub(crate) struct ShellWorld {
    pub(crate) alerts: alerts::Alerts,
    /// The launcher that is open.
    pub(crate) launcher: Option<Cid>,
    pub(crate) overview: OverviewState,
}

/// What a canvas carries of the shell parts, if it carries any.
pub(crate) fn part_of(ui: &Ui, cid: Cid) -> Option<ShellPart> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Shell(part) => Some(part.clone()),
        _ => None,
    })
    .flatten()
}

/// One turn of the alert, at the start of the toolkit's frame: the cards that are due are armed and the windows are looked at.
pub(crate) fn turn(ui: &Rc<Ui>) {
    alerts::frame(ui);
}

/// How long until the alert next needs a turn, in milliseconds, when it does.
pub(crate) fn wake_in(ui: &Rc<Ui>) -> Option<u64> {
    alerts::wake_in(ui)
}

/// Updates a canvas of the shell parts from what it follows: a button's colours, once a frame.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    if let Some(ShellPart::Button(button)) = part_of(ui, cid) {
        dialog::refresh_button(ui, cid, &button);
    }
    Ok(())
}

/// Places the children of a canvas that places them itself: a button centres its label. `false` for any other canvas.
pub(crate) fn arrange(ui: &Ui, cid: Cid) -> bool {
    match part_of(ui, cid) {
        Some(ShellPart::Button(button)) => {
            dialog::arrange_button(ui, cid, &button);
            true
        }
        _ => false,
    }
}

/// What a canvas of the shell parts does with a pointer event by itself, and whether it stops the event: a press on the card or on a button
/// stays there.
pub(crate) fn pointer(ui: &Rc<Ui>, cid: Cid, kind: u32, button: u64) -> bool {
    let Some(part) = part_of(ui, cid) else {
        return false;
    };
    if kind != en::POINTER_PRESS {
        return false;
    }
    match part {
        ShellPart::Dialog(_) => {
            if let Err(fail) = focus_canvas(ui, cid) {
                ui.warn(&format!("NotRespondingDialog.onPress threw: {fail}"));
            }
            true
        }
        ShellPart::Button(held) => {
            if button == LEFT {
                dialog::select(ui, held.dialog, held.index);
                dialog::activate(ui, held.dialog, held.index);
            }
            ui.schedule();
            true
        }
    }
}

/// What a canvas of the shell parts does with a key by itself, and whether it stops it: the card's keyboard, and the launcher's Escape.
pub(crate) fn key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    if let Some(ShellPart::Dialog(_)) = part_of(ui, cid) {
        return dialog::key(ui, cid, raw);
    }
    launcher::key(ui, cid, raw)
}

/// Return was pressed in the launcher `cid` with `text` typed.
pub(crate) fn submit_launcher(ui: &Rc<Ui>, cid: Cid, text: &str) {
    launcher::submit(ui, cid, text);
}

/// Gives the keyboard to the canvas `cid`, as its `focus` does.
///
/// # Errors
///
/// The canvas is gone, or the compositor refuses.
pub(crate) fn focus_canvas(ui: &Ui, cid: Cid) -> gnoblin_object::Res<()> {
    let node = ui
        .read(cid, |widget| widget.alive.then_some(widget.node))
        .flatten()
        .ok_or_else(|| gnoblin_object::Fail::stale("Canvas.focus: the canvas is destroyed"))?;
    ui.focus_node(node)
}
