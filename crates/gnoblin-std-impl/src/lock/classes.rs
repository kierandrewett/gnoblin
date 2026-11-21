//! The classes of `lock.toml` as the object protocol serves them.

use std::rc::Rc;

use gnoblin_object::server::Builder;
use gnoblin_object::{Closure, Fail, Res, Val};

use super::entry::{self, PasswordPart};
use super::screen;
use super::style::LockStyle;
use crate::ui::classes::class_with;
use crate::ui::clock;
use crate::ui::tree::ui_of;
use crate::ui::widget::{CanvasHandle, Ui, WidgetKind};

/// A closure a shell gave, or none: a value that is none takes the closure away.
fn closure_of(value: &Val) -> Res<Option<Closure>> {
    if value.is_none() {
        Ok(None)
    } else {
        Ok(Some(Rc::clone(value.closure()?)))
    }
}

/// The style a constructor was given, or the stock one.
fn style_init(inits: &mut gnoblin_object::server::Inits) -> Res<LockStyle> {
    match inits.take("style") {
        Some(value) if !value.is_none() => LockStyle::of(&value),
        _ => Ok(LockStyle::default()),
    }
}

fn password_class(builder: &mut Builder) {
    fn read<R>(h: &CanvasHandle, pick: impl FnOnce(&PasswordPart) -> R) -> Res<R> {
        entry::read(&h.ui, h.cid, pick).ok_or_else(|| Fail::type_error("the canvas is not a password entry"))
    }
    fn write(h: &CanvasHandle, change: impl FnOnce(&mut PasswordPart)) -> Res<()> {
        read(h, |_| ())?;
        entry::change(&h.ui, h.cid, change);
        h.ui.schedule();
        Ok(())
    }
    let class = class_with(builder, "password_entry", WidgetKind::Box, |ui, _cx, inits, cid| {
        let style = style_init(inits)?;
        entry::init(ui, cid, &style, style.card_width - 2.0 * style.card_padding, false);
        Ok(())
    });
    let _ = class
        .get("style", |h, _| read(h, |part| part.style.to_val()))
        .get("enabled", |h, _| Ok(Val::Bool(read(h, |part| part.enabled)?)))
        .set("enabled", |h, _, v| {
            let enabled = v.bool()?;
            write(h, |part| part.enabled = enabled)
        })
        .get("limit", |h, _| Ok(Val::Uint(read(h, |part| part.limit as u64)?)))
        .set("limit", |h, _, v| {
            let limit = usize::try_from(v.uint()?).unwrap_or(usize::MAX);
            write(h, |part| part.limit = limit)
        })
        .get("size", |h, _| Ok(Val::Double(read(h, |part| part.size)?)))
        .set("size", |h, _, v| {
            let size = v.double()?;
            write(h, |part| part.size = size)
        })
        .get("color", |h, _| Ok(Val::Text(read(h, |part| part.color.clone())?)))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            write(h, |part| part.color = color)
        })
        .get("length", |h, _| {
            read(h, |_| ())?;
            Ok(Val::Uint(entry::length(&h.ui, h.cid) as u64))
        })
        .get("on_submit", |h, _| {
            Ok(read(h, |part| part.on_submit.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("on_submit", |h, _, v| {
            let submit = closure_of(&v)?;
            write(h, |part| part.on_submit = submit)
        })
        .method("clear", |h, _, _| {
            read(h, |_| ())?;
            entry::clear(&h.ui, h.cid);
            Ok(Val::None)
        });
}

/// The state of the `lock_screen` object: the generation's toolkit. When the object ends the screen is down for good.
struct LockHandle {
    ui: Rc<Ui>,
}

impl Drop for LockHandle {
    fn drop(&mut self) {
        screen::end(&self.ui);
    }
}

fn screen_class(builder: &mut Builder) {
    let _ = builder
        .class::<LockHandle>("lock_screen")
        .ctor(|cx, inits| {
            let ui = ui_of(cx)?;
            let style = style_init(inits)?;
            screen::begin(&ui, style, cx.me())?;
            Ok(LockHandle { ui })
        })
        .get("style", |h, _| Ok(h.ui.lock.borrow().style.to_val()))
        .get("shown", |h, _| Ok(Val::Bool(screen::is_shown(&h.ui))))
        .get("now", |h, _| {
            Ok(h.ui.lock.borrow().now.clone().map_or(Val::None, Val::Closure))
        })
        .set("now", |h, _, v| {
            h.ui.lock.borrow_mut().now = closure_of(&v)?;
            h.ui.schedule();
            Ok(())
        })
        .method("hide", |h, _, _| {
            screen::release(&h.ui);
            Ok(Val::None)
        })
        .method("show", |h, _, _| {
            screen::resume(&h.ui);
            Ok(Val::None)
        })
        .function("format_clock", |_cx, args| {
            Ok(Val::Text(clock::format_time("%H:%M", args[0].double()?)))
        })
        .function("default_lock_style", |_cx, _args| Ok(LockStyle::default().to_val()));
}

/// Implements the classes of the lock module.
pub(crate) fn register(builder: &mut Builder) {
    password_class(builder);
    screen_class(builder);
}
