//! `client_collection` and `output_collection`: live views of the windows and outputs.
//!
//! A root holds what the compositor made, a list holds what a shell put in it, and a selection (`where`) holds the members of another
//! collection that match a query, following them as they change. All three tell their listeners when a member joins or leaves, and
//! hear what happens to a member after the member's own listeners do.

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::values::fields_of;
use super::world::{
    ClientState, CollKind, CollState, ModelQuery, OutputNode, QueryScalar, QueryWant, items_in, matches,
};
use super::{handout, listener, with_world};

/// Reads a query record: each field that is given is a term, and a field named `any_x` holds when `x` is one of its values.
pub(super) fn query_of(cx: &Cx<'_>, class: &str, value: &Val) -> Res<ModelQuery> {
    let mut query = ModelQuery::default();
    if matches!(value, Val::None) {
        return Ok(query);
    }
    for (name, field) in fields_of(cx, class, value)? {
        if matches!(field, Val::None) {
            continue;
        }
        if let Some(key) = name.strip_prefix("any_") {
            let wanted = field
                .list()?
                .iter()
                .filter_map(QueryScalar::from_val)
                .collect::<Vec<QueryScalar>>();
            query.terms.push((key.to_owned(), QueryWant::Any(wanted)));
        } else if let Some(wanted) = QueryScalar::from_val(field) {
            query.terms.push((name.to_owned(), QueryWant::One(wanted)));
        }
    }
    Ok(query)
}

/// The id of a member of a collection of `kind`.
fn id_of(cx: &Cx<'_>, kind: CollKind, member: Obj) -> Option<String> {
    match kind {
        CollKind::Client => cx
            .with_state::<ClientState, _>(member, |state| state.node.to_string())
            .ok(),
        CollKind::Output => cx
            .with_state::<OutputNode, _>(member, |state| state.node.to_string())
            .ok(),
    }
}

/// Refuses a change of a collection a shell may not change.
fn changeable(state: &CollState, what: &str) -> Res<()> {
    if state.mutable {
        Ok(())
    } else {
        Err(Fail::new(
            abi::E_REFUSED,
            format!("Collection.{what}: this collection's membership cannot be changed"),
        ))
    }
}

/// Tells the selections of the list `me` that `member` joined or left it, from the next turn: the list's state is held by the call
/// that is changing it.
fn settle_views(cx: &Cx<'_>, me: Obj, member: Obj) {
    cx.later(move |cx| {
        let _ = with_world(cx, |world| world.recompute(cx, me, member));
    });
}

fn register_collection(builder: &mut Builder, class: &'static str, kind: CollKind) {
    let mut collection = builder
        .class::<CollState>(class)
        .ctor(move |cx, _| match kind {
            CollKind::Client => {
                let me = cx.me();
                with_world(cx, |world| world.lists.push(me))?;
                Ok(CollState::list(CollKind::Client))
            }
            CollKind::Output => Err(Fail::refused(
                "the outputs belong to the compositor: `gnoblin.outputs` is the collection",
            )),
        })
        .get("length", |state, cx| {
            Ok(Val::Uint(items_in(cx, &state.spec()).len() as u64))
        })
        .get("kind", move |_, _| Ok(Val::Text(kind.word().to_owned())))
        .get("alive", |_, _| Ok(Val::Bool(true)))
        .get("mutable", |state, _| Ok(Val::Bool(state.mutable)))
        .get("priority", |state, _| Ok(Val::Double(state.priority)))
        .method("at", move |state, cx, args| {
            let items = items_in(cx, &state.spec());
            let index = args[0].int()?;
            let at = if index < 0 { items.len() as i64 + index } else { index };
            match usize::try_from(at).ok().and_then(|at| items.get(at)) {
                Some(member) => handout(cx, *member),
                None => Err(Fail::new(
                    abi::E_NO_SUCH_NODE,
                    format!("the collection has no member at {index}"),
                )),
            }
        })
        .method("get", move |state, cx, args| {
            let id = args[0].text()?;
            let items = items_in(cx, &state.spec());
            match items
                .into_iter()
                .find(|member| id_of(cx, kind, *member).as_deref() == Some(id))
            {
                Some(member) => handout(cx, member),
                None => Err(Fail::new(
                    abi::E_NO_SUCH_NODE,
                    format!("the collection has no member `{id}`"),
                )),
            }
        })
        .method("contains", |state, cx, args| {
            let member = args[0].object()?;
            Ok(Val::Bool(items_in(cx, &state.spec()).contains(&member)))
        })
        .method("to_list", |state, cx, _| {
            let items = items_in(cx, &state.spec());
            items
                .into_iter()
                .map(|member| handout(cx, member))
                .collect::<Res<Vec<Val>>>()
                .map(Val::List)
        })
        .method("where", move |state, cx, args| {
            let query_class = if kind == CollKind::Client {
                "client_query"
            } else {
                "output_query"
            };
            let query = query_of(cx, query_class, &args[0])?;
            let priority = match args.get(1) {
                None | Some(Val::None) => 0.0,
                Some(value) => {
                    let priority = value.double()?;
                    if !priority.is_finite() {
                        return Err(Fail::type_error("where: priority must be a finite number"));
                    }
                    priority
                }
            };
            let parent = cx.me();
            let members: std::collections::HashSet<Obj> = items_in(cx, &state.spec())
                .into_iter()
                .filter(|member| matches(cx, kind, &query, *member))
                .collect();
            let mut view = CollState::view(kind, parent, query, priority);
            view.members = members;
            let class = cx.class_of(parent)?;
            let name = cx
                .class_def_of(class)
                .map(|def| def.name)
                .ok_or_else(|| Fail::err("the collection's class is not in the catalog"))?;
            let obj = cx.spawn(name, view)?;
            cx.hold(parent)?;
            with_world(cx, |world| world.tracked.push(obj))?;
            handout(cx, obj)
        })
        .method("add_listener", |state, cx, args| {
            let me = cx.me();
            let replay = listener::add(cx, me, &mut state.listeners, &args[0])?;
            if let Some(replay) = replay {
                // What is there is told from the next turn, and what the call took is not told twice.
                let calls = items_in(cx, &state.spec())
                    .into_iter()
                    .map(|member| vec![cx.object_value(member), cx.object_value(me)])
                    .collect();
                replay.tell(cx, calls);
            }
            Ok(Val::None)
        })
        .method("remove_listener", |state, cx, args| {
            listener::remove(cx, &mut state.listeners, &args[0])?;
            Ok(Val::None)
        })
        .replay("added", |cx| {
            let me = cx.me();
            super::world::items(cx, me)
                .into_iter()
                .map(|member| vec![cx.object_value(member), cx.object_value(me)])
                .collect()
        });
    if kind == CollKind::Client {
        collection = collection
            .get("focused", |state, cx| {
                let items = items_in(cx, &state.spec());
                let held = with_world(cx, |world| world.focused(cx, &items))?;
                match held {
                    Some(client) => handout(cx, client),
                    None => Ok(Val::None),
                }
            })
            .method("add", move |state, cx, args| {
                changeable(state, "add")?;
                let member = args[0].object()?;
                if !state.list.contains(&member) {
                    state.list.push(member);
                    let me = cx.me();
                    cx.emit_on(me, "added", vec![cx.object_value(member), cx.object_value(me)]);
                    settle_views(cx, me, member);
                }
                Ok(Val::None)
            })
            .method("remove", move |state, cx, args| {
                changeable(state, "remove")?;
                let member = args[0].object()?;
                let Some(at) = state.list.iter().position(|held| *held == member) else {
                    return Ok(Val::Bool(false));
                };
                state.list.remove(at);
                let me = cx.me();
                cx.emit_on(me, "removed", vec![cx.object_value(member), cx.object_value(me)]);
                settle_views(cx, me, member);
                Ok(Val::Bool(true))
            })
            .method("clear", move |state, cx, _| {
                changeable(state, "clear")?;
                let me = cx.me();
                for member in std::mem::take(&mut state.list) {
                    cx.emit_on(me, "removed", vec![cx.object_value(member), cx.object_value(me)]);
                    settle_views(cx, me, member);
                }
                Ok(Val::None)
            })
            .method("swap", move |state, cx, args| {
                changeable(state, "swap")?;
                let (a, b) = (args[0].object()?, args[1].object()?);
                let first = state.list.iter().position(|held| *held == a);
                let second = state.list.iter().position(|held| *held == b);
                let (Some(first), Some(second)) = (first, second) else {
                    return Err(Fail::type_error(
                        "Collection.swap: both members must be in the collection",
                    ));
                };
                state.list.swap(first, second);
                let _ = &cx;
                Ok(Val::None)
            });
    } else {
        collection = collection.method("add", |_, _, args| super::virtual_output::add(args));
    }
    let _ = collection;
}

pub(super) fn register(builder: &mut Builder) {
    register_collection(builder, "client_collection", CollKind::Client);
    register_collection(builder, "output_collection", CollKind::Output);
}
