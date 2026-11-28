//! A library that serves the `probe` catalog, for the tests of the generated code.
//!
//! `build.rs` renders the catalog (`CATALOG`) and the typed API (`api`) from `schema/probe.toml`; this file is the library behind
//! them: the class `probe` implemented with `gnoblin-object`'s server, and the table a client reaches it through. A test calls the
//! typed API, and what it checks is the whole path: the generated tables, the protocol, and the generated wrappers.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinObjectApi};
use gnoblin_object::client::Api;
use gnoblin_object::server::{Builder, Hooks, Library, Provider, api_table};
use gnoblin_object::{Callable, Fail, Val};
// The root names the generated API's signatures spell (`crate::Res`, `crate::Listener`, `crate::Timing`), as an SDK crate has them.
pub use gnoblin_object::typed::Listener;
pub use gnoblin_object::{Res, Timing};

#[allow(dead_code, clippy::all)]
mod catalog {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/probe_catalog.rs"));
}

/// The typed API of the `probe` namespace.
#[allow(dead_code, clippy::all)]
pub mod api {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/probe_api.rs"));
}

/// The classes of the `probe` namespace as the JavaScript runtime carries them, rendered from the same catalog.
pub const OBJECTS_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/script-objects/probe.js"));

/// The declarations of [`OBJECTS_JS`].
pub const OBJECTS_DTS: &str = include_str!(concat!(env!("OUT_DIR"), "/script-objects/probe.d.ts"));

/// The classes of the `probe` namespace as the Python runtime carries them.
pub const OBJECTS_PY: &str = include_str!(concat!(env!("OUT_DIR"), "/script-objects/probe.py"));

use catalog::{CATALOG, CATALOG_HASH};

/// The generation the tests run in.
pub const CTX: u64 = 1;

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static LIB: Library = build();
}

struct ProbeLibrary;

impl Provider for ProbeLibrary {
    fn with<R>(work: impl FnOnce(&Library) -> R) -> R {
        LIB.with(work)
    }
}

static API: GnoblinObjectApi = api_table::<ProbeLibrary>(CATALOG_HASH);

/// The text of the library's last failure on this thread.
pub fn last_error() -> String {
    LAST.with(|last| last.borrow().clone())
}

/// The typed API over the library, for generation `CTX`.
pub fn objects() -> api::Objects {
    api::Objects::new(Api::new(&API, CTX, last_error))
}

/// The library's table, which a script runtime serves the namespace from.
pub fn table() -> &'static GnoblinObjectApi {
    &API
}

/// Tells the listeners what the library queued: what the host's turn does.
pub fn turn() {
    turn_for(CTX);
}

/// [`turn`] for the generation `ctx`, for a host whose generation is not [`CTX`].
pub fn turn_for(ctx: abi::GnoblinCtx) {
    LIB.with(|lib| lib.flush(ctx));
}

/// What the catalog and the implementation disagree about.
pub fn problems() -> Vec<String> {
    LIB.with(Library::problems)
}

/// Objects the library holds for the generation.
pub fn live_objects() -> usize {
    live_objects_for(CTX)
}

/// [`live_objects`] for the generation `ctx`, for a host whose generation is not [`CTX`].
pub fn live_objects_for(ctx: abi::GnoblinCtx) -> usize {
    LIB.with(|lib| lib.live_objects(ctx))
}

const SHADE_DARK: u32 = 2;

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

fn build() -> Library {
    let mut builder = Builder::new(&CATALOG);
    let _ = builder
        .class::<ProbeState>("probe")
        .ctor(|_cx, inits| {
            let mut state = ProbeState {
                shade: 1,
                ..ProbeState::default()
            };
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
            Ok(if s.origin.is_empty() {
                Val::None
            } else {
                Val::Record {
                    class: 0,
                    fields: s.origin.clone(),
                }
            })
        })
        .set("origin", |s, _, value| {
            s.origin = value.record()?.to_vec();
            Ok(())
        })
        .get("names", |s, _| Ok(Val::List(s.names.clone())))
        .set("names", |s, _, value| {
            let names = value.list()?;
            if names
                .iter()
                .any(|name| matches!(name, Val::Text(text) if text.is_empty()))
            {
                return Err(Fail::type_error("`names` must not hold an empty name"));
            }
            s.names = names.to_vec();
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
        .method("greet", |_, _, args| {
            let name = args[0].text()?;
            Ok(Val::Text(if args[1].bool().unwrap_or(false) {
                format!("HELLO, {}!", name.to_uppercase())
            } else {
                format!("hello, {name}")
            }))
        })
        .method("reach", |_, _, args| {
            let from = args[0].record()?;
            let (x, y) = (from[0].double()?, from[1].double()?);
            Ok(Val::Double(x.hypot(y)))
        })
        .method("length_of", |_, _, args| {
            let span = args[0].record()?;
            let closed = span.get(2).is_some_and(|closed| closed.bool().unwrap_or(false));
            Ok(Val::Double(
                span[1].double()? - span[0].double()? + if closed { 1.0 } else { 0.0 },
            ))
        })
        .method("mark", |s, _, _| {
            Ok(Val::Record {
                class: 0,
                fields: vec![
                    Val::Double(s.count as f64),
                    Val::Double(1.0),
                    Val::Text("mark".to_owned()),
                    Val::Enum {
                        class: 0,
                        value: SHADE_DARK,
                    },
                ],
            })
        })
        .method("total", |_, _, args| {
            let mut total = 0.0;
            for point in args[0].list()? {
                total += point.record()?[0].double()?;
            }
            Ok(Val::Double(total))
        })
        .method("echo_shapes", |_, _, args| Ok(args[0].clone()))
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
            let Some(painter) = s.painter else {
                return Ok(Val::None);
            };
            let paint = cx.class_named("paint")?;
            let colour = cx.member_of(paint, abi::MEMBER_METHOD, "colour")?;
            let size = cx.member_of(paint, abi::MEMBER_METHOD, "size")?;
            cx.later(move |cx| {
                let Ok(shell) = cx.shell(painter) else { return };
                let mut seen = String::new();
                if let Ok(Val::Text(answer)) = shell.call(colour, &[]) {
                    seen = answer;
                }
                // A method the shell did not implement is not called: the library keeps its own default.
                if shell.has(size)
                    && let Ok(answer) = shell.call(size, &[]).and_then(|value| value.double())
                {
                    seen = format!("{seen} {answer}");
                }
                let me = cx.me();
                let _ = cx.with_state::<ProbeState, _>(me, |s| s.last_colour = seen);
            });
            Ok(Val::None)
        })
        .function("clamp", |_, args| {
            let (value, low, high) = (args[0].int()?, args[1].int()?, args[2].int()?);
            if low > high {
                return Err(Fail::type_error("`low` is above `high`"));
            }
            Ok(Val::Int(value.clamp(low, high)))
        });
    builder.finish(Hooks {
        first_use: |_| Ok(()),
        schedule: |_| {},
        set_last: |fail| LAST.with(|last| *last.borrow_mut() = fail.message.clone()),
        log: |_, _, _| {},
        foreign: |_: abi::GnoblinCtx, _: &str| Err(Fail::no_such("the probe has no base classes")),
    })
}
