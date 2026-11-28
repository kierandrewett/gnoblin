//! What the toolkit keeps for one generation: the canvases, how they are told apart, and the handle an object holds on its canvas.
//!
//! A canvas is a [`Widget`] in the generation's [`Canvases`]. The object the shell holds is a [`CanvasHandle`] that names it by a [`Cid`]; the
//! widget stays in the world until its object ends, so a canvas the compositor destroyed (or the shell did) still reads as it was
//! with `alive` false. Nothing here calls into the shell: every piece of work that does works on copies it took out of the world first,
//! because a closure or a hook may call back into the toolkit.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gnoblin_core::Timers;
use gnoblin_core::host::{Host, Subscription};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::ShellImpl;
use gnoblin_object::{Closure, MemberId, Obj};

/// The toolkit's own name for a canvas: never reused, and ordered by when the canvas was made, which is the order the frame updates
/// the roots in.
pub(crate) type Cid = u64;

/// The values of the enums of the catalog (`ui.toml`, `widgets.toml`), which are their places in it counted from one.
pub(crate) mod en {
    pub(crate) const PLACEMENT_FRAME: u32 = 1;
    pub(crate) const PLACEMENT_OVER: u32 = 2;
    pub(crate) const PLACEMENT_UNDER: u32 = 3;
    pub(crate) const DIRECTION_ROW: u32 = 1;
    pub(crate) const DIRECTION_COLUMN: u32 = 2;
    pub(crate) const DIRECTION_NONE: u32 = 3;
    pub(crate) const POINTER_PRESS: u32 = 1;
    pub(crate) const POINTER_RELEASE: u32 = 2;
    pub(crate) const POINTER_MOTION: u32 = 3;
    pub(crate) const POINTER_SCROLL: u32 = 4;
    pub(crate) const POINTER_ENTER: u32 = 5;
    pub(crate) const POINTER_LEAVE: u32 = 6;
    pub(crate) const BAR_TOP: u32 = 1;

    /// The word the compositor knows a placement by.
    pub(crate) fn placement_word(value: u32) -> &'static str {
        match value {
            PLACEMENT_OVER => "over",
            PLACEMENT_UNDER => "under",
            _ => "frame",
        }
    }

    /// The pointer kind a compositor event word names.
    pub(crate) fn pointer_kind(word: &str) -> Option<u32> {
        Some(match word {
            "press" => POINTER_PRESS,
            "release" => POINTER_RELEASE,
            "motion" => POINTER_MOTION,
            "scroll" => POINTER_SCROLL,
            "enter" => POINTER_ENTER,
            "leave" => POINTER_LEAVE,
            _ => return None,
        })
    }
}

/// Which class of the toolkit a canvas is, as the TypeScript toolkit's classes nested: a canvas, a box, and what derives from a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WidgetKind {
    Canvas,
    Box,
    Grid,
    Text,
    Icon,
    Entry,
    Clock,
    Bar,
    Toast,
    /// A check or a submenu arrow of a menu row: a box that draws lines.
    MenuMark,
    /// The rule between groups of menu entries.
    MenuSeparator,
    /// A row of a menu card.
    MenuItem,
    /// A column of rows of a menu.
    MenuPanel,
    /// The layer a menu is made on.
    Menu,
}

impl WidgetKind {
    /// The class's name in the catalog.
    pub(crate) fn class(self) -> &'static str {
        match self {
            WidgetKind::Canvas => "canvas",
            WidgetKind::Box => "box",
            WidgetKind::Grid => "grid",
            WidgetKind::Text => "text",
            WidgetKind::Icon => "icon",
            WidgetKind::Entry => "entry",
            WidgetKind::Clock => "clock",
            WidgetKind::Bar => "bar",
            WidgetKind::Toast => "toast",
            // A mark and a separator are boxes to a shell: it never makes one.
            WidgetKind::MenuMark | WidgetKind::MenuSeparator => "box",
            WidgetKind::MenuItem => "menu_item",
            WidgetKind::MenuPanel => "menu_panel",
            WidgetKind::Menu => "window_menu",
        }
    }

    /// The class's name as a shell's log knows it (the names the TypeScript toolkit's classes have).
    pub(crate) fn title(self) -> &'static str {
        match self {
            WidgetKind::Canvas => "Canvas",
            WidgetKind::Box => "Box",
            WidgetKind::Grid => "Grid",
            WidgetKind::Text => "Text",
            WidgetKind::Icon => "Icon",
            WidgetKind::Entry => "Entry",
            WidgetKind::Clock => "Clock",
            WidgetKind::Bar => "Bar",
            WidgetKind::Toast => "Toast",
            WidgetKind::MenuMark => "MenuMark",
            WidgetKind::MenuSeparator => "MenuSeparator",
            WidgetKind::MenuItem => "MenuItem",
            WidgetKind::MenuPanel => "MenuPanel",
            WidgetKind::Menu => "WindowMenu",
        }
    }

    /// Whether the class is a box or derives from one: what a parent box arranges, and what `fill` and `padding` mean to.
    pub(crate) fn is_box(self) -> bool {
        self != WidgetKind::Canvas
    }

    /// Whether the class is a text or derives from one.
    pub(crate) fn is_text(self) -> bool {
        matches!(self, WidgetKind::Text | WidgetKind::Clock)
    }
}

/// The properties last written to the compositor, or their initial values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WrittenProps {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) visible: bool,
    pub(crate) opacity: f64,
    pub(crate) secure: bool,
    pub(crate) input: bool,
    pub(crate) placement: u32,
}

impl Default for WrittenProps {
    fn default() -> WrittenProps {
        WrittenProps {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            visible: true,
            opacity: 1.0,
            secure: false,
            input: true,
            placement: en::PLACEMENT_FRAME,
        }
    }
}

/// What a box adds to a canvas.
#[derive(Clone, Debug)]
pub(crate) struct BoxPart {
    pub(crate) direction: u32,
    pub(crate) gap: f64,
    pub(crate) padding: f64,
    pub(crate) grow: f64,
    pub(crate) fill: String,
    pub(crate) radius: f64,
}

impl Default for BoxPart {
    fn default() -> BoxPart {
        BoxPart {
            direction: en::DIRECTION_NONE,
            gap: 0.0,
            padding: 0.0,
            grow: 0.0,
            fill: "#00000000".to_owned(),
            radius: 0.0,
        }
    }
}

/// The size of a string as the painter lays it out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) ascent: f64,
}

/// What a text adds to a box. `measured` is the answer for the font, size and text it was asked about, kept until a measurement
/// changes (`Ui::epoch`).
#[derive(Clone, Debug)]
pub(crate) struct TextPart {
    pub(crate) text: String,
    pub(crate) size: f64,
    pub(crate) color: String,
    pub(crate) font: String,
    pub(crate) ellipsize: bool,
    pub(crate) measured: Option<(String, u64, Metrics)>,
}

impl Default for TextPart {
    fn default() -> TextPart {
        TextPart {
            text: String::new(),
            size: DEFAULT_TEXT_SIZE,
            color: DEFAULT_TEXT_COLOUR.to_owned(),
            font: DEFAULT_FONT.to_owned(),
            ellipsize: false,
            measured: None,
        }
    }
}

/// What an icon adds to a box.
#[derive(Clone, Debug)]
pub(crate) struct IconPart {
    pub(crate) icon: String,
    pub(crate) size: f64,
    pub(crate) color: String,
}

impl Default for IconPart {
    fn default() -> IconPart {
        IconPart {
            icon: String::new(),
            size: DEFAULT_ICON_SIZE,
            color: DEFAULT_TEXT_COLOUR.to_owned(),
        }
    }
}

/// The default font name of a text.
pub(crate) const DEFAULT_FONT: &str = "sans";
/// The default text size in logical pixels.
pub(crate) const DEFAULT_TEXT_SIZE: f64 = 14.0;
/// The default icon size in logical pixels.
pub(crate) const DEFAULT_ICON_SIZE: f64 = 16.0;
/// The default text colour, matching the stock menus.
pub(crate) const DEFAULT_TEXT_COLOUR: &str = "#cdd6f4";

/// What an entry adds to a box: the model it edits, the three hidden texts it measures with, and how it looks.
#[derive(Clone)]
pub(crate) struct EntryPart {
    pub(crate) model: Obj,
    pub(crate) line: Cid,
    pub(crate) head: Cid,
    pub(crate) mark: Cid,
    pub(crate) secret: bool,
    pub(crate) size: f64,
    pub(crate) color: String,
    pub(crate) font: String,
    pub(crate) selection_fill: String,
    pub(crate) on_submit: Option<Closure>,
    pub(crate) shift: bool,
    pub(crate) control: bool,
    /// The launcher this entry is, if it is one.
    pub(crate) launcher: Option<crate::shell::LauncherPart>,
}

/// What a clock adds to a text.
#[derive(Clone)]
pub(crate) struct ClockPart {
    pub(crate) format: String,
    pub(crate) now: Option<Closure>,
}

/// What a bar adds to a box.
#[derive(Clone)]
pub(crate) struct BarPart {
    pub(crate) edge: u32,
    pub(crate) hide_when: Option<Closure>,
    pub(crate) output: u64,
}

/// What a toast adds to a box.
#[derive(Clone)]
pub(crate) struct ToastPart {
    pub(crate) message: Cid,
    pub(crate) fade_in: f64,
    pub(crate) hold: f64,
    pub(crate) fade_out: f64,
    pub(crate) clock: Option<Closure>,
    /// The reading of the clock when the card first updated; the card's age counts from it.
    pub(crate) start: Option<f64>,
}

/// What only some canvases have. The frame's part is the largest and the rarest: a window has a handful of them.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Special {
    None,
    Entry(Box<EntryPart>),
    Clock(ClockPart),
    Bar(BarPart),
    Toast(ToastPart),
    Frame(crate::frame::FramePart),
    Mark(crate::menus::MarkPart),
    Item(Box<crate::menus::ItemPart>),
    Panel(Box<crate::menus::PanelPart>),
    Menu(Box<crate::menus::MenuPart>),
    Shell(crate::shell::ShellPart),
    Password(Box<crate::lock::PasswordPart>),
    Reposition(Box<crate::policy::GrabPart>),
}

/// The members of the `widget` interface, by id: what a shell that implements it is asked.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WidgetIds {
    pub(crate) refresh: MemberId,
    pub(crate) arrange: MemberId,
    pub(crate) paint: MemberId,
    pub(crate) natural_width: MemberId,
    pub(crate) natural_height: MemberId,
    pub(crate) on_pointer: MemberId,
    pub(crate) on_key: MemberId,
}

/// A `widget` a shell implemented for a canvas: the interface's object (held, so it stays), and what the shell defined of it.
#[derive(Clone)]
pub(crate) struct Behaviour {
    pub(crate) obj: Obj,
    pub(crate) shell: Rc<ShellImpl>,
}

/// One canvas.
pub(crate) struct Widget {
    pub(crate) obj: Obj,
    pub(crate) node: u64,
    pub(crate) kind: WidgetKind,
    pub(crate) alive: bool,
    pub(crate) parent: Option<Cid>,
    pub(crate) children: Vec<Cid>,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) visible: bool,
    pub(crate) opacity: f64,
    pub(crate) secure: bool,
    pub(crate) input: bool,
    pub(crate) placement: u32,
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
    pub(crate) written: WrittenProps,
    /// The bytes last written as the canvas's `content`.
    pub(crate) content: Option<Vec<u8>>,
    /// How many steps of the canvas's update pass have failed.
    pub(crate) failures: u32,
    /// The failures of its steps the log has been told of, by step: a step that fails the same way again says nothing more.
    pub(crate) reported: Vec<(&'static str, String)>,
    pub(crate) behaviour: Option<Behaviour>,
    pub(crate) on_pointer: Option<Closure>,
    pub(crate) on_key: Option<Closure>,
    pub(crate) b: BoxPart,
    pub(crate) t: TextPart,
    pub(crate) i: IconPart,
    pub(crate) sp: Special,
}

impl Widget {
    pub(crate) fn new(kind: WidgetKind, node: u64) -> Widget {
        Widget {
            obj: 0,
            node,
            kind,
            alive: true,
            parent: None,
            children: Vec::new(),
            x: 0.0,
            y: 0.0,
            visible: true,
            opacity: 1.0,
            secure: false,
            input: true,
            placement: en::PLACEMENT_FRAME,
            width: None,
            height: None,
            written: WrittenProps::default(),
            content: None,
            failures: 0,
            reported: Vec::new(),
            behaviour: None,
            on_pointer: None,
            on_key: None,
            b: BoxPart::default(),
            t: TextPart::default(),
            i: IconPart::default(),
            sp: Special::None,
        }
    }
}

/// The canvases of a generation, and what the whole toolkit shares between them.
#[derive(Default)]
pub(crate) struct Canvases {
    next: Cid,
    pub(crate) widgets: BTreeMap<Cid, Widget>,
    pub(crate) by_node: HashMap<u64, Cid>,
    /// The canvas that held the keyboard focus when the seat's focus last moved, for `focus_changed`.
    pub(crate) focus_seen: Option<Cid>,
    /// Makes every bar give way: the shell's question about an output.
    pub(crate) bars_cover: Option<Closure>,
    /// The primary output, for a bar that names none.
    pub(crate) bars_primary: Option<Closure>,
}

impl Canvases {
    /// Takes the next name.
    pub(crate) fn mint(&mut self) -> Cid {
        self.next += 1;
        self.next
    }

    pub(crate) fn get(&self, cid: Cid) -> Option<&Widget> {
        self.widgets.get(&cid)
    }

    pub(crate) fn get_mut(&mut self, cid: Cid) -> Option<&mut Widget> {
        self.widgets.get_mut(&cid)
    }

    /// The canvases that are alive and have no canvas for a parent, in the order they were made: what a frame updates.
    pub(crate) fn roots(&self) -> Vec<Cid> {
        self.widgets
            .iter()
            .filter(|(_, widget)| widget.alive && widget.parent.is_none())
            .map(|(cid, _)| *cid)
            .collect()
    }

    /// How many canvases are alive.
    pub(crate) fn live(&self) -> usize {
        self.widgets.values().filter(|widget| widget.alive).count()
    }

    /// `cid` and everything under it, parents first.
    pub(crate) fn subtree(&self, cid: Cid) -> Vec<Cid> {
        let mut out = Vec::new();
        let mut stack = vec![cid];
        while let Some(next) = stack.pop() {
            if let Some(widget) = self.widgets.get(&next) {
                out.push(next);
                stack.extend(widget.children.iter().rev().copied());
            }
        }
        out
    }

    /// Whether `ancestor` is `cid` or holds it somewhere beneath.
    pub(crate) fn contains(&self, ancestor: Cid, cid: Cid) -> bool {
        let mut at = Some(cid);
        while let Some(next) = at {
            if next == ancestor {
                return true;
            }
            at = self.widgets.get(&next).and_then(|widget| widget.parent);
        }
        false
    }
}

/// A frame hook or a request hook, by the id its `ui_hook` removes it with.
pub(crate) struct Hook {
    pub(crate) id: u64,
    pub(crate) run: Closure,
}

/// The toolkit of one generation.
pub(crate) struct Ui {
    pub(crate) ctx: GnoblinCtx,
    pub(crate) g: Host,
    pub(crate) w: RefCell<Canvases>,
    /// A turn is posted and has not run.
    pub(crate) pending: Cell<bool>,
    /// The timer that asks for the next turn of a time-based part.
    pub(crate) timer: RefCell<Option<Subscription>>,
    /// The core's timers, held from the first read of the core's clock (see `Ui::core_now`).
    pub(crate) core_timers: RefCell<Option<Rc<Timers>>>,
    /// A measurement changed since this was read: what a text measured is stale (see `text::epoch`).
    pub(crate) epoch: Cell<u64>,
    pub(crate) frame_hooks: RefCell<Vec<Hook>>,
    pub(crate) request_hooks: RefCell<Vec<Hook>>,
    pub(crate) next_hook: Cell<u64>,
    pub(crate) ids: RefCell<Option<Ids>>,
    /// The window frames, the decorations and what they keep (`crate::frame`).
    pub(crate) frames: RefCell<crate::frame::FrameWorld>,
    /// The window menu: what it was started with, and the menu that is open.
    pub(crate) menus: RefCell<crate::menus::State>,
    /// The not-responding alerts, the launcher and the overview (`crate::shell`).
    pub(crate) shell: RefCell<crate::shell::ShellWorld>,
    /// The stock lock screen (`crate::lock`).
    pub(crate) lock: RefCell<crate::lock::LockWorld>,
    /// The window policy: focus, the requests of windows and the keyboard move and resize (`crate::policy`).
    pub(crate) policy: RefCell<crate::policy::PolicyWorld>,
}

/// The ids of the classes and members of the object library that the toolkit calls by name, resolved once.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ids {
    pub(crate) widget: WidgetIds,
    pub(crate) model: ModelIds,
}

/// The members of `entry_model` the entry calls.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModelIds {
    pub(crate) text: MemberId,
    pub(crate) caret: MemberId,
    pub(crate) anchor: MemberId,
    pub(crate) insert: MemberId,
    pub(crate) backspace: MemberId,
    pub(crate) delete: MemberId,
    pub(crate) left: MemberId,
    pub(crate) right: MemberId,
    pub(crate) home: MemberId,
    pub(crate) end: MemberId,
    pub(crate) select_all: MemberId,
    pub(crate) before: MemberId,
}

/// The handle an object of the toolkit holds: the generation's toolkit and the canvas. When the object ends, the canvas goes with it.
pub(crate) struct CanvasHandle {
    pub(crate) ui: Rc<Ui>,
    pub(crate) cid: Cid,
}

impl Drop for CanvasHandle {
    fn drop(&mut self) {
        self.ui.object_ended(self.cid);
    }
}
