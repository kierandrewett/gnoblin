#![allow(unsafe_code)]
//! The object protocol end to end, in this process, over two small libraries with hand-written catalogs: values of every kind, a
//! closure property, an interface the shell implements, signals and property changes told from the turn, a class whose base class
//! another library serves, and the property-read tracker. The libraries are `gnoblin-object`'s own server behind its own C table.
//!
//! The string code of a failed call (`GnoblinObjectApi::last_error_code`, appended to the table within ABI 1.0) is held to these failure
//! modes, each of which has the test named beside it:
//!
//! 1. A table shorter than the field (a library built before it) reports no code, and the client never reads past the table:
//!    `a_table_shorter_than_the_field_reports_no_code_and_is_never_read_past`.
//! 2. A table that has the field, and a failure that has no code (a bad argument): the accessor says zero and leaves the buffer alone:
//!    `a_failure_with_no_code_reports_nothing`.
//! 3. A code from an earlier failure on the same thread must not read as the code of a later call:
//!    `a_success_clears_the_code_of_the_failure_before_it`.
//! 4. A code text the client does not know, is too long or is not UTF-8 falls back to the code the integer says, and never panics:
//!    `a_garbage_code_falls_back_to_the_code_the_integer_says`.
//! 5. A buffer too small for the code is truncated and NUL terminated, and the full length returned, as `gnoblin_last_error` does:
//!    `a_small_buffer_gets_a_truncated_nul_terminated_code_and_the_full_length`.
//! 6. The error text is the same whether or not a code is present: `the_error_text_is_the_same_with_and_without_a_code`.
//! 7. A coded failure reads as its code: `a_refused_write_and_a_gone_object_carry_their_codes`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::LazyLock;

use gnoblin_core_abi::{self as abi, GnoblinObjectApi};
use gnoblin_object::catalog::{ArgDef, CatalogDef, ClassDef, DefaultDef, MemberDef, TypeDef};
use gnoblin_object::client::{Api, Object, closure};
use gnoblin_object::server::{Builder, Cx, Hooks, Library, Provider, api_table};
use gnoblin_object::wire::{Arena, from_raw, to_raw};
use gnoblin_object::{Callable, ErrorCode, Fail, Val};

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
            member("find", m, abi::MEMBER_OPTIONAL, ty(abi::VALUE_OBJECT, "kept"), vec![]),
            member("lost", m, 0, ty(abi::VALUE_OBJECT, "kept"), vec![]),
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
        relay_class("relay"),
        relay_class("vault"),
        lens_class(),
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
    // A signal of the derived library whose field is an object of the base library.
    members.push(member(
        "poked",
        abi::MEMBER_SIGNAL,
        0,
        ty(abi::VALUE_NONE, ""),
        vec![("what", ty(abi::VALUE_OBJECT, "test.probe.probe"))],
    ));
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
        .method("find", |_, _, _| Ok(Val::None))
        .method("lost", |_, _, _| Ok(Val::None))
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
    register_relays(&mut builder);
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

#[test]
fn an_object_value_that_names_no_class_is_adopted_as_its_most_derived_class() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let held = Object::from_val(api, &Val::Object { class: 0, obj }).unwrap();
    assert_eq!(
        held.class(),
        class,
        "a sender that names no class leaves the class to the library that serves it"
    );
    drop(held);
    assert!(api.class_of(obj).is_err(), "the adopted reference was the only one");
}

#[test]
fn two_holds_of_one_object_are_equal_and_hash_alike() {
    use std::collections::HashSet;
    let api = probe();
    let class = api.class("probe").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let other = Object::create(api, class, &[]).unwrap();
    api.retain(obj).unwrap();
    // Two references to one object, as two deliveries of it give: one object to the shell.
    let first = Object::adopt(api, class, obj);
    let second = Object::from_val(api, &Val::Object { class: 0, obj }).unwrap();
    assert_eq!(first, second);
    assert_ne!(first, other);
    let set: HashSet<&Object> = [&first, &second, &other].into_iter().collect();
    assert_eq!(set.len(), 2);
}

#[test]
fn a_typed_api_reads_an_object_of_another_library_only_as_that_librarys_foreign_value() {
    use gnoblin_object::typed::foreign_in;
    let api = probe();
    let class = api.class("probe").unwrap();
    let obj = api.create(class, &[]).unwrap();
    // A value another library handed out, naming this one: read in this library's own protocol, its class asked of it.
    let value = foreign_in(
        &Val::Foreign {
            namespace: "test.probe".into(),
            obj,
        },
        "test.probe",
    )
    .unwrap();
    let held = Object::from_val(api, &value).unwrap();
    assert_eq!(held.class(), class);
    assert_eq!(held.handle(), obj);
    // Another library's object, or a plain object value, which the delivering library serves itself, is never read in this one's table.
    let other = Val::Foreign {
        namespace: "test.other".into(),
        obj,
    };
    assert_eq!(foreign_in(&other, "test.probe").unwrap_err().code, abi::E_TYPE);
    let plain = Val::Object { class, obj };
    assert_eq!(foreign_in(&plain, "test.probe").unwrap_err().code, abi::E_TYPE);
}

#[test]
fn a_foreign_object_value_crosses_the_wire_naming_the_library_that_serves_it() {
    let value = Val::Foreign {
        namespace: "test.probe".into(),
        obj: 42,
    };
    let arena = Arena::new();
    let raw = to_raw(&value, &arena);
    assert_eq!(raw.kind, abi::VALUE_FOREIGN_OBJECT);
    assert_eq!(raw.bits, 42);
    // SAFETY: `raw` points into `value` and `arena`, both alive.
    let back = unsafe { from_raw(&raw, false) }.unwrap();
    assert!(
        matches!(&back, Val::Foreign { namespace, obj: 42 } if namespace == "test.probe"),
        "read back as {back:?}"
    );
}

#[test]
fn a_foreign_object_handed_out_takes_a_reference_in_the_library_that_serves_it() {
    let base = probe();
    let obj = base.create(base.class("probe").unwrap(), &[]).unwrap();
    let derived = Api::new(&DERIVED_API, CTX, last_error);
    let fancy = derived.create(derived.class("fancy").unwrap(), &[]).unwrap();
    let foreign = || Val::Foreign {
        namespace: "test.probe".into(),
        obj,
    };
    // Two receivers, as two listeners of one signal: each holds its own reference, in the library that serves the object.
    DERIVED
        .with(|lib| lib.with_cx(CTX, fancy, |cx| cx.hand_out(&Val::List(vec![foreign(), foreign()]))))
        .unwrap()
        .unwrap();
    base.release(obj).unwrap();
    base.release(obj).unwrap();
    assert!(base.class_of(obj).is_ok(), "one receiver still holds it");
    base.release(obj).unwrap();
    assert!(base.class_of(obj).is_err(), "every reference was given back");
    derived.release(fancy).unwrap();
}

#[test]
fn a_hand_out_that_cannot_retain_every_foreign_object_takes_no_reference_at_all() {
    let base = probe();
    let class = base.class("probe").unwrap();
    let live = base.create(class, &[]).unwrap();
    let gone = base.create(class, &[]).unwrap();
    base.release(gone).unwrap();
    let derived = Api::new(&DERIVED_API, CTX, last_error);
    let fancy = derived.create(derived.class("fancy").unwrap(), &[]).unwrap();
    let foreign = |obj| Val::Foreign {
        namespace: "test.probe".into(),
        obj,
    };
    let handed = DERIVED
        .with(|lib| lib.with_cx(CTX, fancy, |cx| cx.hand_out_args(&[foreign(live), foreign(gone)])))
        .unwrap();
    assert!(
        handed.is_err(),
        "a receiver must not be handed a reference it was not given"
    );
    // The reference taken for `live` before `gone` failed was given back: the one from `create` is the only one left.
    base.release(live).unwrap();
    assert!(base.class_of(live).is_err());
    // An unknown library is refused the same way.
    let unknown = Val::Foreign {
        namespace: "test.nowhere".into(),
        obj: 1,
    };
    let handed = DERIVED
        .with(|lib| lib.with_cx(CTX, fancy, |cx| cx.hand_out(&unknown)))
        .unwrap();
    assert!(handed.is_err());
    derived.release(fancy).unwrap();
}

#[test]
fn each_listener_of_a_signal_gets_its_own_reference_to_a_foreign_object_or_is_not_told() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let ticked = api.member(class, abi::MEMBER_SIGNAL, "ticked").unwrap();
    let source = api.create(class, &[]).unwrap();
    let live = api.create(class, &[]).unwrap();
    let gone = api.create(class, &[]).unwrap();
    api.release(gone).unwrap();
    let foreign = |obj| Val::Foreign {
        namespace: "test.probe".into(),
        obj,
    };
    let heard = Rc::new(Cell::new(0));
    for _ in 0..3 {
        let heard = Rc::clone(&heard);
        api.listen(source, ticked, move |_, args| {
            assert!(
                matches!(&args[0], Val::Foreign { obj, .. } if *obj == live),
                "got {args:?}"
            );
            heard.set(heard.get() + 1);
            Ok(Val::None)
        })
        .unwrap();
    }
    let emit = |fields: Vec<Val>| {
        PROBE
            .with(|lib| lib.with_cx(CTX, source, |cx| cx.emit("ticked", fields)))
            .unwrap();
        flush_probe();
    };

    emit(vec![foreign(live)]);
    assert_eq!(heard.get(), 3, "every listener is told");
    // Three receivers, three references, beside the one from `create`.
    for _ in 0..3 {
        api.release(live).unwrap();
    }
    assert!(api.class_of(live).is_ok(), "the reference from `create` is left");

    // A field that cannot be retained: no listener is told, and none holds a reference it was not given.
    emit(vec![foreign(live), foreign(gone)]);
    assert_eq!(heard.get(), 3, "no listener is told");
    api.release(live).unwrap();
    assert!(api.class_of(live).is_err(), "nothing was left holding it");
    api.destroy(source).unwrap();
}

/// A derived object of generation `ctx`, a listener on its `poked` signal that counts what it hears and gives back each reference it is
/// handed, and the signal emitted with `fields` from outside a call.
fn poke(ctx: u64, fields: Vec<Val>) -> (Api, u64, Rc<Cell<u32>>) {
    let derived = Api::new(&DERIVED_API, ctx, last_error);
    let fancy_class = derived.class("fancy").unwrap();
    let fancy = derived.create(fancy_class, &[]).unwrap();
    let poked = derived.member(fancy_class, abi::MEMBER_SIGNAL, "poked").unwrap();
    let heard = Rc::new(Cell::new(0));
    let count = Rc::clone(&heard);
    derived
        .listen(fancy, poked, move |_, args| {
            if let Val::Foreign { obj, .. } = &args[0] {
                Api::new(&PROBE_API, ctx, last_error).release(*obj).unwrap();
            }
            count.set(count.get() + 1);
            Ok(Val::None)
        })
        .unwrap();
    DERIVED
        .with(|lib| lib.with_cx(ctx, fancy, |cx| cx.emit("poked", fields)))
        .unwrap();
    (derived, fancy, heard)
}

#[test]
fn a_queued_signal_keeps_a_foreign_object_alive_until_it_is_delivered() {
    let base = probe();
    let obj = base.create(base.class("probe").unwrap(), &[]).unwrap();
    let (_derived, _fancy, heard) = poke(
        CTX,
        vec![Val::Foreign {
            namespace: "test.probe".into(),
            obj,
        }],
    );
    // What made the value lets the object go before the turn: the queued signal still holds it.
    base.release(obj).unwrap();
    assert!(base.class_of(obj).is_ok(), "the queued signal holds the object");
    DERIVED.with(|lib| lib.flush(CTX));
    assert_eq!(heard.get(), 1, "the listener is told");
    assert!(
        base.class_of(obj).is_err(),
        "once the listener gave back its reference and the signal its own, nothing holds it"
    );
}

#[test]
fn a_generation_that_retires_with_a_signal_queued_gives_back_its_foreign_objects() {
    // A generation of its own: retiring one is for good, and the libraries are this thread's, which another test may run on next.
    const RETIRING: u64 = 8;
    let base = Api::new(&PROBE_API, RETIRING, last_error);
    let obj = base.create(base.class("probe").unwrap(), &[]).unwrap();
    let (_derived, _fancy, heard) = poke(
        RETIRING,
        vec![Val::Foreign {
            namespace: "test.probe".into(),
            obj,
        }],
    );
    // The derived library's generation ends before its turn; the base library's lives on.
    DERIVED.with(|lib| lib.retire(RETIRING));
    assert_eq!(heard.get(), 0, "nobody is told");
    base.release(obj).unwrap();
    assert!(
        base.class_of(obj).is_err(),
        "the reference from `create` was the last one: the queued signal gave its own back"
    );
}

#[test]
fn a_retained_object_lives_until_every_reference_is_released() {
    let api = probe();
    let obj = api.create(api.class("probe").unwrap(), &[]).unwrap();
    api.retain(obj).unwrap();
    api.release(obj).unwrap();
    assert!(api.class_of(obj).is_ok());
    api.release(obj).unwrap();
    assert!(api.class_of(obj).is_err());
    assert!(api.retain(obj).is_err(), "an object that is gone cannot be retained");
}

#[test]
fn a_table_built_before_retain_refuses_it_and_is_never_read_past() {
    unsafe extern "C" fn retain_must_not_be_called(_: u64, _: u64) -> i32 {
        panic!("`retain` was read past the end of the table");
    }
    static SHORT: LazyLock<GnoblinObjectApi> = LazyLock::new(|| {
        // SAFETY: `GnoblinObjectApi` is plain data (integers and function pointers), so a bitwise copy is a valid value.
        let mut copy: GnoblinObjectApi = unsafe { std::ptr::read(&PROBE_API) };
        copy.size = core::mem::offset_of!(GnoblinObjectApi, retain) as u32;
        copy.retain = retain_must_not_be_called;
        copy
    });
    let api = Api::new(&SHORT, CTX, last_error);
    let obj = api.create(api.class("probe").unwrap(), &[]).unwrap();
    let fail = api.retain(obj).unwrap_err();
    assert_eq!(fail.code, abi::E_UNSUPPORTED);
    api.release(obj).unwrap();
}

#[test]
fn a_method_may_answer_none_for_an_object_only_when_the_catalog_says_optional() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let find = api.member(class, abi::MEMBER_METHOD, "find").unwrap();
    let lost = api.member(class, abi::MEMBER_METHOD, "lost").unwrap();
    let obj = api.create(class, &[]).unwrap();
    assert!(
        api.call(obj, find, &[]).unwrap().is_none(),
        "an `optional` result may be none"
    );
    let fault = api.call(obj, lost, &[]).unwrap_err();
    assert!(
        fault.message.contains("probe.lost") && fault.message.contains("optional"),
        "a result that is not marked and answers none names the method: {}",
        fault.message
    );
}

// ---- the string code of a failure ---------------------------------------------------------------------------------------------

/// What the accessor of the probe's own table says right now: its length and the bytes it wrote into a buffer of `cap` bytes.
fn raw_code(cap: usize) -> (usize, Vec<u8>) {
    let mut buf = vec![0xAAu8; cap.max(1)];
    // SAFETY: `buf` is a live buffer of at least `cap` bytes.
    let len = unsafe { (PROBE_API.last_error_code)(buf.as_mut_ptr(), cap) };
    (len, buf)
}

/// A copy of the probe's table cut to `size` bytes, with `accessor` as the function of its last field. What a library built before the
/// field existed looks like to a client: its table is shorter than the field, and the bytes past `size` are never to be read.
fn table_cut_to(size: u32, accessor: unsafe extern "C" fn(*mut u8, usize) -> usize) -> GnoblinObjectApi {
    // SAFETY: `GnoblinObjectApi` is plain data (integers and function pointers), so a bitwise copy is a valid value.
    let mut copy: GnoblinObjectApi = unsafe { std::ptr::read(&PROBE_API) };
    copy.size = size;
    copy.last_error_code = accessor;
    copy
}

const FIELD_AT: u32 = core::mem::offset_of!(GnoblinObjectApi, last_error_code) as u32;

static OLD: LazyLock<GnoblinObjectApi> = LazyLock::new(|| table_cut_to(FIELD_AT, must_not_be_called));
static BOGUS: LazyLock<GnoblinObjectApi> =
    LazyLock::new(|| table_cut_to(size_of::<GnoblinObjectApi>() as u32, says_bogus));
static LONG: LazyLock<GnoblinObjectApi> =
    LazyLock::new(|| table_cut_to(size_of::<GnoblinObjectApi>() as u32, says_too_long));
static BINARY: LazyLock<GnoblinObjectApi> =
    LazyLock::new(|| table_cut_to(size_of::<GnoblinObjectApi>() as u32, says_not_utf8));

unsafe extern "C" fn must_not_be_called(_: *mut u8, _: usize) -> usize {
    // A client that reads past a short table calls this; the process ends, which fails the test.
    std::process::abort()
}

unsafe extern "C" fn says_bogus(buf: *mut u8, cap: usize) -> usize {
    let text = b"bogus_code";
    if !buf.is_null() && cap > text.len() {
        // SAFETY: `buf` has `cap` writable bytes, and the text fits with its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
            buf.add(text.len()).write(0);
        }
    }
    text.len()
}

unsafe extern "C" fn says_too_long(buf: *mut u8, cap: usize) -> usize {
    let text = [b'x'; 100];
    if !buf.is_null() && cap > 0 {
        let kept = text.len().min(cap - 1);
        // SAFETY: `buf` has `cap` writable bytes and `kept + 1 <= cap`.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf, kept);
            buf.add(kept).write(0);
        }
    }
    text.len()
}

unsafe extern "C" fn says_not_utf8(buf: *mut u8, cap: usize) -> usize {
    let text = [0xFFu8, 0xFE, 0xFD];
    if !buf.is_null() && cap > text.len() {
        // SAFETY: `buf` has `cap` writable bytes, and the text fits with its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
            buf.add(text.len()).write(0);
        }
    }
    text.len()
}

/// A call on a client over `table` that fails with the integer `abi::E_STALE`: a destroyed object read.
fn stale_read(table: &'static GnoblinObjectApi) -> Fail {
    let api = Api::new(table, CTX, last_error);
    let class = api.class("probe").unwrap();
    let count = api.member(class, abi::MEMBER_PROPERTY, "count").unwrap();
    let obj = api.create(class, &[]).unwrap();
    api.destroy(obj).unwrap();
    api.get(obj, count).unwrap_err()
}

#[test]
fn a_table_shorter_than_the_field_reports_no_code_and_is_never_read_past() {
    let fail = stale_read(&OLD);
    assert_eq!(fail.code, abi::E_STALE);
    assert_eq!(
        fail.reason(),
        Some(ErrorCode::NotFound),
        "the integer is all an old library says"
    );
    let api = Api::new(&OLD, CTX, last_error);
    let bad = api.create(api.class("point").unwrap(), &[]).unwrap_err();
    assert_eq!(
        bad.reason(),
        None,
        "a bad argument of an old library is still a bad argument"
    );
}

#[test]
fn a_failure_with_no_code_reports_nothing() {
    let api = probe();
    let fail = api.create(api.class("point").unwrap(), &[]).unwrap_err();
    assert_eq!(fail.code, abi::E_TYPE);
    assert_eq!(
        fail.reason(),
        None,
        "a bad argument has no code: a script runtime raises a TypeError"
    );
    let (len, buf) = raw_code(16);
    assert_eq!(len, 0);
    assert_eq!(buf[0], 0xAA, "the buffer is not written when there is no code");
}

#[test]
fn a_success_clears_the_code_of_the_failure_before_it() {
    let api = probe();
    let fail = stale_read(&PROBE_API);
    assert_eq!(fail.reason(), Some(ErrorCode::NotFound));
    assert_eq!(raw_code(16).0, "not_found".len());
    api.class("probe").unwrap();
    assert_eq!(
        raw_code(16).0,
        0,
        "a call that succeeded leaves no code behind for the next reader"
    );
    // And a failure with no code after one with a code does not read as the earlier one.
    stale_read(&PROBE_API);
    api.create(api.class("point").unwrap(), &[]).unwrap_err();
    assert_eq!(raw_code(16).0, 0);
}

#[test]
fn a_garbage_code_falls_back_to_the_code_the_integer_says() {
    for table in [&*BOGUS, &*LONG, &*BINARY] {
        let fail = stale_read(table);
        assert_eq!(fail.code, abi::E_STALE);
        assert_eq!(
            fail.reason(),
            Some(ErrorCode::NotFound),
            "garbage is not a code: the integer's reading stands"
        );
    }
}

#[test]
fn a_small_buffer_gets_a_truncated_nul_terminated_code_and_the_full_length() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let last = api.member(class, abi::MEMBER_PROPERTY, "last").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let refused = api.set(obj, last, &Val::Int(1)).unwrap_err();
    assert_eq!(
        (refused.code, refused.reason()),
        (abi::E_TYPE, Some(ErrorCode::ReadOnly))
    );
    let (len, buf) = raw_code(4);
    assert_eq!(len, "read_only".len(), "the full length is returned");
    assert_eq!(&buf[..4], b"rea\0", "truncated to the buffer and NUL terminated");
    let (len, buf) = raw_code(10);
    assert_eq!((len, &buf[..10]), (9, &b"read_only\0"[..]));
    // A zero capacity only asks for the length.
    let (len, buf) = raw_code(0);
    assert_eq!((len, buf[0]), (9, 0xAA));
    // SAFETY: a null buffer with zero capacity is the documented way to ask for the length.
    assert_eq!(unsafe { (PROBE_API.last_error_code)(std::ptr::null_mut(), 0) }, 9);
}

#[test]
fn the_error_text_is_the_same_with_and_without_a_code() {
    let with_code = stale_read(&PROBE_API);
    let without = stale_read(&OLD);
    assert_eq!(with_code.message, without.message);
    assert!(with_code.message.contains("gone"), "{}", with_code.message);
    let prefixed = Fail::new(abi::E_STALE, "E_STALE: the client is gone").at("Client#7.x");
    assert_eq!(
        prefixed.to_string(),
        "Client#7.x: the client is gone",
        "the symbolic ABI code of the last-error text is dropped"
    );
    let words = with_code.message.clone();
    let at = with_code.at("Probe#3.count");
    assert_eq!(at.target(), Some("Probe#3.count"));
    assert!(at.to_string().starts_with("Probe#3.count: "), "{at}");
    assert!(at.to_string().ends_with(&words), "{at}");
}

#[test]
fn a_refused_write_and_a_gone_object_carry_their_codes() {
    let api = probe();
    let class = api.class("probe").unwrap();
    let last = api.member(class, abi::MEMBER_PROPERTY, "last").unwrap();
    let obj = api.create(class, &[]).unwrap();
    let refused = api.set(obj, last, &Val::Int(1)).unwrap_err();
    assert_eq!(refused.code(), ErrorCode::ReadOnly);
    assert_eq!(stale_read(&PROBE_API).code(), ErrorCode::NotFound);
    assert_eq!(api.class("nothing").unwrap_err().code(), ErrorCode::NotFound);
}

// ---- a call that runs a closure which calls back into the object the call is about -----------------------------------------------
//
// Written before the fix, from the failure in the shell log of `gnoblin.requestTransition` (`GnoblinAugment.workspaces: the object is in
// use by a call that is running`):
//
// 1. The server holds the native state of the object a call is about (`RefCell::try_borrow_mut`) for the whole implementation call.
// 2. A method that runs the caller's closure inside the call (the std library's `request_transition` runs the shell's `work`) therefore
//    cannot let that closure read a getter of the same object: the getter finds the borrow taken and answers `E_BUSY`.
// 3. A class registered with `()` for state has nothing the borrow protects, so it must not be refused; a class with state still is, and
//    that refusal is kept (the closure is told `E_BUSY`; the call that holds the state goes on).
//
// `relay` (state `()`) and `vault` (a state) have the same members, and each calls a closure of the caller's from a method (`run`), a
// setter (`hook`, which runs the closure it is given at once and keeps it) and a getter (`echo`, which runs the kept one).

/// What `relay` keeps for its one object: a class with no state keeps it where it can, and a test runs on a thread of its own.
#[derive(Default)]
struct RelayKept {
    level: Cell<i64>,
    hook: RefCell<Option<Rc<dyn Callable>>>,
}

thread_local! {
    static RELAY_KEPT: RelayKept = RelayKept::default();
}

/// A class of the members `relay` and `vault` share.
fn relay_class(name: &'static str) -> ClassDef {
    let int = ty(abi::VALUE_INT, "");
    let (p, m) = (abi::MEMBER_PROPERTY, abi::MEMBER_METHOD);
    class(
        name,
        abi::CLASS_HANDLE,
        "",
        0,
        vec![
            member("level", p, 0, int, vec![]),
            member("hook", p, abi::MEMBER_CALLBACK, ty(abi::VALUE_CLOSURE, "tick"), vec![]),
            member("echo", p, abi::MEMBER_READONLY, int, vec![]),
            member("run", m, 0, int, vec![("task", ty(abi::VALUE_CLOSURE, "tick"))]),
        ],
    )
}

/// What `vault` holds: a level and the closure `hook` was given.
#[derive(Default)]
struct Vault {
    level: i64,
    hook: Option<Rc<dyn Callable>>,
}

/// Runs the closure kept as `hook`, if there is one, for `echo`: its answer, or zero.
fn echoed(hook: Option<Rc<dyn Callable>>) -> Result<Val, Fail> {
    hook.map_or(Ok(Val::Int(0)), |hook| hook.call(0, &[Val::Int(2)]))
}

fn register_relays(builder: &mut Builder) {
    let _ = builder
        .class::<()>("relay")
        .ctor(|_cx, _inits| Ok(()))
        .get("level", |_, _| Ok(Val::Int(RELAY_KEPT.with(|kept| kept.level.get()))))
        .set("level", |_, _, value| {
            let level = value.int()?;
            RELAY_KEPT.with(|kept| kept.level.set(level));
            Ok(())
        })
        .get("hook", |_, _| {
            Ok(RELAY_KEPT
                .with(|kept| kept.hook.borrow().clone())
                .map_or(Val::None, Val::Closure))
        })
        .set("hook", |_, _, value| {
            let hook = Rc::clone(value.closure()?);
            RELAY_KEPT.with(|kept| *kept.hook.borrow_mut() = Some(Rc::clone(&hook)));
            hook.call(0, &[Val::Int(1)]).map(drop)
        })
        .get("echo", |_, _| {
            echoed(RELAY_KEPT.with(|kept| kept.hook.borrow().clone()))
        })
        .method("run", |_, _, args| args[0].closure()?.call(0, &[Val::Int(3)]));
    let _ = builder
        .class::<Vault>("vault")
        .ctor(|_cx, _inits| Ok(Vault::default()))
        .get("level", |s, _| Ok(Val::Int(s.level)))
        .set("level", |s, _, value| {
            s.level = value.int()?;
            Ok(())
        })
        .get("hook", |s, _| Ok(s.hook.clone().map_or(Val::None, Val::Closure)))
        .set("hook", |s, _, value| {
            let hook = Rc::clone(value.closure()?);
            s.hook = Some(Rc::clone(&hook));
            hook.call(0, &[Val::Int(1)]).map(drop)
        })
        .get("echo", |s, _| echoed(s.hook.clone()))
        .method("run", |_, _, args| args[0].closure()?.call(0, &[Val::Int(3)]));
    let _ = builder
        .class::<Lens>("lens")
        .ctor(|_cx, _inits| Ok(Lens::default()))
        .get("level", |s, _| Ok(Val::Int(s.level)))
        .set("level", |s, _, value| {
            s.level = value.int()?;
            Ok(())
        })
        .get("seen", |s, _| Ok(Val::Int(s.level)))
        .get("hook", |_, _| Ok(Val::None))
        .set_ref("hook", |_, _, value| value.closure()?.call(0, &[Val::Int(1)]).map(drop))
        .method_ref("run", |_, _, args| args[0].closure()?.call(0, &[Val::Int(3)]))
        .method_ref("twice", |_, _, args| Ok(Val::Int(args[0].int()? * 2)))
        .method("bump", |s, _, _| {
            s.level += 1;
            Ok(Val::Int(s.level))
        })
        .method("stir", |s, _, args| {
            s.level += 1;
            args[0].closure()?.call(0, &[Val::Int(3)])
        });
}

/// What `lens` holds: a level that `level`, `bump` and `stir` change and the other members only read.
#[derive(Default)]
struct Lens {
    level: i64,
}

/// A class of a real state whose members that do not change it read, in the catalog, like the ones that do.
fn lens_class() -> ClassDef {
    let int = ty(abi::VALUE_INT, "");
    let task = ty(abi::VALUE_CLOSURE, "tick");
    let (p, m) = (abi::MEMBER_PROPERTY, abi::MEMBER_METHOD);
    class(
        "lens",
        abi::CLASS_HANDLE,
        "",
        0,
        vec![
            member("level", p, 0, int, vec![]),
            member("seen", p, abi::MEMBER_READONLY, int, vec![]),
            member("hook", p, abi::MEMBER_CALLBACK, task, vec![]),
            member("run", m, 0, int, vec![("task", task)]),
            member("twice", m, 0, int, vec![("n", int)]),
            member("bump", m, 0, int, vec![]),
            member("stir", m, 0, int, vec![("task", task)]),
        ],
    )
}

/// A new object of `relay` or `vault` with `level` 40, and the members the closures of these tests call.
struct Relay {
    api: Api,
    obj: u64,
    level: u32,
    hook: u32,
    echo: u32,
    run: u32,
}

fn relay_of(class: &str) -> Relay {
    let api = probe();
    let class = api.class(class).unwrap();
    let level = api.member(class, abi::MEMBER_PROPERTY, "level").unwrap();
    let obj = api.create(class, &[(level, Val::Int(40))]).unwrap();
    Relay {
        api,
        obj,
        level,
        hook: api.member(class, abi::MEMBER_PROPERTY, "hook").unwrap(),
        echo: api.member(class, abi::MEMBER_PROPERTY, "echo").unwrap(),
        run: api.member(class, abi::MEMBER_METHOD, "run").unwrap(),
    }
}

#[test]
fn a_method_of_a_class_with_no_state_runs_a_closure_that_reads_writes_and_calls_its_own_object() {
    let Relay {
        api, obj, level, run, ..
    } = relay_of("relay");
    let task = closure(move |_, args| {
        let seen = api.get(obj, level)?.int()?;
        api.set(obj, level, &Val::Int(seen + args[0].int()?))?;
        // The closure calls the same method again from inside the call that runs it.
        let doubled = api
            .call(
                obj,
                run,
                &[Val::Closure(closure(|_, args| Ok(Val::Int(args[0].int()? * 2))))],
            )?
            .int()?;
        Ok(Val::Int(seen + doubled))
    });
    let answer = api.call(obj, run, &[Val::Closure(task)]).unwrap();
    assert_eq!(
        answer.int().unwrap(),
        40 + 6,
        "the closure read the level and called `run` again"
    );
    assert_eq!(api.get(obj, level).unwrap().int().unwrap(), 43, "and wrote it");
}

#[test]
fn a_setter_of_a_class_with_no_state_runs_a_closure_that_reads_and_writes_its_own_object() {
    let Relay {
        api, obj, level, hook, ..
    } = relay_of("relay");
    let seen = Rc::new(Cell::new(0));
    let told = Rc::clone(&seen);
    let task = closure(move |_, args| {
        told.set(api.get(obj, level)?.int()?);
        api.set(obj, level, &Val::Int(100 + args[0].int()?))?;
        Ok(Val::None)
    });
    api.set(obj, hook, &Val::Closure(task)).unwrap();
    assert_eq!(seen.get(), 40, "the closure read the object the setter was running on");
    assert_eq!(api.get(obj, level).unwrap().int().unwrap(), 101, "and wrote it");
}

#[test]
fn a_getter_of_a_class_with_no_state_runs_a_closure_that_reads_its_own_object() {
    let Relay {
        api,
        obj,
        level,
        hook,
        echo,
        ..
    } = relay_of("relay");
    let task = closure(move |_, args| Ok(Val::Int(api.get(obj, level)?.int()? + args[0].int()?)));
    api.set(obj, hook, &Val::Closure(task)).unwrap();
    assert_eq!(
        api.get(obj, echo).unwrap().int().unwrap(),
        42,
        "the getter's closure read a getter of the object that is being read"
    );
}

#[test]
fn a_class_with_state_still_refuses_a_closure_that_calls_its_own_object_and_the_call_goes_on() {
    let Relay {
        api,
        obj,
        level,
        hook,
        echo,
        run,
    } = relay_of("vault");
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    // What the closure sees of its own object: a read and a write, each refused while the call that runs it holds the state.
    let task = closure(move |_, _| {
        let codes: Vec<i32> = [api.get(obj, level).map(drop), api.set(obj, level, &Val::Int(1))]
            .into_iter()
            .map(|result| result.err().map_or(0, |fail| fail.code))
            .collect();
        log.borrow_mut().push(codes);
        Ok(Val::Int(7))
    });
    assert_eq!(
        api.call(obj, run, &[Val::Closure(Rc::clone(&task))])
            .unwrap()
            .int()
            .unwrap(),
        7,
        "the call that holds the state is not harmed by the refusal"
    );
    api.set(obj, hook, &Val::Closure(Rc::clone(&task))).unwrap();
    assert_eq!(api.get(obj, echo).unwrap().int().unwrap(), 7);
    assert_eq!(
        *heard.borrow(),
        [[abi::E_BUSY, abi::E_BUSY], [abi::E_BUSY, abi::E_BUSY], [0, abi::E_BUSY]],
        "a method and a setter refuse the closure's calls on the object they hold; a getter holds a shared borrow, so its closure reads \
         the object and is refused the write"
    );
    assert_eq!(
        api.get(obj, level).unwrap().int().unwrap(),
        40,
        "the refused write changed nothing, and the object is free again when the call ends"
    );
}

// ---- a class with a real state whose members do not change it ----------------------------------------------------------------------
//
// Written before the fix, from reading `Library::with_native`: it took `try_borrow_mut` on the state for the whole call, so a member that
// runs a closure of the shell inside the call (`window_menu.activate`, `workspace.set_split`, ...) could not let the closure read a getter
// or call another member of the same object, though the member only reads its state. Since the fix, a member that does not change its
// state is registered `method_ref` or `set_ref` and holds a shared borrow, which nests; a member that changes it holds the unique one.
//
// `lens` has a real state, `run` and the setter `hook` (which do not change it, and run a closure), `twice` (which does not change it),
// `seen` (a getter), `bump` and `stir` (which change it, `stir` running a closure).

struct Lens4 {
    api: Api,
    obj: u64,
    level: u32,
    seen: u32,
    hook: u32,
    run: u32,
    twice: u32,
    bump: u32,
    stir: u32,
}

fn a_lens() -> Lens4 {
    let api = probe();
    let class = api.class("lens").unwrap();
    let level = api.member(class, abi::MEMBER_PROPERTY, "level").unwrap();
    let obj = api.create(class, &[(level, Val::Int(40))]).unwrap();
    let property = |name| api.member(class, abi::MEMBER_PROPERTY, name).unwrap();
    let method = |name| api.member(class, abi::MEMBER_METHOD, name).unwrap();
    Lens4 {
        api,
        obj,
        level,
        seen: property("seen"),
        hook: property("hook"),
        run: method("run"),
        twice: method("twice"),
        bump: method("bump"),
        stir: method("stir"),
    }
}

#[test]
fn a_ref_method_runs_a_closure_that_reads_a_getter_and_calls_another_ref_method_of_its_object() {
    let Lens4 {
        api,
        obj,
        seen,
        run,
        twice,
        ..
    } = a_lens();
    let task = closure(move |_, args| {
        let read = api.get(obj, seen)?.int()?;
        let doubled = api.call(obj, twice, &[Val::Int(args[0].int()?)])?.int()?;
        Ok(Val::Int(read + doubled))
    });
    let answer = api.call(obj, run, &[Val::Closure(task)]).unwrap();
    assert_eq!(
        answer.int().unwrap(),
        40 + 6,
        "the closure read `seen` and called `twice` on the object `run` was running on"
    );
}

#[test]
fn a_ref_setter_runs_a_closure_that_reads_a_getter_and_calls_a_ref_method_of_its_object() {
    let Lens4 {
        api,
        obj,
        seen,
        hook,
        twice,
        ..
    } = a_lens();
    let told = Rc::new(Cell::new(0));
    let log = Rc::clone(&told);
    let task = closure(move |_, args| {
        let read = api.get(obj, seen)?.int()?;
        log.set(read + api.call(obj, twice, &[Val::Int(args[0].int()?)])?.int()?);
        Ok(Val::None)
    });
    api.set(obj, hook, &Val::Closure(task)).unwrap();
    assert_eq!(
        told.get(),
        40 + 2,
        "the closure read `seen` and called `twice` from inside the setter"
    );
}

#[test]
fn a_ref_method_still_refuses_a_closure_that_calls_a_member_that_changes_the_state() {
    let Lens4 {
        api,
        obj,
        level,
        run,
        bump,
        ..
    } = a_lens();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    // `bump` and a write to `level` change the state `run` is reading, which would alias it.
    let task = closure(move |_, _| {
        let codes: Vec<i32> = [api.call(obj, bump, &[]).map(drop), api.set(obj, level, &Val::Int(1))]
            .into_iter()
            .map(|result| result.err().map_or(0, |fail| fail.code))
            .collect();
        log.borrow_mut().push(codes);
        Ok(Val::Int(7))
    });
    assert_eq!(
        api.call(obj, run, &[Val::Closure(task)]).unwrap().int().unwrap(),
        7,
        "the call that holds the shared borrow goes on"
    );
    assert_eq!(*heard.borrow(), [[abi::E_BUSY; 2]]);
    assert_eq!(
        api.get(obj, level).unwrap().int().unwrap(),
        40,
        "the refused calls changed nothing"
    );
}

#[test]
fn a_member_that_changes_the_state_refuses_a_closure_that_calls_its_object_even_to_read() {
    let Lens4 {
        api,
        obj,
        level,
        seen,
        twice,
        stir,
        ..
    } = a_lens();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&heard);
    let task = closure(move |_, _| {
        let codes: Vec<i32> = [
            api.get(obj, seen).map(drop),
            api.call(obj, twice, &[Val::Int(1)]).map(drop),
        ]
        .into_iter()
        .map(|result| result.err().map_or(0, |fail| fail.code))
        .collect();
        log.borrow_mut().push(codes);
        Ok(Val::Int(7))
    });
    assert_eq!(
        api.call(obj, stir, &[Val::Closure(task)]).unwrap().int().unwrap(),
        7,
        "the call that holds the unique borrow goes on"
    );
    assert_eq!(
        *heard.borrow(),
        [[abi::E_BUSY; 2]],
        "a getter and a `_ref` method are refused while a `&mut` member runs"
    );
    assert_eq!(
        api.get(obj, level).unwrap().int().unwrap(),
        41,
        "`stir` changed the level, and the object is free again when it ends"
    );
}
