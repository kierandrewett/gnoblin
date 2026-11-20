//! Properties of a scene node kept in step with what a function reads: `node.properties.bind_number("opacity", compute)`.
//!
//! `data/js/gnoblin-core/properties.ts` is the reference. The library runs `compute` under dependency tracking: every property read
//! through the object protocol while it runs is recorded with the value it gave, whether the object is one of this library's or one
//! of the object model's (a window, a collection's length). The result is written to the property. After a turn in which a window or
//! an output changed, the recorded reads are compared with what they give now, and a binding whose reads changed runs again, up to
//! eight times in one turn: one still changing after that is left as it is and reported once.
//!
//! A binding runs the first time from the turn after the call that made it, because a closure of the shell never runs inside a call
//! of the protocol. A second binding of a property replaces the first; the owner's end, `dispose` and `unbind` end one.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::client::Api;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Callable, ClassId, Fail, MemberId, Obj, Res, Val};

use super::registry::{NodeHandle, SceneWorld, world_of};
use crate::math::triple_val;
use crate::object::{core_api, with_lib};

/// The most passes of dirty bindings one turn runs.
const MAX_PASSES: usize = 8;

/// What a binding writes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Number,
    Flag,
    Text,
    Numbers,
}

/// A property read while a binding computed, and what it gave.
struct Read {
    /// Whether the object is one of this library's (else the object model's).
    ours: bool,
    obj: Obj,
    class: ClassId,
    member: MemberId,
    seen: Val,
}

struct NodeBindingState {
    id: u64,
    owner: Obj,
    node: u64,
    property: String,
    shape: Shape,
    compute: Rc<dyn Callable>,
    reads: Vec<Read>,
    disposed: bool,
    /// The failure last reported, so the same one is reported once and not every turn.
    failure: Option<String>,
    /// Never run yet.
    fresh: bool,
}

/// The bindings of a generation.
#[derive(Default)]
pub(crate) struct SceneBindings {
    list: Vec<NodeBindingState>,
    next: u64,
}

impl SceneBindings {
    /// Whether any binding is live.
    pub(crate) fn active(&self) -> bool {
        self.list.iter().any(|binding| !binding.disposed)
    }

    /// The owner's object ended: its bindings end.
    pub(crate) fn owner_ended(&mut self, owner: Option<Obj>) {
        if let Some(owner) = owner {
            self.list
                .iter_mut()
                .filter(|binding| binding.owner == owner)
                .for_each(|binding| binding.disposed = true);
        }
    }

    fn live(&self, id: u64) -> bool {
        self.list.iter().any(|binding| binding.id == id && !binding.disposed)
    }
}

/// Whether two values are the same, as a binding's read is.
fn same(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::None, Val::None) => true,
        (Val::Bool(a), Val::Bool(b)) => a == b,
        (Val::Int(a), Val::Int(b)) => a == b,
        (Val::Uint(a), Val::Uint(b)) => a == b,
        (Val::Double(a), Val::Double(b)) => a == b,
        (Val::Text(a), Val::Text(b)) => a == b,
        (Val::Node(a), Val::Node(b)) => a == b,
        (Val::List(a), Val::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Val::Object { obj: a, .. }, Val::Object { obj: b, .. }) => a == b,
        (Val::Enum { value: a, .. }, Val::Enum { value: b, .. }) => a == b,
        (Val::Record { fields: a, .. }, Val::Record { fields: b, .. }) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        _ => false,
    }
}

thread_local! {
    /// The reads of the binding that is running, of this library's objects and of the object model's.
    static OURS: RefCell<Vec<(Obj, ClassId, MemberId)>> = const { RefCell::new(Vec::new()) };
    static THEIRS: RefCell<Vec<(Obj, ClassId, MemberId)>> = const { RefCell::new(Vec::new()) };
}

impl SceneWorld {
    /// What a read gives now.
    fn read_now(&self, core: Option<&Api>, read: &Read) -> Option<Val> {
        if read.ours {
            with_lib(|lib| lib.get(self.ctx, read.obj, read.member)).ok()
        } else {
            core?.get(read.obj, read.member).ok()
        }
    }

    /// Writes what a binding computed to its property, through the setter of the node's class.
    fn write_result(&self, owner: Obj, property: &str, shape: Shape, value: Val) -> Res<()> {
        let site = format!("the binding of `{property}`");
        let value = match (shape, value) {
            (Shape::Number, value @ Val::Double(_))
            | (Shape::Flag, value @ Val::Bool(_))
            | (Shape::Text, value @ Val::Text(_)) => value,
            (Shape::Numbers, Val::List(items)) => {
                // A rotation or a scale is a vector in the catalog; a list of three numbers is what the shell computes.
                let numbers: Vec<f64> = items.iter().map(Val::double).collect::<Res<_>>()?;
                match <[f64; 3]>::try_from(numbers) {
                    Ok(triple) => triple_val(triple),
                    Err(_) => {
                        return Err(Fail::type_error(format!(
                            "{site} gave a list that is not three numbers"
                        )));
                    }
                }
            }
            (_, other) => return Err(Fail::type_error(format!("{site} gave {}", other.kind_name()))),
        };
        let ctx = self.ctx;
        with_lib(|lib| {
            let class = lib.class_of(ctx, owner)?;
            let writable = lib
                .member_id(class, abi::MEMBER_PROPERTY, property)
                .ok()
                .filter(|member| {
                    lib.catalog()
                        .class(class)
                        .and_then(|def| def.member(*member))
                        .is_some_and(|def| def.flags & abi::MEMBER_READONLY == 0)
                })
                .ok_or_else(|| Fail::no_such(format!("{property} is not a writable property")))?;
            lib.set(ctx, owner, writable, value)
        })
    }

    /// Runs the binding `id`: computes under tracking, writes the result and records what it read.
    fn run_binding(&self, id: u64) {
        let Some((owner, node, property, shape, compute)) = self.engine.borrow().list.iter().find_map(|binding| {
            (binding.id == id && !binding.disposed).then(|| {
                (
                    binding.owner,
                    binding.node,
                    binding.property.clone(),
                    binding.shape,
                    Rc::clone(&binding.compute),
                )
            })
        }) else {
            return;
        };
        OURS.with(|reads| reads.borrow_mut().clear());
        THEIRS.with(|reads| reads.borrow_mut().clear());
        let core = core_api(self.ctx).ok();
        with_lib(|lib| {
            lib.set_tracker(Some(Box::new(|obj, class, member| {
                OURS.with(|reads| reads.borrow_mut().push((obj, class, member)));
            })));
        });
        let guard = core.as_ref().and_then(|api| {
            api.track(|obj, class, member| {
                THEIRS.with(|reads| reads.borrow_mut().push((obj, class, member)));
            })
            .ok()
        });
        let computed = compute.call(0, &[]);
        drop(guard);
        with_lib(|lib| lib.set_tracker(None));
        let mut ours = OURS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
        let mut theirs = THEIRS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
        ours.dedup();
        theirs.dedup();
        let written = computed.and_then(|value| self.write_result(owner, &property, shape, value));
        // What it read is read again now, after its own write, so the write is not a reason to run it again.
        let mut reads = Vec::with_capacity(ours.len() + theirs.len());
        for (is_ours, list) in [(true, ours), (false, theirs)] {
            for (obj, class, member) in list {
                let mut read = Read {
                    ours: is_ours,
                    obj,
                    class,
                    member,
                    seen: Val::None,
                };
                if let Some(now) = self.read_now(core.as_ref(), &read) {
                    read.seen = now;
                    reads.push(read);
                }
            }
        }
        let failure = written.err().map(|fail| fail.message);
        let label = self.label(node);
        let mut engine = self.engine.borrow_mut();
        let Some(binding) = engine.list.iter_mut().find(|binding| binding.id == id) else {
            return;
        };
        binding.reads = reads;
        binding.fresh = false;
        if failure != binding.failure {
            if let Some(message) = &failure {
                drop(engine);
                self.warn(&format!("{label}.{property} threw: {message}"));
                if let Some(binding) = self
                    .engine
                    .borrow_mut()
                    .list
                    .iter_mut()
                    .find(|binding| binding.id == id)
                {
                    binding.failure = failure;
                }
            } else {
                binding.failure = failure;
            }
        }
    }

    /// Runs the bindings that have not run and the ones whose reads changed, and again for the ones their writes made dirty.
    pub(crate) fn binding_turn(&self) {
        if !self.engine.borrow().active() {
            self.engine.borrow_mut().list.retain(|binding| !binding.disposed);
            return;
        }
        // A binding that has not run yet runs once, in the order the bindings were made: the passes are for what runs again.
        let fresh: Vec<u64> = self
            .engine
            .borrow()
            .list
            .iter()
            .filter(|binding| binding.fresh && !binding.disposed)
            .map(|binding| binding.id)
            .collect();
        for id in fresh {
            self.run_binding(id);
        }
        let core = core_api(self.ctx).ok();
        for pass in 0..=MAX_PASSES {
            let dirty: Vec<u64> = {
                let mut engine = self.engine.borrow_mut();
                engine.list.retain(|binding| !binding.disposed);
                engine
                    .list
                    .iter()
                    .filter(|binding| {
                        binding.reads.iter().any(|read| {
                            self.read_now(core.as_ref(), read)
                                .is_none_or(|now| !same(&now, &read.seen))
                        })
                    })
                    .map(|binding| binding.id)
                    .collect()
            };
            if dirty.is_empty() {
                return;
            }
            if pass == MAX_PASSES {
                for id in dirty {
                    self.never_settles(id, core.as_ref());
                }
                return;
            }
            for id in dirty {
                self.run_binding(id);
            }
        }
    }

    /// A binding still changing after the passes: it is left as it is and reported once, with the properties it reads.
    fn never_settles(&self, id: u64, core: Option<&Api>) {
        let (label, property, reads, failure) = {
            let engine = self.engine.borrow();
            let Some(binding) = engine.list.iter().find(|binding| binding.id == id) else {
                return;
            };
            let reads: Vec<String> = binding.reads.iter().filter_map(|read| self.describe(read)).collect();
            (
                self.label(binding.node),
                binding.property.clone(),
                reads,
                binding.failure.clone(),
            )
        };
        let message = format!(
            "still changing after {MAX_PASSES} passes in one frame (it reads {}); a binding that writes what another reads, which writes what it reads, never settles",
            if reads.is_empty() {
                "nothing".to_owned()
            } else {
                reads.join(", ")
            }
        );
        if failure.as_deref() != Some(&message) {
            self.warn(&format!("{label}.{property} threw: {message}"));
        }
        if let Some(binding) = self
            .engine
            .borrow_mut()
            .list
            .iter_mut()
            .find(|binding| binding.id == id)
        {
            binding.failure = Some(message);
            // Left as it is: what it read is what it reads now.
            for read in &mut binding.reads {
                if let Some(now) = self.read_now(core, read) {
                    read.seen = now;
                }
            }
        }
    }

    /// What a read of one of this library's nodes names, for the message of a binding that never settles: `#<node>.<property>`.
    fn describe(&self, read: &Read) -> Option<String> {
        if !read.ours {
            return None;
        }
        let property = with_lib(|lib| {
            lib.catalog()
                .class(read.class)
                .and_then(|def| def.member(read.member))
                .map(|member| member.name.to_owned())
        })?;
        let node = self
            .entries
            .borrow()
            .iter()
            .find(|(_, entry)| entry.obj == read.obj)
            .map(|(node, _)| *node)?;
        Some(format!("#{node}.{property}"))
    }

    /// Ends the binding `id`.
    fn dispose_binding(&self, id: u64) {
        if let Some(binding) = self
            .engine
            .borrow_mut()
            .list
            .iter_mut()
            .find(|binding| binding.id == id)
        {
            binding.disposed = true;
        }
    }

    /// Binds `property` of the node to what `compute` gives: the first binding of a property is replaced.
    fn bind(
        self: &Rc<Self>,
        owner: Obj,
        node: u64,
        property: &str,
        shape: Shape,
        compute: Rc<dyn Callable>,
    ) -> Res<u64> {
        if !self.alive(node) {
            return Err(Fail::stale(format!(
                "{}.properties.bind({property}): the object was destroyed",
                self.label(node)
            )));
        }
        let id = {
            let mut engine = self.engine.borrow_mut();
            for binding in engine
                .list
                .iter_mut()
                .filter(|binding| binding.owner == owner && binding.property == property)
            {
                binding.disposed = true;
            }
            engine.next += 1;
            let id = engine.next;
            engine.list.push(NodeBindingState {
                id,
                owner,
                node,
                property: property.to_owned(),
                shape,
                compute,
                reads: Vec::new(),
                disposed: false,
                failure: None,
                fresh: true,
            });
            id
        };
        self.schedule();
        Ok(id)
    }
}

/// The bindings of one node: `node.properties`.
pub(crate) struct BindingsHandle {
    pub(crate) world: Rc<SceneWorld>,
    pub(crate) owner: Obj,
    pub(crate) node: u64,
}

/// One binding: the way to end it.
struct BindingHandle {
    world: Rc<SceneWorld>,
    id: u64,
    property: String,
}

fn bind_with(h: &BindingsHandle, cx: &mut Cx<'_>, args: &[Val], shape: Shape) -> Res<Val> {
    let property = args[0].text()?.to_owned();
    let compute = Rc::clone(args[1].closure()?);
    let id = h.world.bind(h.owner, h.node, &property, shape, compute)?;
    let obj = cx.spawn(
        "node_binding",
        BindingHandle {
            world: Rc::clone(&h.world),
            id,
            property,
        },
    )?;
    cx.handout(obj)
}

/// Makes the object of the bindings of a node, for `properties`.
pub(crate) fn bindings_val(cx: &mut Cx<'_>, handle: &NodeHandle) -> Res<Val> {
    let obj = cx.spawn(
        "node_bindings",
        BindingsHandle {
            world: world_of(cx)?,
            owner: cx.me(),
            node: handle.node,
        },
    )?;
    cx.handout(obj)
}

pub(crate) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<BindingHandle>("node_binding")
        .ctor(|_cx, _inits| Err(Fail::refused("a binding is made by `properties.bind_*`")))
        .get("property", |h, _| Ok(Val::Text(h.property.clone())))
        .get("alive", |h, _| Ok(Val::Bool(h.world.engine.borrow().live(h.id))))
        .method("dispose", |h, _, _| {
            h.world.dispose_binding(h.id);
            Ok(Val::None)
        });
    let _ = builder
        .class::<BindingsHandle>("node_bindings")
        .ctor(|_cx, _inits| Err(Fail::refused("the bindings of a node are `node.properties`")))
        .method("bind_number", |h, cx, args| bind_with(h, cx, args, Shape::Number))
        .method("bind_flag", |h, cx, args| bind_with(h, cx, args, Shape::Flag))
        .method("bind_text", |h, cx, args| bind_with(h, cx, args, Shape::Text))
        .method("bind_numbers", |h, cx, args| bind_with(h, cx, args, Shape::Numbers))
        .method("unbind", |h, _, args| {
            let property = args[0].text()?;
            let mut engine = h.world.engine.borrow_mut();
            let mut found = false;
            for binding in engine
                .list
                .iter_mut()
                .filter(|binding| binding.owner == h.owner && binding.property == property && !binding.disposed)
            {
                binding.disposed = true;
                found = true;
            }
            Ok(Val::Bool(found))
        })
        .method("dispose", |h, _, _| {
            h.world.engine.borrow_mut().owner_ended(Some(h.owner));
            Ok(Val::None)
        });
}
