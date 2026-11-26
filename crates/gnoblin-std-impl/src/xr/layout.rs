//! A spatial layout (the ring of a room and the bindings that act on its panels), the collection of those bindings, and one binding.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_object::server::ShellImpl;
use gnoblin_object::{ErrorCode, Fail, Obj, Res};

use super::ring::RingParams;
use super::trigger::{self, ControllerTrigger};

/// The ring of a layout as a script gives it: metres and radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spacing {
    pub(crate) radius: f64,
    pub(crate) eye: f64,
    pub(crate) spacing: f64,
    pub(crate) metres_per_pixel: f64,
    pub(crate) focus_scale: f64,
    pub(crate) focus_pull: f64,
    pub(crate) half_arc: f64,
    pub(crate) max_panels: f64,
    pub(crate) stack_step: f64,
}

impl Default for Spacing {
    fn default() -> Spacing {
        let ring = RingParams::default();
        Spacing {
            radius: ring.radius / 1000.0,
            eye: ring.eye / 1000.0,
            spacing: ring.gap,
            metres_per_pixel: ring.mm_per_px / 1000.0,
            focus_scale: ring.focus_scale,
            focus_pull: ring.focus_pull / 1000.0,
            half_arc: ring.half_arc,
            max_panels: ring.max_panels,
            stack_step: ring.stack_step / 1000.0,
        }
    }
}

impl Spacing {
    /// The ring in millimetres.
    pub(crate) fn ring(&self) -> RingParams {
        RingParams {
            radius: self.radius * 1000.0,
            eye: self.eye * 1000.0,
            mm_per_px: self.metres_per_pixel * 1000.0,
            focus_scale: self.focus_scale,
            focus_pull: self.focus_pull * 1000.0,
            gap: self.spacing,
            half_arc: self.half_arc,
            max_panels: self.max_panels,
            stack_step: self.stack_step * 1000.0,
        }
    }

    /// Sets the field `name` to `value`, which must be a number above 0.
    ///
    /// # Errors
    ///
    /// The value is not a finite number above 0.
    pub(crate) fn set(&mut self, name: &str, value: f64) -> Res<()> {
        if !(value.is_finite() && value > 0.0) {
            return Err(Fail::type_error(format!(
                "SpatialLayout: {name} must be a number above 0, got {value}"
            )));
        }
        let field = match name {
            "radius" => &mut self.radius,
            "eye" => &mut self.eye,
            "spacing" => &mut self.spacing,
            "metres_per_pixel" => &mut self.metres_per_pixel,
            "focus_scale" => &mut self.focus_scale,
            "focus_pull" => &mut self.focus_pull,
            "half_arc" => &mut self.half_arc,
            "max_panels" => &mut self.max_panels,
            "stack_step" => &mut self.stack_step,
            _ => return Err(Fail::no_such(format!("SpatialLayout has no `{name}`"))),
        };
        *field = value;
        Ok(())
    }

    pub(crate) fn get(&self, name: &str) -> f64 {
        match name {
            "radius" => self.radius,
            "eye" => self.eye,
            "spacing" => self.spacing,
            "metres_per_pixel" => self.metres_per_pixel,
            "focus_scale" => self.focus_scale,
            "focus_pull" => self.focus_pull,
            "half_arc" => self.half_arc,
            "max_panels" => self.max_panels,
            _ => self.stack_step,
        }
    }
}

/// The fields of [`Spacing`], by the name of their property.
pub(crate) const SPACING: [&str; 9] = [
    "radius",
    "eye",
    "spacing",
    "metres_per_pixel",
    "focus_scale",
    "focus_pull",
    "half_arc",
    "max_panels",
    "stack_step",
];

/// The triggers of the stock bindings of a layout; an empty one leaves its binding out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Triggers {
    pub(crate) move_: String,
    pub(crate) resize: String,
    pub(crate) rotate: String,
}

impl Default for Triggers {
    fn default() -> Triggers {
        Triggers {
            move_: "Grip+Drag".to_owned(),
            resize: "Grip+Trigger+Drag".to_owned(),
            rotate: "Trigger+Thumbstick+Drag".to_owned(),
        }
    }
}

impl Triggers {
    /// Each trigger that is not empty is a good one.
    ///
    /// # Errors
    ///
    /// A trigger that does not read, named as `SpatialLayout trigger "<id>"`.
    pub(crate) fn check(&self) -> Res<()> {
        for (id, text) in self.each() {
            if !text.is_empty() {
                trigger::parse(text, &format!("SpatialLayout trigger \"{id}\""))?;
            }
        }
        Ok(())
    }

    /// The id and trigger of each stock binding, in the order a layout makes them.
    pub(crate) fn each(&self) -> [(&'static str, &str); 3] {
        [
            ("move", self.move_.as_str()),
            ("resize", self.resize.as_str()),
            ("rotate", self.rotate.as_str()),
        ]
    }
}

/// What a move drag follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tracking {
    Controller,
    Ray,
}

/// The corners of a panel as the signs of its local x (right) and y (up), in the order of `resize_corner` after `quadrant`.
pub(crate) const CORNERS: [(f64, f64); 4] = [(-1.0, 1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];

/// What a binding does natively: the drag of its class.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Native {
    /// The base class: it declines every drag unless a handler takes it.
    Plain,
    Move {
        tracking: Tracking,
        /// Whether the panel keeps its own rotation.
        keep: bool,
    },
    Resize {
        pixels_per_metre: f64,
        min_width: f64,
        min_height: f64,
        /// 0 for `quadrant`, else a place in [`CORNERS`] counted from one.
        edges: u32,
    },
    Rotate,
    Scale {
        min: f64,
        max: f64,
    },
}

impl Native {
    /// The id and trigger a binding of this class has unless it is told otherwise.
    pub(crate) fn defaults(&self) -> (&'static str, &'static str) {
        match self {
            Native::Plain => ("", ""),
            Native::Move { .. } => ("move", "Grip+Drag"),
            Native::Resize { .. } => ("resize", "Grip+Trigger+Drag"),
            Native::Rotate => ("rotate", "Trigger+Thumbstick+Drag"),
            Native::Scale { .. } => ("scale", "Grip+Thumbstick+Drag"),
        }
    }

    /// The name of the class, for a refusal.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Native::Plain => "XRControllerDragBinding",
            Native::Move { .. } => "ViewMoveBinding",
            Native::Resize { .. } => "ViewResizeBinding",
            Native::Rotate => "ViewRotateBinding",
            Native::Scale { .. } => "ViewScaleBinding",
        }
    }
}

/// A script's hooks for a binding.
#[derive(Clone)]
pub(crate) struct Handler {
    pub(crate) obj: Obj,
    pub(crate) shell: Rc<ShellImpl>,
}

/// One binding.
pub(crate) struct BindingData {
    /// The object that stands for it, set once it is made.
    pub(crate) obj: Cell<Obj>,
    pub(crate) id: RefCell<String>,
    pub(crate) trigger: RefCell<String>,
    pub(crate) parsed: RefCell<ControllerTrigger>,
    pub(crate) enabled: Cell<bool>,
    pub(crate) priority: Cell<f64>,
    pub(crate) handler: RefCell<Option<Handler>>,
    pub(crate) native: RefCell<Native>,
}

impl BindingData {
    /// Reads `text` as the binding's trigger.
    ///
    /// # Errors
    ///
    /// The trigger does not read.
    pub(crate) fn set_trigger(&self, text: &str) -> Res<()> {
        let parsed = trigger::parse(text, "trigger")?;
        *self.trigger.borrow_mut() = text.to_owned();
        *self.parsed.borrow_mut() = parsed;
        Ok(())
    }
}

/// The bindings of a layout, in the order they were added. Each one's object is held by the collection.
#[derive(Default)]
pub(crate) struct Collection {
    pub(crate) items: RefCell<Vec<Rc<BindingData>>>,
}

impl Collection {
    pub(crate) fn get(&self, id: &str) -> Option<Rc<BindingData>> {
        self.items.borrow().iter().find(|item| *item.id.borrow() == id).cloned()
    }

    /// Whether `binding` may join: its id is free, and no binding has its chord and its priority.
    ///
    /// # Errors
    ///
    /// The id is taken or the chord is held, as `already_exists`.
    pub(crate) fn admits(&self, binding: &BindingData) -> Res<()> {
        let id = binding.id.borrow().clone();
        if id.is_empty() {
            return Err(Fail::type_error("a binding needs an id to join a collection"));
        }
        let items = self.items.borrow();
        if items.iter().any(|item| *item.id.borrow() == id) {
            return Err(Fail::refused(
                ErrorCode::AlreadyExists,
                format!("a binding with id \"{id}\" is already in this collection"),
            ));
        }
        let chord = binding.parsed.borrow().text.clone();
        if let Some(held) = items
            .iter()
            .find(|item| item.parsed.borrow().text == chord && item.priority.get() == binding.priority.get())
        {
            return Err(Fail::refused(
                ErrorCode::AlreadyExists,
                format!(
                    "the binding \"{id}\" takes the chord {chord} at priority {} of the binding \"{}\"",
                    binding.priority.get(),
                    held.id.borrow()
                ),
            ));
        }
        Ok(())
    }
}

/// One layout.
pub(crate) struct LayoutData {
    pub(crate) obj: Cell<Obj>,
    pub(crate) spacing: Cell<Spacing>,
    pub(crate) triggers: RefCell<Triggers>,
    /// The collection's object, held by the layout.
    pub(crate) bindings_obj: Obj,
    pub(crate) bindings: Rc<Collection>,
}
