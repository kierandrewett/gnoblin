#![allow(unsafe_code)]
//! The object protocol end to end, in this process, over two small libraries with hand-written catalogs: values of every kind, a
//! closure property, an interface the shell implements, signals and property changes told from the turn, a class whose base class
//! another library serves, and the property-read tracker. The libraries are `gnoblin-object`'s own server behind its own C table.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinObjectApi};
use gnoblin_object::catalog::{ArgDef, CatalogDef, ClassDef, DefaultDef, MemberDef, TypeDef};
use gnoblin_object::client::{Api, closure};
use gnoblin_object::server::{Builder, Cx, Hooks, Library, Provider, api_table};
use gnoblin_object::{Callable, Fail, Val};

const CTX: u64 = 7;

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static PROBE: Library = build_probe();
    static DERIVED: Library = build_derived();
}

fn last_error() -> String {
    LAST.with(|last| last.borrow().clone())
}

fn hooks() -> Hooks {
    Hooks {
        first_use: |_| Ok(()),
        schedule: |_| {},
        set_last: |fail| LAST.with(|last| *last.borrow_mut() = fail.message.clone()),
        log: |_, _, _| {},
        foreign: |ctx, namespace| {
            if namespace == "test.probe" {
                Ok(Api::new(&PROBE_API, ctx, last_error))
            } else {
                Err(Fail::no_such("no such namespace"))
            }
        },
    }
}

struct ProbeLib;
struct DerivedLib;

impl Provider for ProbeLib {
    fn with<R>(work: impl FnOnce(&Library) -> R) -> R {
        PROBE.with(work)
    }
}

impl Provider for DerivedLib {
    fn with<R>(work: impl FnOnce(&Library) -> R) -> R {
        DERIVED.with(work)
    }
}

static PROBE_API: GnoblinObjectApi = api_table::<ProbeLib>(1);
static DERIVED_API: GnoblinObjectApi = api_table::<DerivedLib>(2);

// ---- the catalogs, built at run time: a library serves `&'static CatalogDef`, which a build writes as a `static` ---------------------

/// A table for the life of the test process. The tests build theirs at run time, once each (a `thread_local!` builds a library once per
/// thread), so the allocation stands for the `static` a generated catalog is.
fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

fn ty(kind: u32, class: &'static str) -> &'static TypeDef {
    leak(TypeDef {
        kind,
        class,
        elem: None,
    })
}

fn list_of(elem: &'static TypeDef) -> &'static TypeDef {
    leak(TypeDef {
        kind: abi::VALUE_LIST,
        class: "",
        elem: Some(elem),
    })
}

fn member(
    name: &'static str,
    kind: u32,
    flags: u32,
    ty: &'static TypeDef,
    args: Vec<(&'static str, &'static TypeDef)>,
) -> MemberDef {
    let args: Vec<ArgDef> = args
        .into_iter()
        .map(|(name, ty)| ArgDef {
            name,
            ty,
            optional: false,
        })
        .collect();
    let kinds: Vec<u32> = args.iter().map(|arg| arg.ty.kind).collect();
    let names = args.iter().map(|arg| arg.name).collect::<Vec<_>>().join(" ");
    MemberDef {
        name,
        kind,
        flags,
        ty,
        args: args.leak(),
        arg_kinds: kinds.leak(),
        arg_names: names.leak(),
        default: DefaultDef::None,
        abi: "",
    }
}

fn class(name: &'static str, kind: u32, base: &'static str, foreign: usize, members: Vec<MemberDef>) -> ClassDef {
    ClassDef {
        name,
        kind,
        flags: 0,
        base,
        foreign_members: foreign,
        members: members.leak(),
    }
}

fn probe_class() -> ClassDef {
    let none = ty(abi::VALUE_NONE, "");
    let int = ty(abi::VALUE_INT, "");
    let text = ty(abi::VALUE_TEXT, "");
    let (p, m, s) = (abi::MEMBER_PROPERTY, abi::MEMBER_METHOD, abi::MEMBER_SIGNAL);
    class(
        "probe",
        abi::CLASS_HANDLE,
        "",
        0,
        vec![
            member("count", p, 0, int, vec![]),
            member("shade", p, 0, ty(abi::VALUE_ENUM, "shade"), vec![]),
            member("origin", p, 0, ty(abi::VALUE_RECORD, "point"), vec![]),
            member("names", p, 0, list_of(text), vec![]),
            member(
                "on_tick",
                p,
                abi::MEMBER_CALLBACK,
                ty(abi::VALUE_CLOSURE, "tick"),
                vec![],
            ),
            member("painter", p, 0, ty(abi::VALUE_OBJECT, "paint"), vec![]),
            member("last", p, abi::MEMBER_READONLY, int, vec![]),
            member("last_colour", p, abi::MEMBER_READONLY, text, vec![]),
            member("add", m, 0, int, vec![("a", int), ("b", int)]),
            member("fire", m, 0, none, vec![("x", int)]),
            member("paint", m, 0, none, vec![]),
            member("ticked", s, 0, none, vec![("x", int)]),
        ],
    )
}

fn probe_catalog() -> &'static CatalogDef {
    let none = ty(abi::VALUE_NONE, "");
    let double = ty(abi::VALUE_DOUBLE, "");
    let text = ty(abi::VALUE_TEXT, "");
    let classes = vec![
        class(
            "paint",
            abi::CLASS_INTERFACE,
            "",
            0,
            vec![
                member("colour", abi::MEMBER_METHOD, abi::MEMBER_REQUIRED, text, vec![]),
                member("size", abi::MEMBER_METHOD, abi::MEMBER_NATIVE_DEFAULT, double, vec![]),
            ],
        ),
        class(
            "point",
            abi::CLASS_RECORD,
            "",
            0,
            vec![
                member("x", abi::MEMBER_PROPERTY, 0, double, vec![]),
                member("y", abi::MEMBER_PROPERTY, 0, double, vec![]),
            ],
        ),
        probe_class(),
        ClassDef {
            name: "kept",
            kind: abi::CLASS_HANDLE,
            flags: abi::CLASS_FLAG_RETAINED,
            base: "",
            foreign_members: 0,
            members: &[],
        },
        class(
            "shade",
            abi::CLASS_ENUM,
            "",
            0,
            vec![
                member("light", abi::MEMBER_VALUE, 0, none, vec![]),
                member("dark", abi::MEMBER_VALUE, 0, none, vec![]),
            ],
        ),
    ];
    leak(CatalogDef {
        namespace: "test.probe",
        hash: 1,
        classes: classes.leak(),
    })
}

fn derived_catalog() -> &'static CatalogDef {
    let text = ty(abi::VALUE_TEXT, "");
    // The members of `probe` come first with the ids they have there; the library forwards them.
    let mut members = probe_class().members.iter().map(clone_member).collect::<Vec<_>>();
    let foreign = members.len();
    members.push(member("label", abi::MEMBER_PROPERTY, 0, text, vec![]));
    members.push(member("describe", abi::MEMBER_METHOD, 0, text, vec![]));
    let classes = vec![class("fancy", abi::CLASS_HANDLE, "test.probe.probe", foreign, members)];
    leak(CatalogDef {
        namespace: "test.derived",
        hash: 2,
        classes: classes.leak(),
    })
}

fn clone_member(source: &MemberDef) -> MemberDef {
    MemberDef {
        name: source.name,
        kind: source.kind,
        flags: source.flags,
        ty: source.ty,
        args: source.args,
        arg_kinds: source.arg_kinds,
        arg_names: source.arg_names,
        default: source.default,
        abi: source.abi,
    }
}

// ---- the implementations ----------------------------------------------------------------------------------------------------

#[derive(Default)]
struct ProbeState {
    count: i64,
    shade: u32,
    origin: Vec<Val>,
    names: Vec<Val>,
    on_tick: Option<Rc<dyn Callable>>,
    painter: Option<u64>,
    last: i64,
    last_colour: String,
}

fn build_probe() -> Library {
    let catalog = probe_catalog();
    let mut builder = Builder::new(catalog);
    let _ = builder.class::<()>("kept").ctor(|_cx, _inits| Ok(()));
    let _ = builder
        .class::<ProbeState>("probe")
        .ctor(|_cx, inits| {
            let mut state = ProbeState::default();
            if let Some(count) = inits.take("count") {
                state.count = count.int()?;
            }
            Ok(state)
        })
        .get("count", |s, _| Ok(Val::Int(s.count)))
        .set("count", |s, cx, value| {
            s.count = value.int()?;
            cx.changed("count");
            Ok(())
        })
        .get("shade", |s, _| {
            Ok(Val::Enum {
                class: 0,
                value: s.shade,
            })
        })
        .set("shade", |s, _, value| {
            s.shade = value.enum_value()?;
            Ok(())
        })
        .get("origin", |s, _| {
            Ok(Val::Record {
                class: 0,
                fields: s.origin.clone(),
            })
        })
        .set("origin", |s, _, value| {
            s.origin = value.record()?.to_vec();
            Ok(())
        })
        .get("names", |s, _| Ok(Val::List(s.names.clone())))
        .set("names", |s, _, value| {
            s.names = value.list()?.to_vec();
            Ok(())
        })
        .get("on_tick", |s, _| Ok(s.on_tick.clone().map_or(Val::None, Val::Closure)))
        .set("on_tick", |s, _, value| {
            s.on_tick = Some(value.closure()?.clone());
            Ok(())
        })
        .get("painter", |s, _| {
            Ok(s.painter.map_or(Val::None, |obj| Val::Object { class: 0, obj }))
        })
        .set("painter", |s, cx, value| {
            let obj = value.object()?;
            cx.hold(obj)?;
            s.painter = Some(obj);
            Ok(())
        })
        .get("last", |s, _| Ok(Val::Int(s.last)))
        .get("last_colour", |s, _| Ok(Val::Text(s.last_colour.clone())))
        .method("add", |_, _, args| Ok(Val::Int(args[0].int()? + args[1].int()?)))
        .method("fire", |s, cx, args| {
            let x = args[0].int()?;
            if let Some(tick) = s.on_tick.clone() {
                // The closure runs from the next turn, never inside this call.
                cx.later(move |cx| {
                    let answer = tick.call(0, &[Val::Int(x)]).and_then(|value| value.int()).unwrap_or(-1);
                    let me = cx.me();
                    let _ = cx.with_state::<ProbeState, _>(me, |s| s.last = answer);
                    cx.emit("ticked", vec![Val::Int(x)]);
                });
            }
            Ok(Val::None)
        })
        .method("paint", |s, cx, _| {
            let Some(painter) = s.painter else { return Ok(Val::None) };
            let paint = cx.class_named("paint")?;
            let colour = cx.member_of(paint, abi::MEMBER_METHOD, "colour")?;
            cx.later(move |cx| {
                let Ok(shell) = cx.shell(painter) else { return };
                if shell.has(colour)
                    && let Ok(Val::Text(answer)) = shell.call(colour, &[])
                {
                    let me = cx.me();
                    let _ = cx.with_state::<ProbeState, _>(me, |s| s.last_colour = answer);
                }
            });
            Ok(Val::None)
        });
    builder.finish(hooks())
}

struct Fancy {
    label: String,
}

fn build_derived() -> Library {
    let mut builder = Builder::new(derived_catalog());
    let _ = builder
        .class::<Fancy>("fancy")
        .ctor(|_cx: &mut Cx<'_>, inits| {
            Ok(Fancy {
                label: inits
                    .take("label")
                    .map_or(Ok(String::new()), |label| label.text().map(str::to_owned))?,
            })
        })
        .get("label", |s, _| Ok(Val::Text(s.label.clone())))
        .set("label", |s, _, value| {
            s.label = value.text()?.to_owned();
            Ok(())
        })
        .method("describe", |s, _, _| Ok(Val::Text(format!("fancy {}", s.label))));
    builder.finish(hooks())
}

// ---- the tests ------------------------------------------------------------------------------------------------------------

fn probe() -> Api {
    Api::new(&PROBE_API, CTX, last_error)
}

fn flush_probe() {
    PROBE.with(|lib| lib.flush(CTX));
}

#[test]
fn both_catalogs_are_implemented() {
    assert!(
        PROBE.with(Library::problems).is_empty(),
        "{:?}",
        PROBE.with(Library::problems)
    );
    assert!(
        DERIVED.with(Library::problems).is_empty(),
        "{:?}",
        DERIVED.with(Library::problems)
    );
}

#[test]
fn values_of_every_kind_cross_and_are_checked() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let count = api.member(class, abi::MEMBER_PROPERTY, "count").unwrap();
    let shade = api.member(class, abi::MEMBER_PROPERTY, "shade").unwrap();
    let origin = api.member(class, abi::MEMBER_PROPERTY, "origin").unwrap();
    let names = api.member(class, abi::MEMBER_PROPERTY, "names").unwrap();
    let add = api.member(class, abi::MEMBER_METHOD, "add").unwrap();
    let shade_class = api.class("shade").unwrap();
    let dark = api.member(shade_class, abi::MEMBER_VALUE, "dark").unwrap();

    let obj = api.create(class, &[(count, Val::Double(3.0))]).unwrap();
    assert_eq!(
        api.get(obj, count).unwrap().int().unwrap(),
        3,
        "a whole number is accepted for an integer"
    );
    assert_eq!(
        api.call(obj, add, &[Val::Int(2), Val::Uint(5)]).unwrap().int().unwrap(),
        7
    );
    api.set(
        obj,
        shade,
        &Val::Enum {
            class: shade_class,
            value: dark,
        },
    )
    .unwrap();
    assert_eq!(api.get(obj, shade).unwrap().enum_value().unwrap(), dark);
    assert_eq!(
        api.set(obj, shade, &Val::Enum { class: 0, value: 9 }).unwrap_err().code,
        abi::E_TYPE,
        "not a value of the enum"
    );
    api.set(
        obj,
        origin,
        &Val::Record {
            class: 0,
            fields: vec![Val::Int(1), Val::Double(2.5)],
        },
    )
    .unwrap();
    let point = api.get(obj, origin).unwrap();
    assert_eq!(point.record().unwrap()[1].double().unwrap(), 2.5);
    api.set(obj, names, &Val::List(vec![Val::from("a"), Val::from("b")]))
        .unwrap();
    assert_eq!(api.get(obj, names).unwrap().list().unwrap().len(), 2);
    assert_eq!(
        api.set(obj, names, &Val::List(vec![Val::Bool(true)])).unwrap_err().code,
        abi::E_TYPE,
        "a list of the wrong element"
    );
    assert_eq!(
        api.call(obj, add, &[Val::Int(1)]).unwrap_err().code,
        abi::E_TYPE,
        "a missing argument"
    );
    assert_eq!(
        api.call(obj, add, &[Val::from("x"), Val::Int(1)]).unwrap_err().code,
        abi::E_TYPE
    );
    assert_eq!(
        api.create(api.class("point").unwrap(), &[]).unwrap_err().code,
        abi::E_TYPE,
        "a record is a value"
    );
    assert_eq!(
        api.create(api.class("paint").unwrap(), &[]).unwrap_err().code,
        abi::E_TYPE,
        "an interface is implemented"
    );
    api.destroy(obj).unwrap();
    assert_eq!(api.get(obj, count).unwrap_err().code, abi::E_STALE);
}

#[test]
fn a_closure_property_runs_from_the_turn_and_a_refused_closure_is_not_adopted() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let on_tick = api.member(class, abi::MEMBER_PROPERTY, "on_tick").unwrap();
    let count = api.member(class, abi::MEMBER_PROPERTY, "count").unwrap();
    let last = api.member(class, abi::MEMBER_PROPERTY, "last").unwrap();
    let fire = api.member(class, abi::MEMBER_METHOD, "fire").unwrap();
    let ticked = api.member(class, abi::MEMBER_SIGNAL, "ticked").unwrap();
    let obj = api.create(class, &[]).unwrap();

    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    api.set(
        obj,
        on_tick,
        &Val::Closure(closure(move |_, args| {
            log.borrow_mut().push(args[0].int().unwrap());
            Ok(Val::Int(args[0].int().unwrap() * 10))
        })),
    )
    .unwrap();
    let told = Rc::new(Cell::new(0));
    let counter = Rc::clone(&told);
    api.listen(obj, ticked, move |_, args| {
        counter.set(counter.get() + args[0].int().unwrap());
        Ok(Val::None)
    })
    .unwrap();
    api.call(obj, fire, &[Val::Int(4)]).unwrap();
    assert!(seen.borrow().is_empty(), "the closure never runs inside the call");
    flush_probe();
    assert_eq!(*seen.borrow(), [4]);
    assert_eq!(api.get(obj, last).unwrap().int().unwrap(), 40, "the answer came back");
    assert_eq!(told.get(), 4, "the signal queued by the task is told in the same turn");

    // A closure the call refuses stays the caller's: it is dropped by its owner, not by the library.
    struct Flag(Rc<Cell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let freed = Rc::new(Cell::new(false));
    let flag = Flag(Rc::clone(&freed));
    let refused = closure(move |_, _| {
        let _ = &flag;
        Ok(Val::None)
    });
    assert!(api.set(obj, count, &Val::Closure(refused)).is_err());
    assert!(freed.get());
}

#[test]
fn an_interface_the_shell_implements_is_called_from_the_turn_and_its_required_methods_are_checked() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let painter = api.member(class, abi::MEMBER_PROPERTY, "painter").unwrap();
    let paint = api.member(class, abi::MEMBER_METHOD, "paint").unwrap();
    let last_colour = api.member(class, abi::MEMBER_PROPERTY, "last_colour").unwrap();
    let paint_class = api.class("paint").unwrap();
    let colour = api.member(paint_class, abi::MEMBER_METHOD, "colour").unwrap();
    let size = api.member(paint_class, abi::MEMBER_METHOD, "size").unwrap();
    let obj = api.create(class, &[]).unwrap();

    let refusal = api.implement(paint_class, &[size], |_, _| Ok(Val::None)).unwrap_err();
    assert_eq!(refusal.code, abi::E_TYPE, "{refusal}: `colour` is required");
    let shell = api
        .implement(paint_class, &[colour], |member, _| {
            assert_eq!(member, 1, "the method asked for is `colour`");
            Ok(Val::from("teal"))
        })
        .unwrap();
    api.set(
        obj,
        painter,
        &Val::Object {
            class: paint_class,
            obj: shell,
        },
    )
    .unwrap();
    api.call(obj, paint, &[]).unwrap();
    assert_eq!(api.get(obj, last_colour).unwrap().text().unwrap(), "");
    flush_probe();
    assert_eq!(api.get(obj, last_colour).unwrap().text().unwrap(), "teal");
}

#[test]
fn property_changes_of_one_turn_are_told_once_with_the_last_value() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let count = api.member(class, abi::MEMBER_PROPERTY, "count").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    let reg = api
        .listen(obj, count, move |_, args| {
            log.borrow_mut().push(args[0].int().unwrap());
            Ok(Val::None)
        })
        .unwrap();
    for value in [1, 2, 3] {
        api.set(obj, count, &Val::Int(value)).unwrap();
    }
    flush_probe();
    assert_eq!(*heard.borrow(), [3]);
    api.unlisten(reg).unwrap();
    api.set(obj, count, &Val::Int(4)).unwrap();
    flush_probe();
    assert_eq!(*heard.borrow(), [3], "an ended listener hears nothing");
    assert_eq!(api.unlisten(reg).unwrap_err().code, abi::E_STALE);
}

#[test]
fn a_class_whose_base_another_library_serves_forwards_the_base_members() {
    let derived = Api::new(&DERIVED_API, CTX, last_error);
    let base = probe();
    let fancy = derived.class("fancy").unwrap();
    let probe_class = base.class("probe").unwrap();
    // A base member has the id it has in the base class.
    let count = derived.member(fancy, abi::MEMBER_PROPERTY, "count").unwrap();
    assert_eq!(count, base.member(probe_class, abi::MEMBER_PROPERTY, "count").unwrap());
    let add = derived.member(fancy, abi::MEMBER_METHOD, "add").unwrap();
    let label = derived.member(fancy, abi::MEMBER_PROPERTY, "label").unwrap();
    let describe = derived.member(fancy, abi::MEMBER_METHOD, "describe").unwrap();

    let before = PROBE.with(|lib| lib.live_objects(CTX));
    let obj = derived
        .create(fancy, &[(count, Val::Int(5)), (label, Val::from("x"))])
        .unwrap();
    assert_eq!(
        PROBE.with(|lib| lib.live_objects(CTX)),
        before + 1,
        "constructing the derived object made its base object"
    );
    assert_eq!(
        derived.get(obj, count).unwrap().int().unwrap(),
        5,
        "a base property is read from the base object"
    );
    derived.set(obj, count, &Val::Int(9)).unwrap();
    assert_eq!(
        derived
            .call(obj, add, &[Val::Int(1), Val::Int(2)])
            .unwrap()
            .int()
            .unwrap(),
        3
    );
    assert_eq!(derived.call(obj, describe, &[]).unwrap().text().unwrap(), "fancy x");
    assert_eq!(derived.class_of(obj).unwrap(), fancy);

    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    let reg = derived
        .listen(obj, count, move |_, args| {
            log.borrow_mut().push(args[0].int().unwrap());
            Ok(Val::None)
        })
        .unwrap();
    derived.set(obj, count, &Val::Int(11)).unwrap();
    flush_probe();
    assert_eq!(
        *heard.borrow(),
        [11],
        "the base library tells a listener on a base member"
    );
    derived.unlisten(reg).unwrap();

    derived.destroy(obj).unwrap();
    assert_eq!(
        PROBE.with(|lib| lib.live_objects(CTX)),
        before,
        "ending the derived object releases its base object"
    );
}

#[test]
fn a_tracker_is_told_every_property_read_until_it_is_dropped() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let count = api.member(class, abi::MEMBER_PROPERTY, "count").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let reads = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&reads);
    let tracking = api
        .track(move |obj, class, member| log.borrow_mut().push((obj, class, member)))
        .unwrap();
    api.get(obj, count).unwrap();
    api.set(obj, count, &Val::Int(1)).unwrap();
    api.get(obj, count).unwrap();
    assert_eq!(
        *reads.borrow(),
        [(obj, class, count), (obj, class, count)],
        "reads are told, writes are not"
    );
    drop(tracking);
    api.get(obj, count).unwrap();
    assert_eq!(reads.borrow().len(), 2, "a dropped guard clears the tracker");
}

#[test]
fn a_turn_of_the_library_announces_through_with_cx() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let ticked = api.member(class, abi::MEMBER_SIGNAL, "ticked").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    api.listen(obj, ticked, move |_, args| {
        log.borrow_mut().push(args[0].int().unwrap());
        Ok(Val::None)
    })
    .unwrap();
    // Outside any call of the object's own: what a library's turn does when the compositor said something.
    PROBE
        .with(|lib| lib.with_cx(CTX, obj, |cx| cx.emit("ticked", vec![Val::Int(5)])))
        .unwrap();
    assert!(heard.borrow().is_empty(), "told from the next turn, not by the call");
    flush_probe();
    assert_eq!(*heard.borrow(), [5]);
    api.destroy(obj).unwrap();
    assert!(
        PROBE.with(|lib| lib.with_cx(CTX, obj, |_| ())).is_err(),
        "an object that is gone has no context"
    );
}

#[test]
fn a_retained_object_ends_once_it_is_unpinned_and_nobody_holds_it() {
    let api = probe();
    let class = api.class("kept").unwrap();
    let unpin = |obj| PROBE.with(|lib| lib.with_cx(CTX, obj, |cx| cx.unpin(obj))).unwrap();
    // Let go of, it stays: it is retained.
    let alone = api.create(class, &[]).unwrap();
    api.release(alone).unwrap();
    assert!(api.class_of(alone).is_ok());
    // Unpinned, nobody holds it: it ends at once.
    unpin(alone);
    assert!(api.class_of(alone).is_err());
    // Unpinned while the shell still holds it: it ends when the shell lets go.
    let held = api.create(class, &[]).unwrap();
    unpin(held);
    assert!(api.class_of(held).is_ok());
    api.release(held).unwrap();
    assert!(api.class_of(held).is_err());
}
