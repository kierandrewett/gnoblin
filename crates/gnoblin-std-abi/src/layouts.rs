//! The built-in layouts of `libgnoblin-std`, declared once: each kind, each option it takes and each action it answers to.
//!
//! A shell that wants a tiling layout does not write one. It names a kind ([`STD_BUILTIN_DWINDLE`]) and a typed property set
//! (`workspace_set_layout_kind`), and the library runs the algorithm. This table is the single definition of that surface:
//!
//! - the library validates every option and argument against it and takes its defaults from it, so the list below is what
//!   the library accepts and nothing else;
//! - `cargo xtask gen-sdks` reads it, so the typed wrapper of every language (`DwindleLayout`, `new DwindleLayout({ ratio })`,
//!   `DwindleLayout(ratio=0.6)`) is generated from it and never written by hand;
//! - the design notes' tables (`design/modular-framework.md` section 22) are written from it.
//!
//! An option or an action is named by its text, in `snake_case`, and carried by a [`GnoblinStdOption`]: the name, the
//! [`STD_VALUE_*`](crate::STD_VALUE_BOOL) kind and the value. An enumeration travels as the number of its member.
//!
//! Adding a kind, an option or an action is one entry here and the algorithm in `gnoblin-std-impl`; no SDK is touched.

use crate::{
    STD_BINDING_MOVE, STD_BINDING_RESIZE, STD_BINDING_SPLIT_RESIZE, STD_BINDING_SWAP, STD_BUILTIN_ARC,
    STD_BUILTIN_CAROUSEL, STD_BUILTIN_DWINDLE, STD_BUILTIN_GRID, STD_BUILTIN_MASTER_STACK, STD_BUILTIN_MONOCLE,
    STD_BUILTIN_STRIP,
};

/// One member of an enumeration option: its text name, the number that travels, and what it means.
#[derive(Clone, Copy, Debug)]
pub struct EnumMember {
    /// The member's name in `snake_case`.
    pub name: &'static str,
    /// The number that carries the member in a [`GnoblinStdOption`](crate::GnoblinStdOption).
    pub value: u32,
    /// What the member means.
    pub doc: &'static str,
}

/// What a value is.
#[derive(Clone, Copy, Debug)]
pub enum ValueShape {
    /// `STD_VALUE_BOOL`.
    Bool,
    /// `STD_VALUE_NUMBER`: a finite number.
    Number,
    /// `STD_VALUE_NUMBER` holding a whole number.
    Int,
    /// `STD_VALUE_ENUM`: the number of one of the members.
    Enum(&'static [EnumMember]),
    /// `STD_VALUE_NODE`: the id of a window.
    Node,
    /// `STD_VALUE_TEXT`: UTF-8 text.
    Text,
}

/// How low a number may go.
#[derive(Clone, Copy, Debug)]
pub enum LowerBound {
    /// No limit (the number is finite).
    Free,
    /// This or more.
    AtLeast(f64),
    /// More than this.
    Above(f64),
}

/// What a value is when none is given.
#[derive(Clone, Copy, Debug)]
pub enum Initial {
    /// Not set (the option is `optional`), or no default (an argument that must be given).
    None,
    /// A boolean.
    Bool(bool),
    /// A number, or the number of an enumeration member.
    Number(f64),
    /// Text.
    Text(&'static str),
}

/// The type of one option, argument or result.
#[derive(Clone, Copy, Debug)]
pub struct ValueDesc {
    /// What it is.
    pub ty: ValueShape,
    /// It may be absent (`STD_VALUE_NONE`).
    pub optional: bool,
    /// For a number: how low it may go.
    pub min: LowerBound,
}

/// An option of a kind, or an argument of an action.
#[derive(Clone, Copy, Debug)]
pub struct OptionDesc {
    /// The name, in `snake_case`.
    pub name: &'static str,
    /// Its type.
    pub value: ValueDesc,
    /// What it is when the shell gives none. An action's argument has [`Initial::None`]: an optional argument that is not
    /// given is absent, and a required one is an error.
    pub default: Initial,
    /// What it means.
    pub doc: &'static str,
}

/// An action of a kind: something a shell asks of the layout of a workspace. An action works on the window that has the
/// keyboard when it needs one.
#[derive(Clone, Copy, Debug)]
pub struct ActionDesc {
    /// The name, in `snake_case`.
    pub name: &'static str,
    /// The arguments, in order.
    pub args: &'static [OptionDesc],
    /// What it answers, if anything. A boolean answer says whether the action changed the layout.
    pub returns: Option<ValueDesc>,
    /// What it does.
    pub doc: &'static str,
}

/// A kind of built-in layout.
#[derive(Clone, Copy, Debug)]
pub struct LayoutKindDesc {
    /// The `STD_BUILTIN_*` number.
    pub id: u32,
    /// The name in `snake_case`.
    pub name: &'static str,
    /// What the layout does.
    pub doc: &'static str,
    /// The options.
    pub options: &'static [OptionDesc],
    /// The actions.
    pub actions: &'static [ActionDesc],
    /// The layout answers `workspace_find_split`: a drag can move one of its dividers.
    pub dividers: bool,
}

const fn value(ty: ValueShape) -> ValueDesc {
    ValueDesc {
        ty,
        optional: false,
        min: LowerBound::Free,
    }
}

const fn maybe(ty: ValueShape) -> ValueDesc {
    ValueDesc {
        ty,
        optional: true,
        min: LowerBound::Free,
    }
}

const fn at_least(ty: ValueShape, min: f64) -> ValueDesc {
    ValueDesc {
        ty,
        optional: false,
        min: LowerBound::AtLeast(min),
    }
}

const fn above(ty: ValueShape, min: f64) -> ValueDesc {
    ValueDesc {
        ty,
        optional: false,
        min: LowerBound::Above(min),
    }
}

const fn arg(name: &'static str, value: ValueDesc, doc: &'static str) -> OptionDesc {
    OptionDesc {
        name,
        value,
        default: Initial::None,
        doc,
    }
}

const fn option(name: &'static str, value: ValueDesc, default: Initial, doc: &'static str) -> OptionDesc {
    OptionDesc {
        name,
        value,
        default,
        doc,
    }
}

const fn action(
    name: &'static str,
    args: &'static [OptionDesc],
    returns: Option<ValueDesc>,
    doc: &'static str,
) -> ActionDesc {
    ActionDesc {
        name,
        args,
        returns,
        doc,
    }
}

const fn member(name: &'static str, value: u32, doc: &'static str) -> EnumMember {
    EnumMember { name, value, doc }
}

/// Where the master area sits in a master-stack layout.
pub const ORIENTATION: &[EnumMember] = &[
    member("left", 0, "Master on the left, the stack to its right."),
    member("right", 1, "Master on the right."),
    member("top", 2, "Master on top, the stack below."),
    member("bottom", 3, "Master at the bottom."),
];

/// How a grid fills its cells.
pub const GRID_FILL: &[EnumMember] = &[
    member("row", 0, "Left to right, then the next row."),
    member("column", 1, "Top to bottom, then the next column."),
];

/// How a dwindle's splits pick their direction.
pub const SPLIT_MODE: &[EnumMember] = &[
    member("aspect", 0, "The shape of the cell, every compute."),
    member("horizontal", 1, "Side by side, for the splits this layout makes."),
    member("vertical", 2, "One above the other, for the splits this layout makes."),
];

/// Where a new window goes in the cell a dwindle splits.
pub const FORCE_SPLIT: &[EnumMember] = &[
    member(
        "by_pointer",
        0,
        "On the side of the cell the pointer is on, and second when there is no pointer.",
    ),
    member("first", 1, "First (left or above)."),
    member("second", 2, "Second (right or below)."),
];

/// Which window of a new dwindle split gets the layout's ratio.
pub const SPLIT_BIAS: &[EnumMember] = &[
    member("first", 0, "The first one (top or left)."),
    member("opened", 1, "The window the new one was opened on, wherever it sits."),
];

/// Where a dwindle reads the pointer from.
pub const POINTER_SOURCE: &[EnumMember] = &[
    member("none", 0, "There is no pointer: the options that read it do nothing."),
    member("seat", 1, "The seat's pointer."),
    member(
        "fixed",
        2,
        "The point `pointer_x`, `pointer_y`, which a shell may move with `workspace_layout_set`.",
    ),
];

/// Where a dwindle's preselection puts the next window.
pub const PRESELECT: &[EnumMember] = &[
    member("left", 0, "To the left of the window it splits."),
    member("right", 1, "To the right of it."),
    member("up", 2, "Above it."),
    member("down", 3, "Below it."),
    member("none", 4, "No choice."),
];

const WINDOW_OPTION_GAP: OptionDesc = option(
    "gap",
    maybe(ValueShape::Number),
    Initial::None,
    "The gap between tiles, replacing the workspace's inner gap; at least 0.",
);

const MASTER_STACK: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_MASTER_STACK,
    name: "master_stack",
    doc: "One master area and a stack: the first `master_count` windows share the master area, the rest share the stack. \
          The divider between them can be dragged.",
    options: &[
        option(
            "ratio",
            value(ValueShape::Number),
            Initial::Number(0.5),
            "The share of the span the master area takes, held to 0.1 to 0.9 when it is used.",
        ),
        option(
            "master_count",
            at_least(ValueShape::Int, 0.0),
            Initial::Number(1.0),
            "How many windows share the master area; 0 makes everything stack.",
        ),
        WINDOW_OPTION_GAP,
        option(
            "orientation",
            value(ValueShape::Enum(ORIENTATION)),
            Initial::Number(0.0),
            "Which side the master area is on.",
        ),
    ],
    actions: &[],
    dividers: true,
};

const GRID: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_GRID,
    name: "grid",
    doc: "Every window the same size, in rows and columns.",
    options: &[
        option(
            "columns",
            at_least(ValueShape::Int, 0.0),
            Initial::Number(0.0),
            "The number of columns; 0 is as square as the number of windows allows.",
        ),
        option(
            "fill",
            value(ValueShape::Enum(GRID_FILL)),
            Initial::Number(0.0),
            "How the cells fill.",
        ),
        WINDOW_OPTION_GAP,
    ],
    actions: &[],
    dividers: false,
};

const MONOCLE: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_MONOCLE,
    name: "monocle",
    doc: "One window fills the area and the rest are hidden: the one that has the keyboard, else the first.",
    options: &[],
    actions: &[],
    dividers: false,
};

const DWINDLE: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_DWINDLE,
    name: "dwindle",
    doc: "A binary split tree that places windows as Hyprland's dwindle layout does: a new window splits the cell of the \
          focused window in two. Ratios are fractions of the first child, 0.1 to 0.9 (Hyprland's 1.0 is 0.5 here).",
    options: &[
        option(
            "split",
            value(ValueShape::Enum(SPLIT_MODE)),
            Initial::Number(0.0),
            "Split direction for each new window.",
        ),
        option(
            "ratio",
            value(ValueShape::Number),
            Initial::Number(0.5),
            "The first child's share of every split that was not dragged or set, held to 0.1 to 0.9.",
        ),
        WINDOW_OPTION_GAP,
        option(
            "force_split",
            value(ValueShape::Enum(FORCE_SPLIT)),
            Initial::Number(0.0),
            "Where a new window goes in the cell it splits.",
        ),
        option(
            "preserve_split",
            value(ValueShape::Bool),
            Initial::Bool(false),
            "Keeps each split's direction as it was made instead of following the shape of its cell.",
        ),
        option(
            "smart_split",
            value(ValueShape::Bool),
            Initial::Bool(false),
            "Chooses the direction and side of a new window by the pointer's place in the cell it splits. Implies \
             `preserve_split`. With no pointer it does nothing.",
        ),
        option(
            "smart_resizing",
            value(ValueShape::Bool),
            Initial::Bool(true),
            "Which divider a split-resize drag moves. True: the divider nearest the press. False: both dividers that \
             bound the window by its place in the tree.",
        ),
        option(
            "permanent_direction_override",
            value(ValueShape::Bool),
            Initial::Bool(false),
            "Keeps a `preselect` choice for every new window until it is cleared, instead of once.",
        ),
        option(
            "split_width_multiplier",
            above(ValueShape::Number, 0.0),
            Initial::Number(1.0),
            "A split goes side by side when the cell's width is at least its height times this, held to 0.1 to 3.0.",
        ),
        option(
            "use_active_for_splits",
            value(ValueShape::Bool),
            Initial::Bool(true),
            "True: a new window splits the focused window. False: it splits the window under the pointer, or the first \
             window when there is no pointer.",
        ),
        option(
            "split_bias",
            value(ValueShape::Enum(SPLIT_BIAS)),
            Initial::Number(0.0),
            "Which window of a new split gets `ratio`.",
        ),
        option(
            "pointer",
            value(ValueShape::Enum(POINTER_SOURCE)),
            Initial::Number(0.0),
            "Where the pointer is read from. `force_split` by pointer, `smart_split` and `use_active_for_splits` read it \
             when a window arrives.",
        ),
        option(
            "pointer_x",
            maybe(ValueShape::Number),
            Initial::None,
            "With `pointer` fixed: the point's x.",
        ),
        option(
            "pointer_y",
            maybe(ValueShape::Number),
            Initial::None,
            "With `pointer` fixed: the point's y.",
        ),
    ],
    actions: &[
        action(
            "toggle_split",
            &[],
            Some(value(ValueShape::Bool)),
            "Turns the split above the focused window from side by side to top and bottom or back. A split that follows \
             the shape of its cell takes its direction from the cell again at once, so this needs `preserve_split`. \
             False when no window with a split above it has focus.",
        ),
        action(
            "swap_split",
            &[],
            Some(value(ValueShape::Bool)),
            "Swaps the two halves of the split above the focused window. False when there is none.",
        ),
        action(
            "rotate_split",
            &[arg(
                "angle",
                value(ValueShape::Number),
                "Degrees clockwise, a multiple of 90; negative turns the other way.",
            )],
            Some(value(ValueShape::Bool)),
            "Turns the split above the focused window: the first half goes from the left to the top to the right to the \
             bottom. 180 swaps the halves. False when there is no split above the focused window.",
        ),
        action(
            "move_to_root",
            &[arg(
                "stable",
                value(ValueShape::Bool),
                "Keeps the window on the same side of the screen; otherwise it goes to the other.",
            )],
            Some(value(ValueShape::Bool)),
            "Makes the focused window a direct child of the root split: it takes the place of the root's other half, \
             which goes where the window was. False when the window is alone or already a child of the root.",
        ),
        action(
            "split_ratio",
            &[arg(
                "amount",
                value(ValueShape::Number),
                "A fraction: 0.1 moves the divider a tenth of the split.",
            )],
            Some(value(ValueShape::Bool)),
            "Changes the share of the split above the focused window by `amount`, held to 0.1 to 0.9. The split keeps \
             the ratio, whatever the layout's `ratio` becomes. False when the window has no split above it.",
        ),
        action(
            "set_split_ratio",
            &[arg("ratio", value(ValueShape::Number), "The share of the first child.")],
            Some(value(ValueShape::Bool)),
            "Sets the share of the split above the focused window to `ratio`, held to 0.1 to 0.9.",
        ),
        action(
            "preselect",
            &[arg(
                "direction",
                value(ValueShape::Enum(PRESELECT)),
                "Where the next window opens, relative to the window it splits.",
            )],
            None,
            "Sets where the next window opens: left or up puts it first with a split that goes that way, right or down \
             second. The split keeps its direction whatever its cell's shape. The choice is used once, unless \
             `permanent_direction_override` is set; none clears it.",
        ),
        action(
            "preselected",
            &[],
            Some(value(ValueShape::Enum(PRESELECT))),
            "The direction `preselect` set for the next window, or none.",
        ),
        action(
            "pseudotile",
            &[
                arg(
                    "width",
                    maybe(ValueShape::Number),
                    "The size to keep, above 0; with `height`.",
                ),
                arg(
                    "height",
                    maybe(ValueShape::Number),
                    "The size to keep, above 0; with `width`.",
                ),
            ],
            Some(value(ValueShape::Bool)),
            "Toggles pseudotiling of the focused window: it keeps its own size, centred in its cell and held to the \
             cell's size, instead of filling the cell. The size is the one given, else the window's own size with its \
             frame, else the size its cell has at the next compute. Answers whether the window is pseudotiled after the \
             call; false also when no window with a tile has focus.",
        ),
        action(
            "is_pseudotiled",
            &[arg("client", value(ValueShape::Node), "The window.")],
            Some(value(ValueShape::Bool)),
            "Whether the window is pseudotiled.",
        ),
    ],
    dividers: true,
};

const STRIP: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_STRIP,
    name: "strip",
    doc: "One column per window in a horizontal strip, with the focus kept in view. Columns sit edge to edge with no gap \
          or margin. The view follows the focus unless `scroll_to` pinned it; a pin ends when the focused window \
          changes. A new window opens right after the focused column, and the layout hands the keyboard to a neighbour \
          when a window closes.",
    options: &[option(
        "column",
        value(ValueShape::Number),
        Initial::Number(0.5),
        "A column's width as a fraction of the strip's, held to 0.1 to 1.",
    )],
    actions: &[
        action(
            "scroll_to",
            &[arg(
                "offset",
                value(ValueShape::Number),
                "Pixels from the strip's start.",
            )],
            None,
            "Scrolls the view to `offset` pixels and keeps it there until the focused window changes.",
        ),
        action(
            "focus_left",
            &[],
            None,
            "Moves the keyboard one column left; at the first column nothing happens.",
        ),
        action(
            "focus_right",
            &[],
            None,
            "Moves the keyboard one column right; at the last column nothing happens.",
        ),
        action(
            "pin",
            &[],
            Some(maybe(ValueShape::Number)),
            "The view offset in pixels that `scroll_to` pinned, or absent while the view follows the focus.",
        ),
    ],
    dividers: false,
};

const ARC: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_ARC,
    name: "arc",
    doc: "Windows around the viewer on an arc, the focused window at the front. Window `i` is at the angle \
          `(i - selected) * spacing`; its offset is `(sin(angle) * radius, 0)`, its depth `-cos(angle) * radius` and it is \
          turned `-angle` about the vertical axis.",
    options: &[
        option(
            "radius",
            above(ValueShape::Number, 0.0),
            Initial::Number(1200.0),
            "The distance from the centre of the arc to each window, in scene units.",
        ),
        option(
            "spacing",
            value(ValueShape::Number),
            Initial::Number(0.35),
            "The angle between neighbouring windows, in radians.",
        ),
    ],
    actions: &[],
    dividers: false,
};

const CAROUSEL: LayoutKindDesc = LayoutKindDesc {
    id: STD_BUILTIN_CAROUSEL,
    name: "carousel",
    doc: "Cards on a perspective ring, the focused window at the front; with no focus among the windows the first is. \
          Each placement is the ring's projected rectangle with the ring's depth and angle.",
    options: RING_OPTIONS,
    actions: &[action(
        "rotate",
        &[arg(
            "steps",
            value(ValueShape::Int),
            "How many cards; negative goes the other way.",
        )],
        None,
        "Moves the keyboard `steps` cards round the ring (the workspace's windows in its order), wrapping at both ends. \
         Nothing happens while no window has the keyboard.",
    )],
    dividers: false,
};

/// The options of the ring: [`CAROUSEL`] lays its cards out with them, and `layout_ring` takes the same set.
pub const RING_OPTIONS: &[OptionDesc] = &[
    option(
        "fov",
        above(ValueShape::Number, 0.0),
        Initial::Number(40.0),
        "Vertical field of view in degrees.",
    ),
    option(
        "card_width",
        above(ValueShape::Number, 0.0),
        Initial::Number(0.36),
        "A card's width as a fraction of the area's.",
    ),
    option(
        "card_height",
        above(ValueShape::Number, 0.0),
        Initial::Number(0.6),
        "A card's height as a fraction of the area's.",
    ),
];

/// Every built-in layout kind, in the order of its `STD_BUILTIN_*` number.
pub const LAYOUT_KINDS: &[LayoutKindDesc] = &[MASTER_STACK, GRID, MONOCLE, DWINDLE, STRIP, ARC, CAROUSEL];

/// The kind with the number `id`.
pub fn layout_kind(id: u32) -> Option<&'static LayoutKindDesc> {
    LAYOUT_KINDS.iter().find(|kind| kind.id == id)
}

impl LayoutKindDesc {
    /// The option called `name`.
    pub fn option(&self, name: &str) -> Option<&'static OptionDesc> {
        self.options.iter().find(|option| option.name == name)
    }

    /// The action called `name`.
    pub fn action(&self, name: &str) -> Option<&'static ActionDesc> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// How a resize drag picks the side or corner it takes hold of.
pub const RESIZE_GRIP: &[EnumMember] = &[
    member("quadrant", 0, "The corner of the quadrant the press landed in."),
    member("nearest", 1, "The edge nearest the press."),
    member("top_left", 2, "Always the top left corner."),
    member("top_right", 3, "Always the top right corner."),
    member("bottom_left", 4, "Always the bottom left corner."),
    member("bottom_right", 5, "Always the bottom right corner."),
];

/// A pointer binding the library runs itself, so a shell of any language gets the same drags.
#[derive(Clone, Copy, Debug)]
pub struct BindingKindDesc {
    /// The `STD_BINDING_*` number.
    pub id: u32,
    /// The name in `snake_case`.
    pub name: &'static str,
    /// What the binding does.
    pub doc: &'static str,
    /// The options. Every binding has `trigger` and `priority` first.
    pub options: &'static [OptionDesc],
}

const fn text(default: &'static str) -> Initial {
    Initial::Text(default)
}

/// Declares the options of a pointer binding: the chord it claims and where it stands among the bindings of that chord.
macro_rules! binding_options {
    ($trigger:literal $(, $more:expr)* $(,)?) => {
        &[
            option(
                "trigger",
                value(ValueShape::Text),
                text($trigger),
                "The pointer chord that starts the binding: modifiers and a button, `Super+Button1`.",
            ),
            option(
                "priority",
                value(ValueShape::Number),
                Initial::Number(0.0),
                "Among the bindings of one chord the higher priority is asked first, and one that declines the drag leaves it to \
                 the next. Two bindings of one chord and one priority are refused.",
            ),
            $($more),*
        ]
    };
}

const MOVE: BindingKindDesc = BindingKindDesc {
    id: STD_BINDING_MOVE,
    name: "move",
    doc: "Drags the window under the pointer with it. The window's frame is held inside the work area as the seat's \
          `drag_constraint` says, the rectangle it floats at is remembered, and a drop on a screen edge snaps the window \
          there when the seat's `edge_tiling` is on. A window that is fullscreen or maximised declines.",
    options: binding_options!(
        "Super+Button1",
        option(
            "snap",
            value(ValueShape::Bool),
            Initial::Bool(true),
            "Whether a drop on a screen edge snaps the window (left and right take half the work area, top all of it).",
        ),
        option(
            "snap_margin",
            at_least(ValueShape::Number, 0.0),
            Initial::Number(4.0),
            "How near a screen edge, in pixels, the pointer must be for a drop to snap the window.",
        ),
    ),
};

const RESIZE: BindingKindDesc = BindingKindDesc {
    id: STD_BINDING_RESIZE,
    name: "resize",
    doc: "Resizes the window under the pointer by the side or corner the grip names. A resize stops at the client's own minimum \
          and maximum size (100 by 80 below when it has none) and the floating rectangle is remembered.",
    options: binding_options!(
        "Super+Button3",
        option(
            "grip",
            value(ValueShape::Enum(RESIZE_GRIP)),
            Initial::Number(0.0),
            "Which side or corner a resize takes hold of.",
        ),
    ),
};

const SWAP: BindingKindDesc = BindingKindDesc {
    id: STD_BINDING_SWAP,
    name: "swap",
    doc: "Swaps a tiled window with the tile it is dropped on: the pointer takes hold of a tile and the release decides.",
    options: binding_options!("Super+Button1"),
};

const SPLIT_RESIZE: BindingKindDesc = BindingKindDesc {
    id: STD_BINDING_SPLIT_RESIZE,
    name: "split_resize",
    doc: "Moves the divider of a tiling layout nearest the press by as far as the pointer travels, so the tiles on both sides \
          follow it while the pointer is down. The layout decides which divider that is; one with no divider (a grid, a monocle) \
          declines. A release keeps the ratio, a cancel puts it back. When the layout hands back a second divider on the other \
          axis the same drag moves both.",
    options: binding_options!("Super+Button3"),
};

/// Every pointer binding the library runs itself, in the order of its `STD_BINDING_*` number.
pub const BINDING_KINDS: &[BindingKindDesc] = &[MOVE, RESIZE, SWAP, SPLIT_RESIZE];

/// The binding kind with the number `id`.
pub fn binding_kind(id: u32) -> Option<&'static BindingKindDesc> {
    BINDING_KINDS.iter().find(|kind| kind.id == id)
}

impl BindingKindDesc {
    /// The option called `name`.
    pub fn option(&self, name: &str) -> Option<&'static OptionDesc> {
        self.options.iter().find(|option| option.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_numbered_in_order_from_one() {
        for (index, kind) in LAYOUT_KINDS.iter().enumerate() {
            assert_eq!(kind.id as usize, index + 1, "{}", kind.name);
        }
    }

    #[test]
    fn bindings_are_numbered_from_one_and_start_with_a_trigger_and_a_priority() {
        for (index, kind) in BINDING_KINDS.iter().enumerate() {
            assert_eq!(kind.id as usize, index + 1, "{}", kind.name);
            assert_eq!(kind.options[0].name, "trigger", "{}", kind.name);
            assert_eq!(kind.options[1].name, "priority", "{}", kind.name);
        }
    }

    #[test]
    fn names_are_unique_snake_case() {
        let ok = |name: &str| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        let mut kinds: Vec<&str> = LAYOUT_KINDS.iter().map(|kind| kind.name).collect();
        for kind in LAYOUT_KINDS {
            assert!(ok(kind.name), "{}", kind.name);
            let mut names: Vec<&str> = kind.options.iter().map(|o| o.name).collect();
            names.extend(kind.actions.iter().map(|a| a.name));
            let all = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), all, "{}: an option and an action share a name", kind.name);
            for name in &names {
                assert!(ok(name), "{}.{name}", kind.name);
            }
        }
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), LAYOUT_KINDS.len());
    }

    #[test]
    fn every_default_fits_its_type() {
        for kind in LAYOUT_KINDS {
            for option in kind.options {
                match (option.value.ty, option.default) {
                    (_, Initial::None) => {
                        assert!(option.value.optional, "{}.{} has no default", kind.name, option.name)
                    }
                    (ValueShape::Bool, Initial::Bool(_)) | (ValueShape::Text, Initial::Text(_)) => {}
                    (ValueShape::Number | ValueShape::Int, Initial::Number(n)) => match option.value.min {
                        LowerBound::Free => {}
                        LowerBound::AtLeast(min) => assert!(n >= min, "{}.{}", kind.name, option.name),
                        LowerBound::Above(min) => assert!(n > min, "{}.{}", kind.name, option.name),
                    },
                    (ValueShape::Enum(members), Initial::Number(n)) => {
                        assert!(
                            members.iter().any(|m| f64::from(m.value) == n),
                            "{}.{}",
                            kind.name,
                            option.name
                        );
                    }
                    (ty, default) => panic!("{}.{}: {default:?} is not a {ty:?}", kind.name, option.name),
                }
            }
        }
    }
}
