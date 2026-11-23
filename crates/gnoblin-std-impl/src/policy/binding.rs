//! What the pointer and gesture bindings of this module share: the object a shell makes (`policy_binding` and its subclasses), and the
//! way it becomes a binding of the core's input matcher.
//!
//! The matcher is the core's (`gnoblin.model`'s `input`): the chord, the threshold, the tiers, the priority, the decline, the claim and its
//! hand-over across a reload. This library is its guest. A binding of this module is installed by making an object that implements the
//! core's `pointer_drag_binding` or `gesture_binding` through the object protocol and binding it on an input context, so the drag or
//! the gesture reaches this library's rules the way it would reach a shell's own binding.

use std::mem::ManuallyDrop;
use std::rc::Rc;

use gnoblin_core::objects::{
    Binding, BindingEvent, BindingOptions, GestureBindingHandlers, InputContext, ModelRoot, Objects,
    PointerDragBindingHandlers,
};
use gnoblin_object::server::{ClassBuilder, Cx};
use gnoblin_object::{ErrorCode, Fail, Res, Val};

use super::drags::Stock;
use super::gesture_actions::{Pinch, Swipe};
use super::slide::Slide;
use crate::ui::tree::core_fail;
use crate::ui::{Ui, ui_of};

/// What a binding is while it is installed: the core's binding, which the core keeps until it is unbound.
///
/// The handle is never dropped: when the process ends the thread-locals of the two libraries go in an order nobody chooses, and dropping a
/// handle of the core's object table then calls into a table that is gone. The core frees the binding with its generation.
pub(super) struct PolicyBound {
    binding: ManuallyDrop<Binding>,
}

impl PolicyBound {
    /// Takes the binding off the context. A drag or a gesture in flight is cancelled by the core.
    fn remove(self) {
        let _ = self.binding.unbind();
    }
}

/// How a binding differs from another: the rule it runs.
pub(super) enum Flavour {
    Stock(Stock),
    Swipe(Swipe),
    Pinch(Pinch),
    Slide(Slide),
}

/// A binding of this module, before and after it is installed.
pub(crate) struct PolicyBinding {
    pub(super) trigger: String,
    pub(super) priority: f64,
    pub(super) flavour: Flavour,
    installed: Option<PolicyBound>,
}

impl PolicyBinding {
    pub(super) fn new(trigger: String, priority: f64, flavour: Flavour) -> PolicyBinding {
        PolicyBinding {
            trigger,
            priority,
            flavour,
            installed: None,
        }
    }

    /// A setting of an installed binding is not changed: the matcher has its trigger and priority, and the rule its options.
    pub(super) fn open(&self) -> Res<()> {
        if self.installed.is_some() {
            return Err(Fail::refused(
                ErrorCode::InvalidState,
                "the binding is installed: uninstall it before changing it",
            ));
        }
        Ok(())
    }

    fn install(&mut self, ui: &Rc<Ui>, context: Option<&str>) -> Res<()> {
        if self.installed.is_some() {
            return Err(Fail::refused(
                ErrorCode::InvalidState,
                "the binding is installed already",
            ));
        }
        let objects = ui.g.objects().map_err(core_fail)?;
        let context = context_in(&objects, context)?;
        let options = BindingOptions {
            priority: Some(self.priority),
            ..BindingOptions::default()
        };
        let binding = match &self.flavour {
            Flavour::Stock(stock) => {
                let handlers = stock.handlers(ui);
                context.bind_drag(&self.trigger, &handlers.implement(&objects)?, Some(&options))?
            }
            Flavour::Swipe(swipe) => {
                let handlers = swipe.handlers(ui);
                context.bind_gesture(&self.trigger, &handlers.implement(&objects)?, Some(&options))?
            }
            Flavour::Pinch(pinch) => {
                let handlers = pinch.handlers(ui);
                context.bind_gesture(&self.trigger, &handlers.implement(&objects)?, Some(&options))?
            }
            Flavour::Slide(slide) => {
                let handlers = slide.handlers(ui);
                context.bind_gesture(&self.trigger, &handlers.implement(&objects)?, Some(&options))?
            }
        };
        self.installed = Some(PolicyBound {
            binding: ManuallyDrop::new(binding),
        });
        Ok(())
    }

    fn uninstall(&mut self) {
        if let Some(installed) = self.installed.take() {
            installed.remove();
        }
    }
}

/// The input context named `name`, or the first seat's.
fn context_in(objects: &Objects, name: Option<&str>) -> Res<InputContext> {
    let model = ModelRoot::get(objects)?;
    let input = model
        .input()?
        .ok_or_else(|| Fail::refused(ErrorCode::Unavailable, "the compositor reports no input"))?;
    match name {
        Some(name) => input
            .find_context(name)?
            .ok_or_else(|| Fail::no_such(format!("there is no input context named `{name}`"))),
        None => input
            .primary()?
            .ok_or_else(|| Fail::refused(ErrorCode::Unavailable, "the compositor reports no seat")),
    }
}

/// The members every binding class has: its trigger, its priority, and the way on and off a context.
pub(super) fn members(class: ClassBuilder<'_, PolicyBinding>) -> ClassBuilder<'_, PolicyBinding> {
    shared(
        class
            .get("trigger", |h, _| Ok(Val::Text(h.trigger.clone())))
            .set("trigger", |h, _, value| {
                h.open()?;
                h.trigger = value.text()?.to_owned();
                Ok(())
            }),
    )
}

/// The class the bindings extend: nothing makes one of its own.
pub(super) fn register_base(builder: &mut gnoblin_object::server::Builder) {
    let class = builder.class::<PolicyBinding>("policy_binding").ctor(|_cx, _inits| {
        Err(Fail::refused(
            ErrorCode::Unsupported,
            "make a move, resize, swap, split resize, swipe, pinch or slide binding",
        ))
    });
    let _ = shared(class);
}

/// What the base class holds: the priority, whether the binding is installed, and `install` and `uninstall`.
fn shared(class: ClassBuilder<'_, PolicyBinding>) -> ClassBuilder<'_, PolicyBinding> {
    class
        .get("priority", |h, _| Ok(Val::Double(h.priority)))
        .set("priority", |h, _, value| {
            h.open()?;
            h.priority = value.double()?;
            Ok(())
        })
        .get("installed", |h, _| Ok(Val::Bool(h.installed.is_some())))
        .method("install", |h, cx: &mut Cx<'_>, args| {
            let ui = ui_of(cx)?;
            let name = args
                .first()
                .filter(|value| !value.is_none())
                .map(Val::text)
                .transpose()?;
            h.install(&ui, name)?;
            // The binding lives as long as it is installed, whatever the shell does with the object.
            if let Err(fail) = cx.hold(cx.me()) {
                h.uninstall();
                return Err(fail);
            }
            Ok(Val::None)
        })
        .method("uninstall", |h, cx, _| {
            if h.installed.is_some() {
                h.uninstall();
                cx.unhold(cx.me());
            }
            Ok(Val::None)
        })
}

/// The handlers of a drag whose steps `run` answers: `start` takes the drag or declines it, `update`, `end` and `cancel` follow it.
pub(super) fn drag_handlers(
    start: impl FnMut(&BindingEvent) -> Res<bool> + 'static,
    step: impl FnMut(BindStep, &BindingEvent) + 'static,
) -> PointerDragBindingHandlers {
    let mut start = start;
    let step = Rc::new(std::cell::RefCell::new(step));
    let (update, end, cancel) = (Rc::clone(&step), Rc::clone(&step), step);
    PointerDragBindingHandlers {
        on_start: Box::new(move |event| start(&event)),
        on_update: Some(Box::new(move |event| {
            update.borrow_mut()(BindStep::Update, &event);
            Ok(())
        })),
        on_end: Some(Box::new(move |event| {
            end.borrow_mut()(BindStep::End, &event);
            Ok(())
        })),
        on_cancel: Some(Box::new(move |event| {
            cancel.borrow_mut()(BindStep::Cancel, &event);
            Ok(())
        })),
    }
}

/// The handlers of a gesture, as `drag_handlers` makes them for a drag.
pub(super) fn gesture_handlers(
    begin: impl FnMut(&BindingEvent) -> Res<bool> + 'static,
    step: impl FnMut(BindStep, &BindingEvent) + 'static,
) -> GestureBindingHandlers {
    let mut begin = begin;
    let step = Rc::new(std::cell::RefCell::new(step));
    let (update, end, cancel) = (Rc::clone(&step), Rc::clone(&step), step);
    GestureBindingHandlers {
        on_begin: Box::new(move |event| begin(&event)),
        on_update: Some(Box::new(move |event| {
            update.borrow_mut()(BindStep::Update, &event);
            Ok(())
        })),
        on_end: Some(Box::new(move |event| {
            end.borrow_mut()(BindStep::End, &event);
            Ok(())
        })),
        on_cancel: Some(Box::new(move |event| {
            cancel.borrow_mut()(BindStep::Cancel, &event);
            Ok(())
        })),
    }
}

/// A step of a drag or a gesture after its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindStep {
    Update,
    End,
    Cancel,
}
