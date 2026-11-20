//! `gnoblin`: the service that holds the roots of the model, `find`, the timers and the focus history.
//!
//! Its signals are a facade: `client_title_changed` is `clients.title_changed`, and a listener of it stands in the collection's own list,
//! in the order listeners were added.

use std::rc::Rc;

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::collection::query_of;
use super::values::fields_of;
use super::world::{Kind, items, matches};
use super::{handout, operation, with_world};
use crate::session;

/// The service.
pub(super) struct Service;

/// The history's state: nothing, the order is the model's.
pub(super) struct History;

/// The signals of a window, as the collection and the facade name them.
const CLIENT_SIGNALS: &[&str] = &[
    "title_changed",
    "app_id_changed",
    "decoration_changed",
    "pointer_enter",
    "pointer_motion",
    "pointer_leave",
    "pointer_press",
    "pointer_release",
    "focus_changed",
    "geometry_changed",
    "closed",
    "ready",
];

/// The signals of an output, as the collection and the facade name them.
const OUTPUT_SIGNALS: &[&str] = &[
    "applied",
    "connected",
    "disconnected",
    "mode_changed",
    "scale_changed",
    "geometry_changed",
];

/// The most windows a history returns, and how many it does when it is not told.
const HISTORY_LIMIT: i64 = 256;
const HISTORY_DEFAULT: usize = 50;

/// The root a facade signal forwards to.
fn root(cx: &Cx<'_>, clients: bool) -> Res<Obj> {
    with_world(cx, |world| if clients { world.client_root } else { world.output_root })
}

/// The windows in the order they were last focused, narrowed by `filter`.
fn history(cx: &Cx<'_>, filter: Option<&Val>) -> Res<Val> {
    let mut limit = HISTORY_DEFAULT;
    let mut workspace = false;
    let mut monitor = false;
    if let Some(filter) = filter.filter(|value| !matches!(value, Val::None)) {
        for (name, value) in fields_of(cx, "focus_filter", filter)? {
            if matches!(value, Val::None) {
                continue;
            }
            match name {
                "limit" => {
                    let given = value.int()?;
                    if !(1..=HISTORY_LIMIT).contains(&given) {
                        return Err(Fail::type_error(format!(
                            "gnoblin.focus.history: limit is an integer from 1 to {HISTORY_LIMIT}"
                        )));
                    }
                    limit = usize::try_from(given).unwrap_or(HISTORY_DEFAULT);
                }
                "workspace_id" => workspace = true,
                "monitor_id" => monitor = true,
                _ => {}
            }
        }
    }
    let (client_root, order) = with_world(cx, |world| (world.client_root, world.focus.clone()))?;
    let live = items(cx, client_root);
    let mut ordered: Vec<Obj> = order.into_iter().filter(|obj| live.contains(obj)).collect();
    let unseen: Vec<Obj> = live.iter().copied().filter(|obj| !ordered.contains(obj)).collect();
    ordered.extend(unseen);
    // A window of the core alone has no workspace and no output: a filter on either keeps none. A package that gives windows both
    // answers for them.
    if workspace || monitor {
        ordered.clear();
    }
    ordered
        .into_iter()
        .take(limit)
        .map(|obj| handout(cx, obj))
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

/// The members of the root of `kind` that match `query`, in order.
fn find(cx: &Cx<'_>, kind: Kind, query: Option<&Val>) -> Res<Val> {
    let (class, root) = match kind {
        Kind::Client => ("client_query", root(cx, true)?),
        Kind::Output => ("output_query", root(cx, false)?),
    };
    let query = match query {
        Some(value) => query_of(cx, class, value)?,
        None => super::world::Query::default(),
    };
    items(cx, root)
        .into_iter()
        .filter(|member| matches(cx, kind, &query, *member))
        .map(|member| handout(cx, member))
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

pub(super) fn register(builder: &mut Builder) {
    let mut class = builder
        .class::<Service>("gnoblin")
        .ctor(|cx, _| {
            with_world(cx, |_| ())?;
            Ok(Service)
        })
        .get("clients", |_, cx| {
            let root = root(cx, true)?;
            handout(cx, root)
        })
        .get("outputs", |_, cx| {
            let root = root(cx, false)?;
            handout(cx, root)
        })
        .get("focus", |_, cx| {
            let ctx = cx.ctx();
            let history = crate::object::with_lib(|lib| {
                let class = lib.class_id("focus_history")?;
                lib.create(ctx, class, Vec::new()).map(|obj| (class, obj))
            })?;
            Ok(Val::Object {
                class: history.0,
                obj: history.1,
            })
        })
        .method("find_clients", |_, cx, args| find(cx, Kind::Client, args.first()))
        .method("find_outputs", |_, cx, args| find(cx, Kind::Output, args.first()))
        .method("timer", |_, cx, args| {
            operation::start(cx, args[0].double()?, Rc::clone(args[1].closure()?))
        })
        .method("now", |_, _, _| {
            Ok(Val::Double(session::with(|session| session.now_ms())))
        });
    class = class
        .redirect("client_created", |cx| Ok((root(cx, true)?, "added")))
        .redirect("output_added", |cx| Ok((root(cx, false)?, "added")))
        .redirect("output_removed", |cx| Ok((root(cx, false)?, "removed")));
    for signal in CLIENT_SIGNALS {
        let facade = format!("client_{signal}");
        class = class.redirect(&facade, move |cx| Ok((root(cx, true)?, signal)));
    }
    for signal in OUTPUT_SIGNALS {
        let facade = format!("output_{signal}");
        class = class.redirect(&facade, move |cx| Ok((root(cx, false)?, signal)));
    }
    let _ = class;
    let _ = builder
        .class::<History>("focus_history")
        .ctor(|cx, _| {
            with_world(cx, |_| ())?;
            Ok(History)
        })
        .method("history", |_, cx, args| history(cx, args.first()));
}
