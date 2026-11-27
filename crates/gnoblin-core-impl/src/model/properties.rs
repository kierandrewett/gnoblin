//! `property_bindings`: a property kept in step with the properties and collections a function reads.
//!
//! `bind_double("opacity", compute)` runs `compute` under dependency tracking: every property read through the core's object table while
//! it runs is recorded with the value it gave. The result is written to the property. After each turn the recorded reads are compared
//! with what they give now, and a binding whose reads changed runs again, up to eight times in one turn: one still changing after that
//! is left as it is and reported once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Callable, ClassId, ErrorCode, Fail, MemberId, Obj, Res, Val};

/// The most passes of dirty bindings one turn runs.
const MAX_PASSES: usize = 8;

/// What a binding writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindShape {
    Double,
    Bool,
    Text,
    List,
}

struct PropBinding {
    id: u64,
    owner: Obj,
    property: String,
    shape: BindShape,
    compute: Rc<dyn Callable>,
    /// What the last run read, and what it saw.
    reads: Vec<(Obj, ClassId, MemberId, Val)>,
    disposed: bool,
    /// The failure last reported, so the same one is reported once and not every turn.
    failure: Option<String>,
}

#[derive(Default)]
struct BindingEngine {
    bindings: Vec<PropBinding>,
    next: u64,
}

thread_local! {
    static ENGINES: RefCell<HashMap<GnoblinCtx, BindingEngine>> = RefCell::new(HashMap::new());
    /// The reads of the binding that is running.
    static READS: RefCell<Vec<(Obj, ClassId, MemberId)>> = const { RefCell::new(Vec::new()) };
}

/// The generation ended: its bindings go with it.
pub(crate) fn retire(ctx: GnoblinCtx) {
    ENGINES.with(|engines| engines.borrow_mut().remove(&ctx));
}

/// The owner is gone: its bindings end.
pub(super) fn dispose_of(cx: &Cx<'_>, owner: Obj) {
    ENGINES.with(|engines| {
        if let Some(engine) = engines.borrow_mut().get_mut(&cx.ctx()) {
            engine
                .bindings
                .iter_mut()
                .filter(|binding| binding.owner == owner)
                .for_each(|binding| binding.disposed = true);
        }
    });
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

/// Runs the binding `id` of the generation: computes under tracking, writes the result and records what it read.
fn run(cx: &Cx<'_>, id: u64) {
    let ctx = cx.ctx();
    let Some((owner, property, shape, compute)) = ENGINES.with(|engines| {
        engines
            .borrow()
            .get(&ctx)
            .and_then(|engine| {
                engine
                    .bindings
                    .iter()
                    .find(|binding| binding.id == id && !binding.disposed)
            })
            .map(|binding| {
                (
                    binding.owner,
                    binding.property.clone(),
                    binding.shape,
                    Rc::clone(&binding.compute),
                )
            })
    }) else {
        return;
    };
    READS.with(|reads| reads.borrow_mut().clear());
    crate::object::with_lib(|lib| {
        lib.set_tracker(Some(Box::new(|obj, class, member| {
            READS.with(|reads| reads.borrow_mut().push((obj, class, member)));
        })));
    });
    let computed = compute.call(0, &[]);
    crate::object::with_lib(|lib| lib.set_tracker(None));
    let mut seen_reads = READS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
    seen_reads.dedup();
    let site = format!("the binding of `{property}`");
    let written = computed.and_then(|value| {
        let value = match (shape, value) {
            // A whole number is a number: a script's `c.x + 1` crosses as an integer.
            (BindShape::Double, Val::Int(number)) => Val::Double(number as f64),
            (BindShape::Double, value @ Val::Double(_))
            | (BindShape::Bool, value @ Val::Bool(_))
            | (BindShape::Text, value @ Val::Text(_)) => value,
            (BindShape::List, value @ Val::List(_)) => value,
            (_, other) => return Err(Fail::type_error(format!("{site} gave {}", other.kind_name()))),
        };
        crate::object::with_lib(|lib| {
            let class = lib.class_of(ctx, owner)?;
            let member = lib.member_id(class, abi::MEMBER_PROPERTY, &property)?;
            lib.set(ctx, owner, member, value)
        })
    });
    // What it read is read again now, after its own write, so the write is not a reason to run it again.
    let reads: Vec<(Obj, ClassId, MemberId, Val)> = seen_reads
        .into_iter()
        .filter_map(|(obj, class, member)| {
            crate::object::with_lib(|lib| lib.get(ctx, obj, member))
                .ok()
                .map(|value| (obj, class, member, value))
        })
        .collect();
    let failure = written.err().map(|fail| fail.message);
    ENGINES.with(|engines| {
        let mut engines = engines.borrow_mut();
        let Some(binding) = engines
            .get_mut(&ctx)
            .and_then(|engine| engine.bindings.iter_mut().find(|binding| binding.id == id))
        else {
            return;
        };
        binding.reads = reads;
        if failure != binding.failure {
            if let Some(message) = &failure {
                cx.log(abi::LOG_NOTICE, &format!("{site}: {message}"));
            }
            binding.failure = failure;
        }
    });
}

/// Runs the bindings whose reads changed, and again for the ones their writes made dirty.
pub(super) fn turn(cx: &Cx<'_>) {
    let ctx = cx.ctx();
    for pass in 0..=MAX_PASSES {
        let dirty: Vec<u64> = ENGINES.with(|engines| {
            let mut engines = engines.borrow_mut();
            let Some(engine) = engines.get_mut(&ctx) else {
                return Vec::new();
            };
            engine.bindings.retain(|binding| !binding.disposed);
            engine
                .bindings
                .iter()
                .filter(|binding| {
                    binding.reads.iter().any(|(obj, _, member, seen)| {
                        crate::object::with_lib(|lib| lib.get(ctx, *obj, *member)).map_or(true, |now| !same(&now, seen))
                    })
                })
                .map(|binding| binding.id)
                .collect()
        });
        if dirty.is_empty() {
            return;
        }
        if pass == MAX_PASSES {
            ENGINES.with(|engines| {
                let mut engines = engines.borrow_mut();
                if let Some(engine) = engines.get_mut(&ctx) {
                    for binding in engine.bindings.iter_mut().filter(|binding| dirty.contains(&binding.id)) {
                        let message = format!(
                            "still changing after {MAX_PASSES} passes in one turn; a binding that writes what another reads, which writes what it reads, never settles"
                        );
                        if binding.failure.as_deref() != Some(&message) {
                            cx.log(
                                abi::LOG_NOTICE,
                                &format!("the binding of `{}`: {message}", binding.property),
                            );
                            binding.failure = Some(message);
                        }
                        // Left as it is: what it reads now is what it has seen.
                        binding.reads.clear();
                    }
                }
            });
            return;
        }
        for id in dirty {
            run(cx, id);
        }
    }
}

/// The bindings of one object.
pub(super) struct BindingsState {
    owner: Obj,
}

/// One binding.
pub(super) struct BindingState {
    ctx: GnoblinCtx,
    id: u64,
}

/// The `properties` object of `owner`: made on first use, owned by `owner`, and made again for the one that asks next.
pub(super) fn bindings_of(cx: &Cx<'_>, owner: Obj, cache: &std::cell::Cell<Option<Obj>>) -> Res<Val> {
    if let Some(held) = cache.get()
        && cx.class_of(held).is_ok()
    {
        return cx.handout(held);
    }
    let bindings = cx.spawn("property_bindings", BindingsState { owner })?;
    cx.set_parent(bindings, owner)?;
    cache.set(Some(bindings));
    cx.handout(bindings)
}

fn bind(shape: BindShape) -> impl Fn(&mut BindingsState, &mut Cx<'_>, &[Val]) -> Res<Val> + 'static {
    move |state, cx, args| {
        let property = args[0].text()?.to_owned();
        let compute = Rc::clone(args[1].closure()?);
        let owner = state.owner;
        let class = cx.class_of(owner)?;
        let member = cx.member_of(class, abi::MEMBER_PROPERTY, &property)?;
        let def = cx
            .class_def_of(class)
            .and_then(|def| def.member(member))
            .ok_or_else(|| Fail::no_such(format!("`{property}` is not a property")))?;
        if def.flags & abi::MEMBER_READONLY != 0 {
            return Err(Fail::read_only(format!("{property} is not a writable property")));
        }
        let ctx = cx.ctx();
        let id = ENGINES.with(|engines| {
            let mut engines = engines.borrow_mut();
            let engine = engines.entry(ctx).or_default();
            // A property has one binding: the new one replaces the old.
            engine
                .bindings
                .iter_mut()
                .filter(|binding| binding.owner == owner && binding.property == property)
                .for_each(|binding| binding.disposed = true);
            engine.next += 1;
            let id = engine.next;
            engine.bindings.push(PropBinding {
                id,
                owner,
                property: property.clone(),
                shape,
                compute,
                reads: Vec::new(),
                disposed: false,
                failure: None,
            });
            id
        });
        // The library never runs a function inside a call: the first run is the next turn's.
        cx.later(move |cx| run(cx, id));
        let binding = cx.spawn("property_binding", BindingState { ctx, id })?;
        cx.handout(binding)
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<BindingsState>("property_bindings")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the bindings of an object are its `properties`",
            ))
        })
        .method("bind_double", bind(BindShape::Double))
        .method("bind_bool", bind(BindShape::Bool))
        .method("bind_string", bind(BindShape::Text))
        .method("bind_list", bind(BindShape::List))
        .method("unbind", |state, cx, args| {
            let property = args[0].text()?;
            let owner = state.owner;
            let found =
                ENGINES.with(|engines| {
                    let mut engines = engines.borrow_mut();
                    let mut found = false;
                    if let Some(engine) = engines.get_mut(&cx.ctx()) {
                        for binding in engine.bindings.iter_mut().filter(|binding| {
                            binding.owner == owner && binding.property == property && !binding.disposed
                        }) {
                            binding.disposed = true;
                            found = true;
                        }
                    }
                    found
                });
            Ok(Val::Bool(found))
        })
        .method("dispose", |state, cx, _| {
            dispose_of(cx, state.owner);
            Ok(Val::None)
        });
    let _ = builder
        .class::<BindingState>("property_binding")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a binding is made with `bind_double` and the others",
            ))
        })
        .get("property", |state, _| {
            let name = ENGINES.with(|engines| {
                engines
                    .borrow()
                    .get(&state.ctx)
                    .and_then(|engine| engine.bindings.iter().find(|binding| binding.id == state.id))
                    .map(|binding| binding.property.clone())
            });
            Ok(Val::Text(name.unwrap_or_default()))
        })
        .get("alive", |state, _| {
            let alive = ENGINES.with(|engines| {
                engines.borrow().get(&state.ctx).is_some_and(|engine| {
                    engine
                        .bindings
                        .iter()
                        .any(|binding| binding.id == state.id && !binding.disposed)
                })
            });
            Ok(Val::Bool(alive))
        })
        .method("dispose", |state, _, _| {
            ENGINES.with(|engines| {
                if let Some(engine) = engines.borrow_mut().get_mut(&state.ctx) {
                    engine
                        .bindings
                        .iter_mut()
                        .filter(|binding| binding.id == state.id)
                        .for_each(|binding| binding.disposed = true);
                }
            });
            Ok(Val::None)
        });
}
