//! Typed events: windows appearing and going, their properties changing, requests from their clients, input sent to a node.
//!
//! Each `on_*` method registers one named subscription with the core and returns a [`Subscription`]. The callback runs on the shell
//! thread, after the compositor's whole batch of changes has been applied, so it reads the tree as the batch left it: the window
//! it is told of is complete (its title, its size) and a write it makes goes out with the rest of the turn's.
//!
//! Windows that were already in the tree when the shell attached are not "created": list them with
//! [`Gnoblin::windows`](crate::Gnoblin::windows) first, then listen.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

use gnoblin_core_abi::{self as abi, GnoblinNodeEvent};

use crate::error::Result;
use crate::gnoblin::{CALLS, Gnoblin, Subscription, drop_slot};
use crate::owned::Value;
use crate::tree::{Node, Window};
use crate::value::{Class, MemberKind, NodeId, ShellValue, SignalEvent};

/// A closure behind a `user` pointer, with the generation's handle to build nodes from.
pub(crate) struct Slot<F> {
    pub(crate) f: F,
    pub(crate) g: Gnoblin,
}

/// What the core says about a change of the tree, borrowed for the callback.
pub(crate) struct NodeNotice<'a> {
    pub node: NodeId,
    pub value: Option<ShellValue<'a>>,
}

unsafe extern "C" fn node_tramp<F: FnMut(&Gnoblin, &NodeNotice<'_>)>(
    user: *mut c_void,
    event: *const GnoblinNodeEvent,
) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<Slot<F>>` leaked at registration. The core passes it back unchanged, on the shell thread, and does
    // not free it before `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<Slot<F>>() };
    // SAFETY: the core passes a pointer valid until this callback returns, or null.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return abi::E_ERR;
    };
    let value = if event.value.is_null() {
        None
    } else {
        // SAFETY: a change's value is valid until the callback returns, which is what `ShellValue::from_raw` needs.
        Some(unsafe { ShellValue::from_raw(&*event.value) })
    };
    let notice = NodeNotice {
        node: NodeId(event.node),
        value,
    };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(&slot.g, &notice))) {
        Ok(()) => abi::E_OK,
        Err(_) => abi::E_PANIC,
    }
}

/// A request a window's client made that policy decides: it asked to be maximised, to close, to open its menu.
///
/// A request changes nothing until the shell answers it: by writing the window's `maximised`, `fullscreen`, `minimised`, `width`
/// and `height`, or by calling `close`.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowRequest {
    /// What was asked for: `move`, `resize`, `maximize`, `unmaximize`, `fullscreen`, `unfullscreen`, `minimize`, `unminimize`,
    /// `close`, `raise`, `lower`, `menu` or `activate`.
    pub name: String,
    /// For `menu`: where it opens, in global logical coordinates. Zero for the others.
    pub x: f64,
    /// For `menu`: where it opens.
    pub y: f64,
}

/// An input event sent to a node: a key, a pointer press, motion, a wheel turn.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeEvent {
    /// The node it was sent to.
    pub node: NodeId,
    /// `press`, `release`, `motion`, `scroll`, `enter`, `leave`, `key` or `request`.
    pub kind: String,
    /// The pointer's place in the node's own coordinates.
    pub x: f64,
    /// The pointer's place in the node's own coordinates.
    pub y: f64,
    /// The button of a press or release, an evdev code.
    pub button: u64,
    /// The key's name (`Return`, `a`) for a `key` event.
    pub key: String,
    /// What the key types, empty for a key that types nothing.
    pub text: String,
    /// A scroll's horizontal delta.
    pub dx: f64,
    /// A scroll's vertical delta.
    pub dy: f64,
    /// Whether a key went down (false for its release).
    pub pressed: bool,
}

impl NodeEvent {
    /// Reads the fields of the `event` signal of the `tree` class, in the schema's order: frame, seq, node, kind, x, y, button, key,
    /// text, dx, dy, request, pressed.
    fn from_signal(event: &SignalEvent<'_>) -> Option<(NodeEvent, String)> {
        let text = |index: usize| match event.value(index) {
            Some(ShellValue::Text(text)) => text.to_owned(),
            _ => String::new(),
        };
        let number = |index: usize| match event.value(index) {
            Some(ShellValue::Double(value)) => value,
            _ => 0.0,
        };
        let uint = |index: usize| match event.value(index) {
            Some(ShellValue::Uint(value)) => value,
            _ => 0,
        };
        if event.len() < 13 {
            return None;
        }
        let request = text(11);
        Some((
            NodeEvent {
                node: event.node(),
                kind: text(3),
                x: number(4),
                y: number(5),
                button: uint(6),
                key: text(7),
                text: text(8),
                dx: number(9),
                dy: number(10),
                pressed: matches!(event.value(12), Some(ShellValue::Bool(true))),
            },
            request,
        ))
    }
}

impl Gnoblin {
    /// Registers `f` for the nodes of `class` (every class when `None`) that come into the tree.
    fn on_node_created_raw<F>(&self, class: Option<Class>, f: F) -> Result<Subscription>
    where
        F: FnMut(&Gnoblin, &NodeNotice<'_>) + 'static,
    {
        self.attach()?;
        let slot = Box::into_raw(Box::new(Slot { f, g: self.clone() }));
        // SAFETY: `slot` is a leaked box the core hands back to `node_tramp::<F>` and `drop_slot::<Slot<F>>`; on failure the
        // core has not adopted it and `adopt` reclaims it.
        let reg = unsafe {
            (self.api().on_node_created)(
                self.ctx(),
                class.map_or(0, |class| class.0),
                node_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<Slot<F>>),
            )
        };
        self.adopt(reg, slot)
    }

    fn on_node_destroyed_raw<F>(&self, class: Option<Class>, f: F) -> Result<Subscription>
    where
        F: FnMut(&Gnoblin, &NodeNotice<'_>) + 'static,
    {
        self.attach()?;
        let slot = Box::into_raw(Box::new(Slot { f, g: self.clone() }));
        // SAFETY: as for `on_node_created_raw`.
        let reg = unsafe {
            (self.api().on_node_destroyed)(
                self.ctx(),
                class.map_or(0, |class| class.0),
                node_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<Slot<F>>),
            )
        };
        self.adopt(reg, slot)
    }

    fn on_change_raw<F>(&self, node: Option<NodeId>, class: Class, property: u32, f: F) -> Result<Subscription>
    where
        F: FnMut(&Gnoblin, &NodeNotice<'_>) + 'static,
    {
        self.attach()?;
        let slot = Box::into_raw(Box::new(Slot { f, g: self.clone() }));
        // SAFETY: as for `on_node_created_raw`.
        let reg = unsafe {
            (self.api().on_change)(
                self.ctx(),
                node.unwrap_or(NodeId::NIL).0,
                class.0,
                property,
                node_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<Slot<F>>),
            )
        };
        self.adopt(reg, slot)
    }

    /// Calls `f` with each window that appears: a client opened one. The callback reads the window as it will stay (its title, its
    /// size), and what it writes leaves with the turn's other writes.
    ///
    /// ```ignore
    /// g.on_client_created(|window| {
    ///     window.set_geometry(Rect::new(0.0, 0.0, 800.0, 600.0)).ok();
    /// })?;
    /// ```
    pub fn on_client_created<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window) + 'static,
    {
        let window = self.class("window")?;
        self.on_node_created_raw(Some(window), move |g, notice| f(&Window::new(g.clone(), notice.node)))
    }

    /// Calls `f` with each window that goes: its client closed it. The window is already gone from the tree, so reading it fails
    /// with `NoSuchNode`; its id is what is left.
    pub fn on_client_destroyed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window) + 'static,
    {
        let window = self.class("window")?;
        self.on_node_destroyed_raw(Some(window), move |g, notice| f(&Window::new(g.clone(), notice.node)))
    }

    /// Calls `f` with a window and its new title when a client changes it.
    pub fn on_title_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window, &str) + 'static,
    {
        self.on_window_text("title", move |window, text| f(window, text))
    }

    /// Calls `f` with a window and its new application id when a client changes it.
    pub fn on_app_id_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window, &str) + 'static,
    {
        self.on_window_text("app_id", move |window, text| f(window, text))
    }

    fn on_window_text<F>(&self, property: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window, &str) + 'static,
    {
        let window = self.class("window")?;
        let property = self.member(window, MemberKind::Property, property)?;
        self.on_change_raw(None, window, property.id, move |g, notice| {
            if let Some(ShellValue::Text(text)) = &notice.value {
                f(&Window::new(g.clone(), notice.node), text);
            }
        })
    }

    /// Calls `f` with a window and the new value when its property `property` changes, whoever changed it (the compositor, or the
    /// shell itself). `property` is a schema name such as `focused`, `maximised`, `width`.
    pub fn on_window_property_changed<F>(&self, property: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window, &Value) + 'static,
    {
        let window = self.class("window")?;
        let property = self.member(window, MemberKind::Property, property)?;
        self.on_change_raw(None, window, property.id, move |g, notice| {
            if let Some(value) = notice.value.as_ref().and_then(Value::from_shell) {
                f(&Window::new(g.clone(), notice.node), &value);
            }
        })
    }

    /// Calls `f` when a property of `node` changes, with the property's new value. `property` is a schema name.
    pub fn on_property_changed<F>(&self, node: &Node, property: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Node, &Value) + 'static,
    {
        let class = node.class()?;
        let property = self.member(class, MemberKind::Property, property)?;
        self.on_change_raw(Some(node.id()), class, property.id, move |g, notice| {
            if let Some(value) = notice.value.as_ref().and_then(Value::from_shell) {
                f(&Node::new(g.clone(), notice.node), &value);
            }
        })
    }

    /// Calls `f` with the window that has the keyboard (`None` when none does) each time it changes.
    pub fn on_focus_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(Option<&Window>) + 'static,
    {
        let seat_class = self.class("seat")?;
        let property = self.member(seat_class, MemberKind::Property, "focus")?;
        self.on_change_raw(None, seat_class, property.id, move |g, notice| {
            if let Some(ShellValue::Node(node)) = &notice.value {
                if *node == NodeId::NIL {
                    f(None);
                } else {
                    f(Some(&Window::new(g.clone(), *node)));
                }
            }
        })
    }

    /// Calls `f` when a window's client makes a request that policy decides (maximise, close, open its menu, ...). A request
    /// changes nothing until the shell answers it.
    pub fn on_window_request<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Window, &WindowRequest) + 'static,
    {
        let g = self.clone();
        let window = self.class("window")?;
        self.on_tree_event(None, move |event, request| {
            if event.kind != "request" {
                return;
            }
            // Only a window's requests: a request names a window of the tree.
            let node = Node::new(g.clone(), event.node);
            if node.class().ok() != Some(window) {
                return;
            }
            f(
                &Window::new(g.clone(), event.node),
                &WindowRequest {
                    name: request.to_owned(),
                    x: event.x,
                    y: event.y,
                },
            );
        })
    }

    /// Calls `f` for each input event sent to `node`: the keys, buttons, motion and wheel a node that holds an input grab hears, or
    /// the pointer over a node of the shell's own.
    pub fn on_node_event<F>(&self, node: &Node, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&NodeEvent) + 'static,
    {
        self.on_tree_event(Some(node.id()), move |event, _request| f(event))
    }

    fn on_tree_event<F>(&self, node: Option<NodeId>, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&NodeEvent, &str) + 'static,
    {
        self.attach()?;
        let tree = self.class("tree")?;
        let signal = self.member(tree, MemberKind::Signal, "event")?;
        self.on_signal(node, tree, signal.id, move |event| {
            if let Some((event, request)) = NodeEvent::from_signal(event) {
                f(&event, &request);
            }
        })
    }
}
