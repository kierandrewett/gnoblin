//! The compositor's tree: nodes, windows, and the writes that place them.
//!
//! The compositor keeps a tree of nodes. A window is a node of class `window`; a shell that places windows writes their `x`,
//! `y`, `width` and `height`. [`Node`] is a handle on one node of any class, [`Window`] a handle on a window with the
//! accessors a window manager needs. Both carry the [`Gnoblin`] of the generation that made them, so after a reload a handle
//! the old generation kept fails with `Retired` instead of acting, and both name a node by its id, which the compositor never
//! reuses, so a handle to a window that closed fails with `NoSuchNode` instead of touching another window.
//!
//! Reads see the shell's own writes at once. Writes are queued and leave as one transaction when the callback that made them
//! returns (the end of the dispatch turn), so placing ten windows in a callback is one round trip, and a window never
//! flickers through an in-between state.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

use gnoblin_core_abi::{self as abi, GnoblinAnswer, GnoblinNodes, GnoblinPending, GnoblinStr, GnoblinValue};

use crate::error::{CoreError, ErrorKind, Result, check};
use crate::gnoblin::{CALLS, CallbackSlot, Gnoblin, copy_str, drop_slot};
use crate::owned::Value;
use crate::value::{Class, MemberKind, NodeId};

/// A node of the compositor's tree, by id. Cheap to clone and compare.
#[derive(Clone)]
pub struct Node {
    g: Gnoblin,
    id: NodeId,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Node").field(&self.id.0).finish()
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Node) -> bool {
        self.id == other.id
    }
}

impl Eq for Node {}

impl std::hash::Hash for Node {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// A rectangle in the compositor's logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Rect {
    /// A rectangle.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }
}

/// The compositor's answer to a call that was made with a callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The answer's bytes: text for most calls.
    pub result: Vec<u8>,
    /// The failure, `None` on success.
    pub error: Option<String>,
}

impl Answer {
    /// The result as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.result).into_owned()
    }

    /// The result, or the failure as an error.
    pub fn into_result(self) -> Result<Vec<u8>> {
        match self.error {
            None => Ok(self.result),
            Some(error) => Err(CoreError::new(ErrorKind::Refused, error)),
        }
    }
}

/// A call that has been made and whose answer has not arrived.
#[derive(Clone)]
pub struct PendingCall {
    g: Gnoblin,
    pending: GnoblinPending,
}

impl PendingCall {
    /// Stops waiting: the answer's callback never runs and is dropped now.
    pub fn cancel(self) -> Result<()> {
        // SAFETY: `cancel` takes only the pending handle.
        check(unsafe { (self.g.api().cancel)(self.pending) })
    }
}

/// What [`Gnoblin::adopt_node`] found or made.
#[derive(Clone, Debug)]
pub struct Adoption {
    /// The node.
    pub node: Node,
    /// Whether an earlier generation made it: the node is as that generation left it, so the caller keeps what it holds
    /// rather than building it again. `false` for a node made now.
    pub adopted: bool,
}

impl Node {
    pub(crate) fn new(g: Gnoblin, id: NodeId) -> Node {
        Node { g, id }
    }

    /// The node's id.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The generation's handle this node was made from.
    pub fn gnoblin(&self) -> &Gnoblin {
        &self.g
    }

    /// Whether the node is in the tree now. A node that closed, or one the shell destroyed in this tick, is not.
    pub fn is_alive(&self) -> bool {
        // SAFETY: `node_exists` takes the context handle and a node id.
        unsafe { (self.g.api().node_exists)(self.g.ctx(), self.id.0) == 1 }
    }

    /// The node's class.
    pub fn class(&self) -> Result<Class> {
        // SAFETY: `node_class` takes the context handle and a node id.
        let id = unsafe { (self.g.api().node_class)(self.g.ctx(), self.id.0) };
        if id == 0 {
            Err(CoreError::from_code(abi::E_NO_SUCH_NODE))
        } else {
            Ok(Class(id))
        }
    }

    /// The name of the node's class (`window`).
    pub fn class_name(&self) -> Result<String> {
        let class = self.class()?;
        let mut name = GnoblinStr::EMPTY;
        // SAFETY: `class_name` fills `name`, a live local; the text it points at is copied before the next ABI call.
        check(unsafe { (self.g.api().class_name)(self.g.ctx(), class.0, &mut name) })?;
        Ok(copy_str(&name))
    }

    /// The node's parent, `None` for the root and for a node that is not inserted anywhere yet.
    pub fn parent(&self) -> Result<Option<Node>> {
        let mut parent = 0;
        // SAFETY: `node_parent` writes one node id through the pointer it is given, a live local.
        check(unsafe { (self.g.api().node_parent)(self.g.ctx(), self.id.0, &mut parent) })?;
        Ok((parent != 0).then(|| Node::new(self.g.clone(), NodeId(parent))))
    }

    /// The node's children, in order.
    pub fn children(&self) -> Result<Vec<Node>> {
        let mut nodes = GnoblinNodes {
            ptr: core::ptr::null(),
            len: 0,
        };
        // SAFETY: `node_children` fills `nodes`, a live local; the array it points at is copied before the next ABI call.
        check(unsafe { (self.g.api().node_children)(self.g.ctx(), self.id.0, &mut nodes) })?;
        let ids: &[u64] = if nodes.ptr.is_null() {
            &[]
        } else {
            // SAFETY: the core lends `len` ids at `ptr` until the next ABI call on this thread.
            unsafe { std::slice::from_raw_parts(nodes.ptr, nodes.len) }
        };
        Ok(ids.iter().map(|id| Node::new(self.g.clone(), NodeId(*id))).collect())
    }

    /// Reads a property by its schema name (`title`, `x`, `opacity`). A property nothing has set reads as its schema default.
    pub fn get(&self, property: &str) -> Result<Value> {
        let class = self.class()?;
        let member = self.g.member(class, MemberKind::Property, property)?;
        let mut raw = GnoblinValue {
            size: size_of::<GnoblinValue>() as u32,
            kind: 0,
            flags: 0,
            reserved: 0,
            bits: 0,
            ptr: core::ptr::null::<c_void>(),
            len: 0,
        };
        // SAFETY: `node_get` fills `raw`, a live local.
        check(unsafe { (self.g.api().node_get)(self.g.ctx(), self.id.0, member.id, &mut raw) })?;
        // SAFETY: the core filled `raw` with pointers that are valid until the value is freed, which happens right after.
        let value = unsafe { Value::from_raw(&raw) };
        // SAFETY: `raw` is a value the core lent, freed once.
        let _ = unsafe { (self.g.api().value_free)(&mut raw) };
        value.ok_or_else(|| {
            CoreError::new(
                ErrorKind::Unsupported,
                format!("`{property}` is of a kind this SDK cannot read"),
            )
        })
    }

    /// Reads a property as a number.
    pub fn get_f64(&self, property: &str) -> Result<f64> {
        let value = self.get(property)?;
        value
            .as_f64()
            .ok_or_else(|| CoreError::new(ErrorKind::Type, format!("`{property}` is {value:?}, not a number")))
    }

    /// Reads a property as a boolean.
    pub fn get_bool(&self, property: &str) -> Result<bool> {
        let value = self.get(property)?;
        value
            .as_bool()
            .ok_or_else(|| CoreError::new(ErrorKind::Type, format!("`{property}` is {value:?}, not a boolean")))
    }

    /// Reads a property as text.
    pub fn get_text(&self, property: &str) -> Result<String> {
        match self.get(property)? {
            Value::Text(text) => Ok(text),
            other => Err(CoreError::new(
                ErrorKind::Type,
                format!("`{property}` is {other:?}, not text"),
            )),
        }
    }

    /// Reads a property that refers to another node (a seat's `focus`): `None` when it refers to nothing.
    pub fn get_node(&self, property: &str) -> Result<Option<Node>> {
        match self.get(property)? {
            Value::Node(node) if node == NodeId::NIL => Ok(None),
            Value::Node(node) => Ok(Some(Node::new(self.g.clone(), node))),
            other => Err(CoreError::new(
                ErrorKind::Type,
                format!("`{property}` is {other:?}, not a node"),
            )),
        }
    }

    /// Writes a property. The write is queued and leaves with the rest of the tick's writes. A property the schema marks
    /// read-only (`title`) is refused with `Refused`; a value of the wrong kind with `Type`.
    pub fn set(&self, property: &str, value: impl Into<Value>) -> Result<()> {
        self.set_animated(property, value, 0.0)
    }

    /// Writes a property so that the compositor plays the change over `seconds` (zero is instant). Only a property the schema
    /// marks animated (`x`, `opacity`, ...) takes a duration.
    pub fn set_animated(&self, property: &str, value: impl Into<Value>, seconds: f64) -> Result<()> {
        let class = self.class()?;
        let member = self.g.member(class, MemberKind::Property, property)?;
        let value = value.into();
        // SAFETY: `write_set` reads the value for the call, which `with_raw` keeps alive.
        value.with_raw(|raw| {
            check(unsafe { (self.g.api().write_set)(self.g.ctx(), self.id.0, member.id, raw, seconds) })
        })
    }

    /// Calls a method of the node (`close`, `kill`) with `args`, which are checked against the method's parameters.
    pub fn call(&self, method: &str, args: &[Value]) -> Result<()> {
        self.call_inner(method, args, None).map(|_| ())
    }

    /// Calls a method and runs `on_answer` once, on the shell thread, when the compositor answers. A call the compositor gives up
    /// on is answered with an error. Cancel the wait with [`PendingCall::cancel`].
    pub fn call_with<F>(&self, method: &str, args: &[Value], on_answer: F) -> Result<PendingCall>
    where
        F: FnOnce(Answer) + 'static,
    {
        self.call_inner(method, args, Some(Box::new(on_answer)))?
            .ok_or_else(|| CoreError::new(ErrorKind::Failed, "the core gave no handle for the pending call"))
    }

    fn call_inner(
        &self,
        method: &str,
        args: &[Value],
        on_answer: Option<Box<dyn FnOnce(Answer)>>,
    ) -> Result<Option<PendingCall>> {
        let class = self.class()?;
        let member = self.g.member(class, MemberKind::Method, method)?;
        let slot = on_answer.map(|f| Box::into_raw(Box::new(CallbackSlot { f: Some(f) })));
        let (cb, user, drop_user): (Option<abi::GnoblinAnswerFn>, *mut c_void, Option<abi::GnoblinDropFn>) = match slot
        {
            Some(slot) => (
                Some(answer_tramp),
                slot.cast(),
                Some(drop_slot::<CallbackSlot<Option<Box<dyn FnOnce(Answer)>>>>),
            ),
            None => (None, core::ptr::null_mut(), None),
        };
        let mut pending: GnoblinPending = 0;
        let mut raws: Vec<GnoblinValue> = Vec::with_capacity(args.len());
        let rc = with_raws(args, &mut raws, &mut |raws| {
            // SAFETY: `write_call` reads the arguments for the call; `user` and `drop_user` are the slot's, which the core
            // adopts on success.
            unsafe {
                (self.g.api().write_call)(
                    self.g.ctx(),
                    self.id.0,
                    member.id,
                    raws.as_ptr(),
                    raws.len(),
                    cb,
                    user,
                    drop_user,
                    &mut pending,
                )
            }
        });
        if rc < abi::E_OK {
            if let Some(slot) = slot {
                // SAFETY: the core refused the call, so it has not adopted the slot and nothing else owns the box.
                drop(unsafe { Box::from_raw(slot) });
            }
            return Err(CoreError::from_code(rc));
        }
        if slot.is_some() {
            crate::gnoblin::REGISTERED.fetch_add(1, Ordering::Relaxed);
        }
        Ok((pending != 0).then(|| PendingCall {
            g: self.g.clone(),
            pending,
        }))
    }

    /// Inserts the node under `parent` at `index` (`None` appends, which is on top), moving it from where it was.
    pub fn insert_into(&self, parent: &Node, index: Option<usize>) -> Result<()> {
        let index = index.map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        // SAFETY: `write_insert` takes the context handle, two node ids and an index.
        check(unsafe { (self.g.api().write_insert)(self.g.ctx(), parent.id.0, self.id.0, index) })
    }

    /// Destroys the node and everything beneath it. Destroying a window the compositor owns is not what this is for: close it.
    pub fn destroy(&self) -> Result<()> {
        // SAFETY: `write_destroy` takes the context handle and a node id.
        check(unsafe { (self.g.api().write_destroy)(self.g.ctx(), self.id.0) })
    }
}

/// Packs `values` as ABI structs that borrow from them and calls `f` with the array.
fn with_raws<R>(values: &[Value], acc: &mut Vec<GnoblinValue>, f: &mut dyn FnMut(&[GnoblinValue]) -> R) -> R {
    match values.split_first() {
        None => f(acc),
        Some((first, rest)) => first.with_raw(|raw| {
            acc.push(*raw);
            let result = with_raws(rest, acc, f);
            acc.pop();
            result
        }),
    }
}

/// The answer to a call or a flush, to the closure behind `user`. The closure runs once.
pub(crate) unsafe extern "C" fn answer_tramp(user: *mut c_void, answer: *const GnoblinAnswer) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<CallbackSlot<Option<Box<dyn FnOnce(Answer)>>>>` leaked by the call; the core hands it back on
    // the shell thread and does not free it before `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<CallbackSlot<Option<Box<dyn FnOnce(Answer)>>>>() };
    // SAFETY: the core passes a pointer valid until this callback returns, or null.
    let Some(answer) = (unsafe { answer.as_ref() }) else {
        return abi::E_ERR;
    };
    let result = if answer.result.ptr.is_null() {
        Vec::new()
    } else {
        // SAFETY: the core lends `len` bytes at `ptr` until the callback returns.
        unsafe { std::slice::from_raw_parts(answer.result.ptr, answer.result.len) }.to_vec()
    };
    let error = copy_str(&answer.error);
    let answer = Answer {
        result,
        error: (!error.is_empty() || answer.code != abi::E_OK).then_some(error),
    };
    match slot.f.take() {
        Some(f) => match catch_unwind(AssertUnwindSafe(|| f(answer))) {
            Ok(()) => abi::E_OK,
            Err(_) => abi::E_PANIC,
        },
        None => abi::E_OK,
    }
}

/// A window: a node of class `window`, with the accessors a window manager needs. It dereferences to its [`Node`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window(Node);

/// A window an application owns, in the desktop's own word for it. The core API has one kind of window.
pub type Client = Window;

impl Deref for Window {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.0
    }
}

impl Window {
    pub(crate) fn new(g: Gnoblin, id: NodeId) -> Window {
        Window(Node::new(g, id))
    }

    /// The window made of `node`, if `node` is a window.
    pub fn from_node(node: Node) -> Result<Window> {
        let window = node.gnoblin().class("window")?;
        if node.class()? == window {
            Ok(Window(node))
        } else {
            Err(CoreError::new(
                ErrorKind::Type,
                format!(
                    "node {} is a {}, not a window",
                    node.id().0,
                    node.class_name().unwrap_or_default()
                ),
            ))
        }
    }

    /// The window's node.
    pub fn node(&self) -> &Node {
        &self.0
    }

    /// The window's title, as the application set it.
    pub fn title(&self) -> Result<String> {
        self.0.get_text("title")
    }

    /// The application id the window's client declared (`org.gnome.Nautilus`).
    pub fn app_id(&self) -> Result<String> {
        self.0.get_text("app_id")
    }

    /// The window's rectangle: the part of its surface the user sees as the window.
    pub fn geometry(&self) -> Result<Rect> {
        Ok(Rect {
            x: self.0.get_f64("x")?,
            y: self.0.get_f64("y")?,
            width: self.0.get_f64("width")?,
            height: self.0.get_f64("height")?,
        })
    }

    /// Places and sizes the window. The compositor turns the size into the client's configure.
    pub fn set_geometry(&self, rect: Rect) -> Result<()> {
        self.set_position(rect.x, rect.y)?;
        self.set_size(rect.width, rect.height)
    }

    /// Moves the window.
    pub fn set_position(&self, x: f64, y: f64) -> Result<()> {
        self.0.set("x", x)?;
        self.0.set("y", y)
    }

    /// Resizes the window.
    pub fn set_size(&self, width: f64, height: f64) -> Result<()> {
        self.0.set("width", width)?;
        self.0.set("height", height)
    }

    /// Whether the window has the keyboard.
    pub fn is_focused(&self) -> Result<bool> {
        self.0.get_bool("focused")
    }

    /// Whether the window is maximised.
    pub fn is_maximised(&self) -> Result<bool> {
        self.0.get_bool("maximised")
    }

    /// Whether the window is fullscreen.
    pub fn is_fullscreen(&self) -> Result<bool> {
        self.0.get_bool("fullscreen")
    }

    /// Whether the window is minimised.
    pub fn is_minimised(&self) -> Result<bool> {
        self.0.get_bool("minimised")
    }

    /// Maximises or restores the window.
    pub fn set_maximised(&self, maximised: bool) -> Result<()> {
        self.0.set("maximised", maximised)
    }

    /// Makes the window fullscreen or restores it.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<()> {
        self.0.set("fullscreen", fullscreen)
    }

    /// Minimises or restores the window.
    pub fn set_minimised(&self, minimised: bool) -> Result<()> {
        self.0.set("minimised", minimised)
    }

    /// Shows or hides the window.
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        self.0.set("visible", visible)
    }

    /// Sets the window's opacity, 0 to 1.
    pub fn set_opacity(&self, opacity: f64) -> Result<()> {
        self.0.set("opacity", opacity)
    }

    /// Asks the client to close the window.
    pub fn close(&self) -> Result<()> {
        self.0.call("close", &[])
    }

    /// Ends the client's connection: for a window that does not answer `close`.
    pub fn kill(&self) -> Result<()> {
        self.0.call("kill", &[])
    }

    /// Gives the window the keyboard: writes it as its seat's `focus`.
    pub fn focus(&self) -> Result<()> {
        let seat = self.0.gnoblin().main_seat()?;
        seat.set("focus", self.0.id())
    }

    /// Raises the window above its siblings.
    pub fn raise(&self) -> Result<()> {
        let root = self.0.gnoblin().root()?;
        self.0.insert_into(&root, None)
    }

    /// Lowers the window beneath its siblings.
    pub fn lower(&self) -> Result<()> {
        let root = self.0.gnoblin().root()?;
        self.0.insert_into(&root, Some(0))
    }
}

impl Gnoblin {
    /// A handle on the node with this id. It does not check that the node exists: use [`Node::is_alive`].
    pub fn node(&self, id: NodeId) -> Node {
        Node::new(self.clone(), id)
    }

    /// The root of the tree. Attaches to the tree if this generation has not.
    pub fn root(&self) -> Result<Node> {
        self.attach()?;
        let mut root = 0;
        // SAFETY: `tree_root` writes one node id through the pointer it is given, a live local.
        check(unsafe { (self.api().tree_root)(self.ctx(), &mut root) })?;
        Ok(Node::new(self.clone(), NodeId(root)))
    }

    /// Every window in the tree, in stacking order (the last is on top). Attaches to the tree if this generation has not.
    pub fn windows(&self) -> Result<Vec<Window>> {
        let window = self.class("window")?;
        let mut windows = Vec::new();
        for child in self.root()?.children()? {
            if child.class()? == window {
                windows.push(Window(child));
            }
        }
        Ok(windows)
    }

    /// The window with this id, if it is one.
    pub fn window(&self, id: NodeId) -> Result<Window> {
        self.attach()?;
        Window::from_node(self.node(id))
    }

    /// A handle on the window with this id, without checking that it is one: like [`Gnoblin::node`], for an id the caller knows
    /// (a window that has just closed, whose id an event carries). Reads and writes through it fail with `NoSuchNode` once the
    /// window is gone.
    pub fn window_handle(&self, id: NodeId) -> Window {
        Window::new(self.clone(), id)
    }

    /// Makes a node of class `class` (a tree class such as `canvas`) and returns it. It exists in this shell's reads at once and
    /// in the compositor when the tick's transaction is applied; insert it into the tree to show it.
    pub fn create_node(&self, class: &str) -> Result<Node> {
        self.attach()?;
        let class = self.class(class)?;
        let mut node = 0;
        // SAFETY: `write_create` writes one node id through the pointer it is given, a live local.
        check(unsafe { (self.api().write_create)(self.ctx(), class.0, &mut node) })?;
        Ok(Node::new(self.clone(), NodeId(node)))
    }

    /// The node of class `class` that this shell calls `name`: the one an earlier generation made (a reload runs the shell's
    /// entry again, and it must find its nodes where the last one left them), or a new one when there is none. Its `name`
    /// property is `name`. An adopted node is exactly as it was: its children and its values are untouched, so a canvas keeps
    /// what it drew and the compositor sees no node come or go.
    ///
    /// The node outlives the generation that made it. For one that should not, see
    /// [`adopt_ephemeral_node`](Self::adopt_ephemeral_node).
    pub fn adopt_node(&self, class: &str, name: &str) -> Result<Adoption> {
        self.adopt_with(class, name, 0)
    }

    /// As [`adopt_node`](Self::adopt_node), for a node that dies with its generation: when the next generation takes over, the
    /// compositor destroys every ephemeral node it did not adopt during its `init`. The place for a toast, an overlay or a
    /// debug marker that a reload should clear unless the new code still wants it.
    pub fn adopt_ephemeral_node(&self, class: &str, name: &str) -> Result<Adoption> {
        self.adopt_with(class, name, abi::NODE_FLAG_EPHEMERAL)
    }

    fn adopt_with(&self, class: &str, name: &str, flags: u32) -> Result<Adoption> {
        self.attach()?;
        let class = self.class(class)?;
        let mut node = 0;
        // SAFETY: `node_adopt` reads `name.len()` bytes at the pointer for the call and writes one node id through `node`, a
        // live local.
        let rc = unsafe { (self.api().node_adopt)(self.ctx(), class.0, name.as_ptr(), name.len(), flags, &mut node) };
        check(rc)?;
        Ok(Adoption {
            node: Node::new(self.clone(), NodeId(node)),
            adopted: rc == abi::ADOPTED,
        })
    }

    /// Sends the tick's writes now, and runs `on_done` once, on the shell thread, when everything sent so far has been answered:
    /// with the serial the compositor applied it at, or the refusal. A shell rarely needs this: writes leave by themselves when the
    /// callback that made them returns.
    pub fn flush<F>(&self, on_done: F) -> Result<PendingCall>
    where
        F: FnOnce(Result<u64>) + 'static,
    {
        self.attach()?;
        let wrapped: Box<dyn FnOnce(Answer)> = Box::new(move |answer| {
            on_done(match answer.error {
                Some(error) => Err(CoreError::new(ErrorKind::Refused, error)),
                None => Ok(answer.text().trim().parse().unwrap_or(0)),
            });
        });
        let slot = Box::into_raw(Box::new(CallbackSlot { f: Some(wrapped) }));
        let mut pending: GnoblinPending = 0;
        // SAFETY: `flush` adopts `slot` on success; the answer trampoline and `drop_slot` are its owners then.
        let rc = unsafe {
            (self.api().flush)(
                self.ctx(),
                Some(answer_tramp),
                slot.cast(),
                Some(drop_slot::<CallbackSlot<Option<Box<dyn FnOnce(Answer)>>>>),
                &mut pending,
            )
        };
        if rc < abi::E_OK {
            // SAFETY: the core refused the flush, so nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            return Err(CoreError::from_code(rc));
        }
        crate::gnoblin::REGISTERED.fetch_add(1, Ordering::Relaxed);
        Ok(PendingCall {
            g: self.clone(),
            pending,
        })
    }
}
