//! What the toolkit hears from the compositor: pointer events and keys sent to its canvases, requests a client made of a window, the
//! seat's keyboard moving, and a node going. A pointer event goes to the canvas the compositor hit, then up through its parents until
//! a handler answers `true`; a key goes to the canvas that holds the keyboard focus and up the same way.

use std::rc::Rc;

use gnoblin_core::{ShellValue, SignalEvent};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::{Fail, Obj, Res, Val};

use super::entry;
use super::tree::{core_fail, find, forget};
use super::widget::{Cid, Ui, en};

/// An event of the `tree` class's `event` signal, as the compositor sent it.
pub(crate) struct CanvasEvent {
    pub(crate) node: u64,
    pub(crate) kind: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) button: u64,
    pub(crate) key: String,
    pub(crate) text: String,
    pub(crate) dx: f64,
    pub(crate) dy: f64,
    pub(crate) request: String,
    pub(crate) pressed: bool,
}

impl CanvasEvent {
    /// Reads the fields of the signal in the schema's order: frame, seq, node, kind, x, y, button, key, text, dx, dy, request, pressed.
    fn read(event: &SignalEvent<'_>) -> Option<CanvasEvent> {
        if event.len() < 13 {
            return None;
        }
        let text = |index: usize| match event.value(index) {
            Some(ShellValue::Text(text)) => text.to_owned(),
            _ => String::new(),
        };
        let number = |index: usize| match event.value(index) {
            Some(ShellValue::Double(value)) => value,
            _ => 0.0,
        };
        Some(CanvasEvent {
            node: event.node().0,
            kind: text(3),
            x: number(4),
            y: number(5),
            button: match event.value(6) {
                Some(ShellValue::Uint(value)) => value,
                _ => 0,
            },
            key: text(7),
            text: text(8),
            dx: number(9),
            dy: number(10),
            request: text(11),
            pressed: matches!(event.value(12), Some(ShellValue::Bool(true))),
        })
    }
}

/// The generation ended: the root canvases go, and the toolkit with them.
fn shutdown(ctx: GnoblinCtx) {
    let Some(ui) = find(ctx) else {
        return;
    };
    let roots = ui.w.borrow().roots();
    for root in roots {
        ui.destroy(root);
    }
    forget(ctx);
}

/// Subscribes the toolkit to what it needs of the compositor.
///
/// # Errors
///
/// The core refuses a subscription.
pub(crate) fn subscribe(ui: &Rc<Ui>) -> Res<()> {
    let ctx = ui.ctx;
    let g = &ui.g;
    g.on_shutdown(move || shutdown(ctx)).map_err(core_fail)?;
    g.on_signal_named("tree", "event", move |event| {
        if let (Some(ui), Some(raw)) = (find(ctx), CanvasEvent::read(event)) {
            ui.on_event(&raw);
        }
    })
    .map_err(core_fail)?;
    g.on_node_destroyed("canvas", move |node| {
        if let Some(ui) = find(ctx) {
            let cid = ui.w.borrow().by_node.get(&node.id().0).copied();
            if let Some(cid) = cid {
                ui.node_gone(cid);
            }
        }
    })
    .map_err(core_fail)?;
    g.on_class_property_changed("seat", "focus", move |_node, _value| {
        if let Some(ui) = find(ctx) {
            ui.focus_moved();
        }
    })
    .map_err(core_fail)?;
    Ok(())
}

impl Ui {
    /// An event the compositor sent: a pointer event or a key addressed to a canvas of ours, or a request made of any window.
    fn on_event(self: &Rc<Self>, raw: &CanvasEvent) {
        if raw.kind == "request" {
            self.request(raw);
            return;
        }
        let Some(cid) = self.w.borrow().by_node.get(&raw.node).copied() else {
            return;
        };
        if raw.kind == "key" {
            self.dispatch_key(cid, raw);
        } else if let Some(kind) = en::pointer_kind(&raw.kind) {
            self.dispatch_pointer(cid, kind, raw);
        }
    }

    /// A request made of a window goes to the shell's hooks, not to a canvas: a window's menu is not a canvas event.
    fn request(self: &Rc<Self>, raw: &CanvasEvent) {
        let record = Val::Record {
            class: 0,
            fields: vec![
                Val::Uint(raw.node),
                Val::Text(raw.request.clone()),
                Val::Double(raw.x),
                Val::Double(raw.y),
            ],
        };
        let hooks: Vec<_> = self
            .request_hooks
            .borrow()
            .iter()
            .map(|hook| Rc::clone(&hook.run))
            .collect();
        for hook in hooks {
            if let Err(fail) = hook.call(0, std::slice::from_ref(&record)) {
                self.warn(&format!("onUiRequest threw: {fail}"));
            }
        }
    }

    /// A `ui_pointer_event` record, in the coordinates of the canvas whose handler runs.
    fn pointer_record(&self, kind: u32, x: f64, y: f64, raw: &CanvasEvent, target: Obj) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::Enum { class: 0, value: kind },
                Val::Double(x),
                Val::Double(y),
                Val::Uint(raw.button),
                Val::Double(raw.dx),
                Val::Double(raw.dy),
                self.handout(target),
            ],
        }
    }

    /// Sends a pointer event to `target` and then up its parents until a handler stops it. Coordinates are relative to the canvas
    /// whose handler is running.
    fn dispatch_pointer(self: &Rc<Self>, target: Cid, kind: u32, raw: &CanvasEvent) {
        let Some(target_obj) = self.read(target, |widget| widget.obj) else {
            return;
        };
        let (mut x, mut y) = (raw.x, raw.y);
        let mut at = Some(target);
        while let Some(cid) = at {
            let Some((obj, wx, wy, parent, closure, title)) = self.read(cid, |widget| {
                (
                    widget.obj,
                    widget.x,
                    widget.y,
                    widget.parent,
                    widget.on_pointer.clone(),
                    widget.kind.title(),
                )
            }) else {
                break;
            };
            let mut stopped = false;
            let record = self.pointer_record(kind, x, y, raw, target_obj);
            if let Some(done) = self.hook_call(cid, |ids| ids.on_pointer, std::slice::from_ref(&record)) {
                match done {
                    Ok(value) => stopped = value.bool().unwrap_or(false),
                    Err(message) => self.warn(&format!("{title}.on_pointer threw: {message}")),
                }
            }
            if !stopped && let Some(closure) = closure {
                match closure.call(0, std::slice::from_ref(&record)) {
                    Ok(value) => stopped = value.bool().unwrap_or(false),
                    Err(fail) => self.warn(&format!("{title}.on_pointer threw: {fail}")),
                }
            }
            self.announce(obj, |cx| {
                cx.emit(
                    "pointer",
                    vec![Val::Record {
                        class: 0,
                        fields: vec![
                            Val::Enum { class: 0, value: kind },
                            Val::Double(x),
                            Val::Double(y),
                            Val::Uint(raw.button),
                            Val::Double(raw.dx),
                            Val::Double(raw.dy),
                            cx.object_value(target_obj),
                        ],
                    }],
                );
            });
            if stopped {
                break;
            }
            x += wx;
            y += wy;
            at = parent;
        }
    }

    /// A `ui_key_event` record. Its text may be a passphrase: nothing logs it.
    fn key_record(&self, raw: &CanvasEvent, target: Obj) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::Text(raw.key.clone()),
                Val::Text(raw.text.clone()),
                Val::Bool(raw.pressed),
                self.handout(target),
            ],
        }
    }

    /// Sends a key to `target` (the canvas that holds the keyboard focus) and then up its parents until a handler stops it.
    fn dispatch_key(self: &Rc<Self>, target: Cid, raw: &CanvasEvent) {
        let Some(target_obj) = self.read(target, |widget| widget.obj) else {
            return;
        };
        let mut at = Some(target);
        while let Some(cid) = at {
            let Some((obj, parent, closure, title)) = self.read(cid, |widget| {
                (widget.obj, widget.parent, widget.on_key.clone(), widget.kind.title())
            }) else {
                break;
            };
            // What the canvas does with a key by itself comes first: an entry types it. A shell's hooks hear it after, and what the
            // entry used it stops.
            let mut stopped = entry::on_key(self, cid, raw);
            let record = self.key_record(raw, target_obj);
            if !stopped && let Some(done) = self.hook_call(cid, |ids| ids.on_key, std::slice::from_ref(&record)) {
                match done {
                    Ok(value) => stopped = value.bool().unwrap_or(false),
                    Err(message) => self.warn(&format!("{title}.on_key threw: {message}")),
                }
            }
            if !stopped && let Some(closure) = closure {
                match closure.call(0, std::slice::from_ref(&record)) {
                    Ok(value) => stopped = value.bool().unwrap_or(false),
                    Err(fail) => self.warn(&format!("{title}.on_key threw: {fail}")),
                }
            }
            self.announce(obj, |cx| {
                cx.emit(
                    "key",
                    vec![Val::Record {
                        class: 0,
                        fields: vec![
                            Val::Text(raw.key.clone()),
                            Val::Text(raw.text.clone()),
                            Val::Bool(raw.pressed),
                            cx.object_value(target_obj),
                        ],
                    }],
                );
            });
            if stopped {
                break;
            }
            at = parent;
        }
    }

    /// The seat's keyboard moved: the canvas that lost it and the one that gained it are told.
    fn focus_moved(self: &Rc<Self>) {
        let now = self
            .seat_focus()
            .and_then(|node| self.w.borrow().by_node.get(&node).copied());
        let before = {
            let mut w = self.w.borrow_mut();
            std::mem::replace(&mut w.focus_seen, now)
        };
        if before == now {
            return;
        }
        for (cid, focused) in [(before, false), (now, true)] {
            if let Some(obj) = cid.and_then(|cid| self.read(cid, |widget| widget.obj)) {
                self.announce(obj, |cx| {
                    cx.changed("focused");
                    cx.emit("focus_changed", vec![Val::Bool(focused)]);
                });
            }
        }
    }

    /// The compositor ended the node of `cid`, or it was written off: the canvas and everything under it are gone, as far as the
    /// toolkit is concerned, and their objects say so.
    pub(crate) fn node_gone(self: &Rc<Self>, cid: Cid) {
        for ended in self.bury(cid) {
            if let Some(obj) = self.read(ended, |widget| widget.obj) {
                self.announce(obj, |cx| {
                    cx.changed("alive");
                    cx.emit("gone", Vec::new());
                    // Nothing keeps the canvas on screen any more: its object ends like any other, when the shell lets go of it.
                    cx.unpin(obj);
                });
            }
        }
    }

    /// Gives the seat's keyboard focus to a canvas by node, or takes it away with zero.
    ///
    /// # Errors
    ///
    /// The node is neither a canvas of ours nor a window, or the compositor refuses.
    pub(crate) fn focus_node(&self, node: u64) -> Res<()> {
        if node != 0 {
            let ours = self.w.borrow().by_node.contains_key(&node);
            if !ours {
                let target = self.node(node);
                if !target.is_alive() {
                    return Err(Fail::no_such("focus: the host refused (unknown-node)"));
                }
                let is_window = target.class_name().is_ok_and(|class| class == "window");
                if !is_window {
                    return Err(Fail::refused("focus: the host refused (not-owned)"));
                }
            }
        }
        self.set_focus(node)
    }
}
