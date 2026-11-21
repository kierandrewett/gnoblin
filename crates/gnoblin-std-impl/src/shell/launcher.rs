//! The launcher: a command entry that opens over the windows. Return runs the command and closes the entry; Escape closes it without
//! running anything; a blank text runs nothing.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Closure, Obj, Res, Val};

use super::LauncherPart;
use crate::ui::CanvasEvent;
use crate::ui::entry;
use crate::ui::widget::{CanvasHandle, Cid, Special, Ui, WidgetKind};

/// The entry's width in pixels.
const WIDTH: f64 = 600.0;
/// The entry's height in pixels.
const HEIGHT: f64 = 48.0;
/// The entry's corner radius in pixels.
const RADIUS: f64 = 12.0;
/// The entry's background colour.
const FILL: &str = "#313244";
/// The entry's text size in pixels.
const FONT_SIZE: f64 = 18.0;
/// The distance from the top of the screen to the entry, in pixels.
const TOP: f64 = 200.0;
/// The width in pixels assumed for the screen while the tree has none.
const FALLBACK_SCREEN_WIDTH: f64 = 1200.0;

/// What the launcher adds to the entry it is: its size, look and text size.
///
/// # Errors
///
/// A canvas cannot be made.
pub(crate) fn build(ui: &Rc<Ui>, cx: &mut Cx<'_>, me: Obj, cid: Cid) -> Res<()> {
    entry::build(ui, cx, me, cid)?;
    ui.tweak(cid, |widget| {
        widget.width = Some(WIDTH);
        widget.height = Some(HEIGHT);
        widget.b.radius = RADIUS;
        widget.b.fill = FILL.to_owned();
        if let Special::Entry(part) = &mut widget.sp {
            part.size = FONT_SIZE;
            part.launcher = Some(LauncherPart::default());
        }
    });
    Ok(())
}

/// The launcher part of an entry canvas.
fn part(ui: &Ui, cid: Cid) -> Option<LauncherPart> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Entry(entry) => entry.launcher.clone(),
        _ => None,
    })
    .flatten()
}

/// The shell's way of starting a command, if it gave one.
pub(crate) fn spawn_of(ui: &Ui, cid: Cid) -> Option<Closure> {
    part(ui, cid).and_then(|held| held.spawn)
}

/// Gives the launcher a way of starting a command.
pub(crate) fn set_spawn(ui: &Ui, cid: Cid, spawn: Option<Closure>) {
    ui.tweak(cid, |widget| {
        if let Special::Entry(entry) = &mut widget.sp
            && let Some(launcher) = &mut entry.launcher
        {
            launcher.spawn = spawn;
        }
    });
}

/// Opens the launcher in the middle of the screen's width, over the windows, and gives it the keyboard. When one is open, focuses that
/// one and answers it.
///
/// # Errors
///
/// The compositor refuses the canvas or the keyboard.
pub(crate) fn open(ui: &Rc<Ui>, cx: &mut Cx<'_>, spawn: Option<Closure>) -> Res<Val> {
    let current = ui.shell.borrow().launcher.filter(|cid| ui.alive(*cid));
    if let Some(cid) = current {
        super::focus_canvas(ui, cid)?;
        return match ui.read(cid, |widget| widget.obj) {
            Some(obj) => cx.handout(obj),
            None => Ok(Val::None),
        };
    }
    let cid = ui.make(WidgetKind::Entry)?;
    let obj = cx.spawn("launcher", CanvasHandle { ui: Rc::clone(ui), cid })?;
    ui.tweak(cid, |widget| widget.obj = obj);
    build(ui, cx, obj, cid)?;
    let screen = ui.primary_screen().map_or(FALLBACK_SCREEN_WIDTH, |(width, _)| width);
    ui.tweak(cid, |widget| {
        widget.x = (screen / 2.0).floor() - WIDTH / 2.0;
        widget.y = TOP;
    });
    set_spawn(ui, cid, spawn);
    ui.mount_top(cid)?;
    super::focus_canvas(ui, cid)?;
    ui.shell.borrow_mut().launcher = Some(cid);
    ui.schedule();
    cx.handout(obj)
}

/// Starts `text` as the core's `shell` does.
fn core_spawn(ui: &Ui, text: &str) -> Result<(), String> {
    let objects = ui.g.objects().map_err(|error| error.to_string())?;
    let model = gnoblin_core::objects::ModelRoot::get(&objects).map_err(|fail| fail.to_string())?;
    model.shell(text, None).map(drop).map_err(|fail| fail.to_string())
}

/// Return was pressed with `text` typed: a text that is not blank runs, and the launcher closes. A command that cannot be started leaves the
/// launcher open.
pub(crate) fn submit(ui: &Rc<Ui>, cid: Cid, text: &str) {
    if !text.trim().is_empty() {
        let started = match spawn_of(ui, cid) {
            Some(spawn) => spawn
                .call(0, &[Val::Text(text.to_owned())])
                .map(drop)
                .map_err(|fail| fail.to_string()),
            None => core_spawn(ui, text),
        };
        if let Err(message) = started {
            ui.warn(&format!("Launcher.onSubmit threw: {message}"));
            return;
        }
    }
    ui.kill(cid);
}

/// Escape closes the launcher. Answers whether the key was used.
pub(crate) fn key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    if part(ui, cid).is_none() || !raw.pressed || raw.key != "Escape" {
        return false;
    }
    ui.kill(cid);
    true
}
