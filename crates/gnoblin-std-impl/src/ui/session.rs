//! What the toolkit shares with the session: the primary screen, the pointer, whether the session is locked, what the titlebar's clicks
//! do, the hooks a shell runs from the toolkit's frame, and the object that removes one.

use std::rc::Rc;

use gnoblin_object::server::Builder;
use gnoblin_object::{Closure, Fail, Res, Val};

use super::tree::ui_of;
use super::update::now_ms;
use super::widget::{Hook, Ui};

/// What each titlebar click does when the seat says nothing else.
const SEAT_ACTIONS: [(&str, &str); 3] = [
    ("double_click", "toggle-maximize"),
    ("middle_click", "lower"),
    ("right_click", "menu"),
];

impl Ui {
    /// The nodes of the class `class` under the root.
    fn root_children_of(&self, class: &str) -> Vec<gnoblin_core::Node> {
        let Ok(root) = self.g.root() else {
            return Vec::new();
        };
        root.children()
            .unwrap_or_default()
            .into_iter()
            .filter(|child| child.class_name().is_ok_and(|name| name == class))
            .collect()
    }

    /// The size of the primary screen (the first when none is primary), or `None` while the tree has no sized screen.
    pub(crate) fn primary_screen(&self) -> Option<(f64, f64)> {
        let screens = self.root_children_of("screen");
        let primary = screens
            .iter()
            .find(|screen| screen.get_bool("primary").unwrap_or(false))
            .or_else(|| screens.first())?;
        let (width, height) = (
            primary.get_f64("width").unwrap_or(0.0),
            primary.get_f64("height").unwrap_or(0.0),
        );
        (width > 0.0 && height > 0.0).then_some((width, height))
    }

    /// Whether the session is locked or about to be: nothing opens over the lock screen.
    pub(crate) fn locked(&self) -> bool {
        self.root_children_of("lock").iter().any(|lock| {
            lock.get_bool("engaged").unwrap_or(false) || lock.get_text("state").is_ok_and(|state| state != "not-locked")
        })
    }

    /// The tree's lock node, if it has one.
    pub(crate) fn lock_node(&self) -> Option<u64> {
        self.root_children_of("lock").first().map(|lock| lock.id().0)
    }

    /// The session lock's state as the compositor last published it: `not-locked` while the tree has no lock node.
    pub(crate) fn lock_state(&self) -> String {
        self.root_children_of("lock")
            .first()
            .and_then(|lock| lock.get_text("state").ok())
            .filter(|state| !state.is_empty())
            .unwrap_or_else(|| "not-locked".to_owned())
    }

    /// The pointer's place in logical screen pixels, or `None` when the tree has no seat.
    pub(crate) fn pointer_place(&self) -> Option<(f64, f64)> {
        crate::nodes::pointer(&self.g)
    }

    /// What the titlebar click `key` does, as the seat configures it. The default stands for what the seat does not say.
    pub(crate) fn seat_action(&self, key: &str) -> Option<String> {
        let (_, default) = SEAT_ACTIONS.iter().find(|(name, _)| *name == key)?;
        let seat = crate::nodes::seat(&self.g)?;
        Some(
            seat.get_text(key)
                .ok()
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| (*default).to_owned()),
        )
    }

    /// Adds a hook to `hooks` and answers its object.
    fn add_hook(self: &Rc<Self>, cx: &mut gnoblin_object::server::Cx<'_>, frames: bool, run: Closure) -> Res<Val> {
        let id = self.next_hook.get() + 1;
        self.next_hook.set(id);
        let hook = Hook { id, run };
        if frames {
            self.frame_hooks.borrow_mut().push(hook);
        } else {
            self.request_hooks.borrow_mut().push(hook);
        }
        let obj = cx.spawn(
            "ui_hook",
            HookHandle {
                ui: Rc::clone(self),
                id,
                frames,
            },
        )?;
        self.schedule();
        cx.handout(obj)
    }
}

/// The state of a `ui_hook`: which hook of which list it removes.
struct HookHandle {
    ui: Rc<Ui>,
    id: u64,
    frames: bool,
}

impl HookHandle {
    fn remove(&self) {
        let hooks = if self.frames {
            &self.ui.frame_hooks
        } else {
            &self.ui.request_hooks
        };
        hooks.borrow_mut().retain(|hook| hook.id != self.id);
    }
}

fn point(x: f64, y: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![Val::Double(x), Val::Double(y)],
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<HookHandle>("ui_hook")
        .ctor(|_cx, _inits| Err(Fail::refused("a hook is made by `ui.on_frame` and `ui.on_request`")))
        .method("remove", |hook, _cx, _args| {
            hook.remove();
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("ui")
        .function("now", |_cx, _args| Ok(Val::Double(now_ms())))
        .function("focused", |cx, _args| {
            let ui = ui_of(cx)?;
            Ok(Val::Uint(ui.seat_focus().unwrap_or(0)))
        })
        .function("focus", |cx, args| {
            let ui = ui_of(cx)?;
            ui.focus_node(args[0].uint()?)?;
            Ok(Val::None)
        })
        .function("locked", |cx, _args| Ok(Val::Bool(ui_of(cx)?.locked())))
        .function("pointer", |cx, _args| {
            Ok(ui_of(cx)?.pointer_place().map_or(Val::None, |(x, y)| point(x, y)))
        })
        .function("screen", |cx, _args| {
            Ok(ui_of(cx)?
                .primary_screen()
                .map_or(Val::None, |(width, height)| Val::Record {
                    class: 0,
                    fields: vec![Val::Double(width), Val::Double(height)],
                }))
        })
        .function("seat_action", |cx, args| {
            Ok(ui_of(cx)?
                .seat_action(args[0].text()?)
                .map_or_else(|| Val::Text(String::new()), Val::Text))
        })
        .function("canvas_count", |cx, _args| {
            Ok(Val::Uint(ui_of(cx)?.w.borrow().live() as u64))
        })
        .function("on_frame", |cx, args| {
            let ui = ui_of(cx)?;
            ui.add_hook(cx, true, Rc::clone(args[0].closure()?))
        })
        .function("on_request", |cx, args| {
            let ui = ui_of(cx)?;
            ui.add_hook(cx, false, Rc::clone(args[0].closure()?))
        });
}
