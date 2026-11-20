//! What a frame reads of the window it frames, the `frame_client` a closure is given, and the calls a frame makes into the shell.

use gnoblin_object::{Closure, Val};

use super::{FrameHooks, FrameId, FrameKind};
use crate::nodes;
use crate::object::record::record;
use crate::ui::widget::Ui;

/// Which shell operation a click on a frame asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hook {
    Menu,
    Lower,
    Maximise,
    Minimise,
    Press,
    Release,
}

impl FrameHooks {
    /// The closure that does `hook`, if the shell gave one.
    pub(crate) fn get(&self, hook: Hook) -> Option<Closure> {
        match hook {
            Hook::Menu => &self.menu,
            Hook::Lower => &self.lower,
            Hook::Maximise => &self.maximise,
            Hook::Minimise => &self.minimise,
            Hook::Press => &self.press,
            Hook::Release => &self.release,
        }
        .clone()
    }
}

/// A window of the tree, as a frame reads it. A window that is gone reads as the empty value of each property.
#[derive(Clone, Copy)]
pub(crate) struct Win<'a> {
    pub(crate) ui: &'a Ui,
    pub(crate) node: u64,
}

impl<'a> Win<'a> {
    pub(crate) fn of(ui: &'a Ui, node: u64) -> Win<'a> {
        Win { ui, node }
    }

    pub(crate) fn alive(&self) -> bool {
        nodes::alive(&self.ui.g, self.node)
    }

    pub(crate) fn text(&self, key: &str) -> String {
        nodes::text(&self.ui.g, self.node, key)
    }

    pub(crate) fn flag(&self, key: &str) -> bool {
        nodes::flag(&self.ui.g, self.node, key)
    }

    pub(crate) fn number(&self, key: &str) -> f64 {
        nodes::number(&self.ui.g, self.node, key)
    }

    pub(crate) fn title(&self) -> String {
        self.text("title")
    }

    pub(crate) fn app_id(&self) -> String {
        self.text("app_id")
    }

    pub(crate) fn focused(&self) -> bool {
        self.flag("focused")
    }

    pub(crate) fn fullscreen(&self) -> bool {
        self.flag("fullscreen")
    }

    pub(crate) fn maximised(&self) -> bool {
        self.flag("maximised")
    }

    pub(crate) fn width(&self) -> f64 {
        self.number("width")
    }

    pub(crate) fn height(&self) -> f64 {
        self.number("height")
    }

    pub(crate) fn decoration(&self) -> String {
        self.text("decoration")
    }

    /// The `frame_client` of the window as it is now: what a closure is given.
    pub(crate) fn record(&self) -> Val {
        record(vec![
            Val::Text(self.node.to_string()),
            Val::Text(self.title()),
            Val::Text(self.app_id()),
            Val::Bool(self.focused()),
            Val::Bool(self.fullscreen()),
            Val::Bool(self.maximised()),
            Val::Bool(self.flag("minimised")),
            Val::Double(self.number("x")),
            Val::Double(self.number("y")),
            Val::Double(self.width()),
            Val::Double(self.height()),
            Val::Text(self.decoration()),
            Val::Bool(self.alive()),
        ])
    }
}

impl Ui {
    /// The window of a frame: its node, `None` once the frame is gone.
    pub(crate) fn frame_node(&self, frame: FrameId) -> Option<u64> {
        self.frames.borrow().frames.get(&frame).map(|rec| rec.node)
    }

    /// The window of a frame as a frame reads it, or `None` when the frame is gone.
    pub(crate) fn frame_node_alive(&self, frame: FrameId) -> Option<u64> {
        self.frames
            .borrow()
            .frames
            .get(&frame)
            .filter(|rec| rec.alive)
            .map(|rec| rec.node)
    }

    /// Whether the frame is of a window the shell frames.
    pub(crate) fn frame_kind(&self, frame: FrameId) -> Option<FrameKind> {
        self.frames.borrow().frames.get(&frame).map(|rec| rec.kind)
    }

    /// The shell's closure for `hook`, if it gave one.
    pub(crate) fn hook(&self, hook: Hook) -> Option<Closure> {
        self.frames.borrow().deco.hooks.get(hook)
    }

    /// Runs the shell's closure `closure` and reads a boolean answer. A closure that threw, or answered something else, is an error.
    pub(crate) fn ask(&self, closure: &Closure, args: &[Val]) -> Result<bool, String> {
        closure
            .call(0, args)
            .and_then(|answer| answer.bool())
            .map_err(|fail| fail.message)
    }

    /// Asks `closure` about the window `node`.
    pub(crate) fn ask_window(&self, closure: &Closure, node: u64) -> Result<bool, String> {
        self.ask(closure, &[Win::of(self, node).record()])
    }
}
