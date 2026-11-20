//! The built-in layouts: tiling algorithms that live in the library, so every language's shell gets the same one.
//!
//! A shell names a kind (`STD_BUILTIN_DWINDLE`) and a typed property set; the library keeps the layout inside the workspace and runs
//! it. `gnoblin_std_abi::layouts::LAYOUT_KINDS` is the one declaration of what a kind has: its options (with types, ranges and
//! defaults), its actions and whether it has dividers. This module reads it, so an option the table does not name does not exist,
//! and the algorithms (one file per kind, over window ids and rectangles) read their options by name from [`OptionSet`].
//!
//! The algorithms are pure over window ids and rectangles, as the TypeScript library's are, and are held to its rectangles by
//! `tests/tiling.rs`. What a layout needs from the library comes in with the call: [`LayoutContext`] for a compute and
//! [`ActionEnv`] for an action. A layout changes the keyboard only by answering the window to focus; the library does it.

mod arc;
mod carousel;
mod dwindle;
mod master;
mod strip;
mod wire;

use gnoblin_core::Rect;
use gnoblin_std_abi::GnoblinStdSplit;
use gnoblin_std_abi::layouts::{Initial, LayoutKindDesc, LowerBound, OptionDesc, ValueDesc, ValueShape};
use gnoblin_std_abi::{STD_AXIS_X, STD_AXIS_Y, STD_SPLIT_CROSS};

use crate::error::{Fail, Res};
use crate::layout::{LayoutContext, Placement};
use crate::state::NodeId;

pub(crate) use carousel::{RingShape, ring_slots};
pub(crate) use wire::{decode_value, encode_value, read_args, read_one, read_options, read_set, read_text};

/// A value of an option, an argument or an answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    None,
    Bool(bool),
    /// A number, a whole number, or the number of an enumeration member.
    Num(f64),
    Node(u64),
    Text(String),
}

/// The values of the options of a kind of layout or binding, in the order the table declares them.
#[derive(Clone, Debug)]
pub(crate) struct OptionSet {
    owner: &'static str,
    descs: &'static [OptionDesc],
    values: Vec<Val>,
}

impl OptionSet {
    /// The defaults a layout kind declares.
    #[cfg(test)]
    pub(crate) fn defaults(kind: &'static LayoutKindDesc) -> OptionSet {
        OptionSet::of(kind.name, kind.options)
    }

    /// The defaults the table declares for the options `descs` of `owner`.
    pub(crate) fn of(owner: &'static str, descs: &'static [OptionDesc]) -> OptionSet {
        let values = descs
            .iter()
            .map(|option| match option.default {
                Initial::None => Val::None,
                Initial::Bool(value) => Val::Bool(value),
                Initial::Number(value) => Val::Num(value),
                Initial::Text(value) => Val::Text(value.to_owned()),
            })
            .collect();
        OptionSet { owner, descs, values }
    }

    fn index(&self, name: &str) -> usize {
        self.descs
            .iter()
            .position(|option| option.name == name)
            .unwrap_or_else(|| unreachable!("{} has no option `{name}`: the code and the table disagree", self.owner))
    }

    /// Sets the option `name` to a value that is checked already.
    pub(crate) fn put(&mut self, name: &str, value: Val) {
        let at = self.index(name);
        self.values[at] = value;
    }

    pub(crate) fn get(&self, name: &str) -> &Val {
        &self.values[self.index(name)]
    }

    pub(crate) fn boolean(&self, name: &str) -> bool {
        matches!(self.get(name), Val::Bool(true))
    }

    /// A number option, or the number of an enumeration member.
    pub(crate) fn num(&self, name: &str) -> f64 {
        match self.get(name) {
            Val::Num(value) => *value,
            _ => 0.0,
        }
    }

    /// An optional number option.
    pub(crate) fn maybe(&self, name: &str) -> Option<f64> {
        match self.get(name) {
            Val::Num(value) => Some(*value),
            _ => None,
        }
    }

    /// An enumeration option: the number of the member.
    pub(crate) fn member(&self, name: &str) -> u32 {
        self.num(name) as u32
    }
}

/// What an action needs to know about the workspace it runs on.
#[derive(Clone, Debug, Default)]
pub(crate) struct ActionEnv {
    /// The workspace's clients, in its order.
    pub(crate) clients: Vec<NodeId>,
    /// The client that has the keyboard, if it is one of them.
    pub(crate) focused: Option<NodeId>,
    /// The focused client's own size with its frame, when it has one.
    pub(crate) focused_size: Option<(f64, f64)>,
}

/// What an action did.
#[derive(Debug)]
pub(crate) struct ActionResult {
    /// What it answers.
    pub(crate) value: Val,
    /// The layout changed, so the workspace computes again.
    pub(crate) changed: bool,
    /// The window that is to get the keyboard.
    pub(crate) focus: Option<NodeId>,
}

impl ActionResult {
    pub(crate) fn nothing() -> ActionResult {
        ActionResult {
            value: Val::None,
            changed: false,
            focus: None,
        }
    }

    /// A boolean answer that says whether the layout changed.
    pub(crate) fn did(did: bool) -> ActionResult {
        ActionResult {
            value: Val::Bool(did),
            changed: did,
            focus: None,
        }
    }

    /// An answer that changes nothing.
    pub(crate) fn answer(value: Val) -> ActionResult {
        ActionResult {
            value,
            changed: false,
            focus: None,
        }
    }
}

/// A point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pt {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// One divider as a layout describes it: dragging the pointer `span` pixels along `x_axis` moves `ratio` by one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DividerSpec {
    pub(crate) x_axis: bool,
    pub(crate) span: f64,
    pub(crate) direction: i32,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) ratio: f64,
}

/// The dividers one drag moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SplitSpec {
    pub(crate) token: u64,
    pub(crate) divider: DividerSpec,
    pub(crate) cross: Option<DividerSpec>,
}

impl SplitSpec {
    pub(crate) fn to_raw(self) -> GnoblinStdSplit {
        let axis = |divider: &DividerSpec| if divider.x_axis { STD_AXIS_X } else { STD_AXIS_Y };
        let cross = self.cross;
        GnoblinStdSplit {
            size: size_of::<GnoblinStdSplit>() as u32,
            flags: if cross.is_some() { STD_SPLIT_CROSS } else { 0 },
            token: self.token,
            axis: axis(&self.divider),
            direction: self.divider.direction,
            span: self.divider.span,
            min: self.divider.min,
            max: self.divider.max,
            ratio: self.divider.ratio,
            cross_axis: cross.as_ref().map_or(0, axis),
            cross_direction: cross.map_or(1, |c| c.direction),
            cross_span: cross.map_or(0.0, |c| c.span),
            cross_min: cross.map_or(0.0, |c| c.min),
            cross_max: cross.map_or(0.0, |c| c.max),
            cross_ratio: cross.map_or(0.0, |c| c.ratio),
        }
    }
}

/// A built-in layout. Every function runs outside the library's borrow of its state, on the shell thread.
pub(crate) trait Imp {
    fn options(&self) -> &OptionSet;
    fn options_mut(&mut self) -> &mut OptionSet;

    /// An option changed: the layout refreshes what it derived from them.
    fn refresh(&mut self) {}

    /// Where the tiled clients go.
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement>;

    /// Whether `compute` reads the pointer (`LayoutContext::pointer`).
    fn wants_pointer(&self) -> bool {
        false
    }

    /// Whether the layout hands the keyboard on itself when a client leaves (`client_removed` answers who gets it).
    fn hands_focus(&self) -> bool {
        false
    }

    /// A client left the workspace; the answer is the window that gets the keyboard, if the layout hands it on.
    fn client_removed(&mut self, _cx: &LayoutContext, _client: NodeId) -> Option<NodeId> {
        None
    }

    /// Two clients traded places in the workspace's order.
    fn clients_swapped(&mut self, _a: NodeId, _b: NodeId) {}

    /// The divider a drag that pressed `client` at `at` would move.
    fn find_split(&mut self, _client: NodeId, _at: Pt) -> Option<SplitSpec> {
        None
    }

    fn set_split(&mut self, _token: u64, _ratio: f64, _cross: Option<f64>) {}

    fn reset_split(&mut self, _token: u64) {}

    /// Runs an action. `args` are in the order the table declares them, checked against it.
    fn act(&mut self, action: &str, _args: &[Val], _env: &ActionEnv) -> Res<ActionResult> {
        Err(Fail::no_such(format!("there is no action `{action}`")))
    }
}

/// A built-in layout of a workspace.
pub(crate) struct Builtin {
    pub(crate) kind: &'static LayoutKindDesc,
    imp: std::cell::RefCell<Box<dyn Imp>>,
}

impl Builtin {
    /// Makes the layout `kind` with `options` (already checked).
    pub(crate) fn new(kind: &'static LayoutKindDesc, options: OptionSet) -> Builtin {
        let mut imp: Box<dyn Imp> = match kind.id {
            gnoblin_std_abi::STD_BUILTIN_MASTER_STACK => Box::new(master::MasterStack::new(options)),
            gnoblin_std_abi::STD_BUILTIN_GRID => Box::new(master::Grid::new(options)),
            gnoblin_std_abi::STD_BUILTIN_MONOCLE => Box::new(master::Monocle::new(options)),
            gnoblin_std_abi::STD_BUILTIN_DWINDLE => Box::new(dwindle::Dwindle::new(options)),
            gnoblin_std_abi::STD_BUILTIN_STRIP => Box::new(strip::ColumnStrip::new(options)),
            gnoblin_std_abi::STD_BUILTIN_ARC => Box::new(arc::Arc::new(options)),
            _ => Box::new(carousel::Carousel::new(options)),
        };
        imp.refresh();
        Builtin {
            kind,
            imp: std::cell::RefCell::new(imp),
        }
    }

    pub(crate) fn compute(&self, cx: &LayoutContext) -> Vec<Placement> {
        self.imp.borrow_mut().compute(cx)
    }

    pub(crate) fn wants_pointer(&self) -> bool {
        self.imp.borrow().wants_pointer()
    }

    pub(crate) fn hands_focus(&self) -> bool {
        self.imp.borrow().hands_focus()
    }

    pub(crate) fn client_removed(&self, cx: &LayoutContext, client: NodeId) -> Option<NodeId> {
        self.imp.borrow_mut().client_removed(cx, client)
    }

    pub(crate) fn clients_swapped(&self, a: NodeId, b: NodeId) {
        self.imp.borrow_mut().clients_swapped(a, b);
    }

    pub(crate) fn find_split(&self, client: NodeId, x: f64, y: f64) -> Option<SplitSpec> {
        self.imp.borrow_mut().find_split(client, Pt { x, y })
    }

    pub(crate) fn set_split(&self, token: u64, ratio: f64, cross: Option<f64>) {
        self.imp.borrow_mut().set_split(token, ratio, cross);
    }

    pub(crate) fn reset_split(&self, token: u64) {
        self.imp.borrow_mut().reset_split(token);
    }

    /// Sets one option, checked against the table.
    pub(crate) fn set_option(&self, name: &str, value: Val) {
        let mut imp = self.imp.borrow_mut();
        imp.options_mut().put(name, value);
        imp.refresh();
    }

    /// The value of an option.
    pub(crate) fn option(&self, name: &str) -> Val {
        self.imp.borrow().options().get(name).clone()
    }

    /// Runs an action; the arguments are in the table's order.
    pub(crate) fn act(&self, action: &str, args: &[Val], env: &ActionEnv) -> Res<ActionResult> {
        self.imp.borrow_mut().act(action, args, env)
    }
}

// ---- shared arithmetic ------------------------------------------------------------------------------------------------------

/// The error a floor of float arithmetic is allowed: the right edge of 619.2 + 404.8 is 1023.9999999999999, and flooring it as it is
/// would lose a pixel the layout meant to give.
const EDGE_EPSILON: f64 = 1e-6;

/// `value` limited to `low..=high`, as the TypeScript library's `clamp`: `low` must not be above `high`.
pub(crate) fn clamp(value: f64, low: f64, high: f64) -> f64 {
    low.max(high.min(value))
}

/// A placement that leaves `node` where it is, shown, with the defaults.
pub(crate) fn plain(node: NodeId) -> Placement {
    Placement {
        node,
        bounds: None,
        visible: true,
        tiled: false,
        z: 0.0,
        rotation: [0.0; 3],
        scale: [1.0; 3],
        offset: None,
    }
}

/// A tiled placement at whole pixels. Each of the four edges is floored on its own, and the size is the distance between two floored
/// edges: a master of 607.2 and a stack of 404.8 that starts at 619.2 are 607 and 404 wide when each is floored apart, and the stack
/// still ends at 1023 in an area that ends at 1024.
pub(crate) fn tile(node: NodeId, x: f64, y: f64, width: f64, height: f64) -> Placement {
    let left = (x + EDGE_EPSILON).floor();
    let top = (y + EDGE_EPSILON).floor();
    Placement {
        tiled: true,
        bounds: Some(Rect::new(
            left,
            top,
            (x + width + EDGE_EPSILON).floor() - left,
            (y + height + EDGE_EPSILON).floor() - top,
        )),
        ..plain(node)
    }
}

/// The rectangle a tiling layout fills and the gap between its tiles: the area inset by the workspace's outer gap, and the layout's own
/// gap when it has one, else the workspace's inner gap.
pub(crate) fn geometry(own: Option<f64>, cx: &LayoutContext) -> (Rect, f64) {
    let outer = cx.gap_outer;
    let area = Rect::new(
        cx.area.x + outer,
        cx.area.y + outer,
        (cx.area.width - 2.0 * outer).max(0.0),
        (cx.area.height - 2.0 * outer).max(0.0),
    );
    (area, own.unwrap_or(cx.gap_inner))
}

/// Whether `value` is allowed by `value`'s bound.
pub(crate) fn within(desc: &ValueDesc, number: f64) -> bool {
    match desc.min {
        LowerBound::Free => true,
        LowerBound::AtLeast(min) => number >= min,
        LowerBound::Above(min) => number > min,
    }
}

/// The text a failed range check says.
pub(crate) fn range_text(desc: &ValueDesc) -> String {
    let what = if matches!(desc.ty, ValueShape::Int) {
        "a whole number"
    } else {
        "a finite number"
    };
    match desc.min {
        LowerBound::Free => what.to_owned(),
        LowerBound::AtLeast(min) => format!("{what} of at least {min}"),
        LowerBound::Above(min) => format!("{what} above {min}"),
    }
}
