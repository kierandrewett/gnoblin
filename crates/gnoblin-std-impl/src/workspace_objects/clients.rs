//! `client_collection` and `client_state`: a live list of clients, and what the library remembers about one client.
//!
//! A collection is a name for one of the library's own (`collection.rs`): a workspace's clients, which the shell changes, the list of every
//! client, or a selection of one (`where`), which follows its parent as clients come, go and change. A client is a node number. What the
//! TypeScript library kept as properties of its `Client` (`frameInset`, `minSize`, `maxSize`, `floatingBounds`, the snap record of
//! `gnoblin-std/snap`, `moveToWorkspace`) is the library's record here (`client_state.rs`), so a shell of any language reads and writes the
//! one, and the library's refit of a window to a new work area honours it.

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Res, Val};
use gnoblin_std_abi as sabi;

use super::values::{checked_rect, client_of, client_val, member_val, rect_of, rect_val, wire_of};
use super::workspaces::{WorkspaceRef, workspace_val};
use super::{COLLECTION_OBJECTS, edit, look};
use crate::collection::{Source, Want};
use crate::object::record::{field, number_field, record};
use crate::state::{Id, NodeId};

/// What a `client_collection` holds: where its members are.
struct CollectionRef {
    ctx: GnoblinCtx,
    id: Id,
}

/// The object of the collection `id`: the one a shell has, or a new one.
pub(super) fn collection_val(cx: &mut Cx<'_>, id: Id) -> Res<Val> {
    let ctx = cx.ctx();
    let found = COLLECTION_OBJECTS.with(|held| held.borrow().get(&(ctx, id)).copied());
    if let Some(obj) = found
        && cx.class_of(obj).is_ok()
    {
        return cx.handout(obj);
    }
    let obj = cx.spawn("client_collection", CollectionRef { ctx, id })?;
    COLLECTION_OBJECTS.with(|held| held.borrow_mut().insert((ctx, id), obj));
    cx.handout(obj)
}

/// The members of a collection of clients as numbers.
fn members(held: &CollectionRef) -> Res<Vec<NodeId>> {
    look(held.ctx, |st| {
        if st.colls.contains_key(&held.id) {
            Ok(st.items(held.id))
        } else {
            Err(crate::error::Fail::stale(format!(
                "collection {} does not exist",
                held.id
            )))
        }
    })
}

/// The terms of a `client_query`, in the order of its fields, each a key and what it must equal.
fn terms_of(value: &Val) -> Res<Vec<(String, Want)>> {
    let fields = value.record()?;
    let mut terms = Vec::new();
    for (at, key, text) in [
        (0, "app_id", true),
        (1, "title", true),
        (2, "workspace", true),
        (3, "focused", false),
        (4, "fullscreen", false),
        (5, "maximised", false),
        (6, "minimised", false),
    ] {
        if let Some(wanted) = field(fields, at) {
            terms.push((
                key.to_owned(),
                if text {
                    Want::Text(wanted.text()?.to_owned())
                } else {
                    Want::Bool(wanted.bool()?)
                },
            ));
        }
    }
    Ok(terms)
}

/// The workspace a mutable collection belongs to.
fn workspace_of(held: &CollectionRef) -> Res<Id> {
    look(held.ctx, |st| {
        let coll = st
            .colls
            .get(&held.id)
            .ok_or_else(|| crate::error::Fail::stale(format!("collection {} does not exist", held.id)))?;
        match coll.source {
            Some(Source::WorkspaceClients(workspace)) => Ok(workspace),
            Some(_) => Err(crate::error::Fail::refused(
                "this collection's membership cannot be changed: it is the list of every client",
            )),
            None => Err(crate::error::Fail::refused(
                "this collection is a selection: its membership follows its parent's",
            )),
        }
    })
}

fn register_collection(builder: &mut Builder) {
    let _ = builder
        .class::<CollectionRef>("client_collection")
        .ctor(|_, _| {
            Err(Fail::refused(
                "a collection of clients is the clients of a workspace, every client, or a selection: ask for one with \
                 `workspace.clients`, `all` or `where`",
            ))
        })
        .get("length", |held, _| {
            members(held).map(|list| Val::Uint(list.len() as u64))
        })
        .get("members", |held, _| {
            members(held).map(|list| Val::List(list.into_iter().map(client_val).collect()))
        })
        .method("contains", |held, _, args| {
            let client = client_of(&args[0])?;
            members(held).map(|list| Val::Bool(list.contains(&client)))
        })
        .method("where", |held, cx, args| {
            let terms = terms_of(&args[0])?;
            let view = edit(held.ctx, |st| st.make_view(held.id, terms))?;
            collection_val(cx, view)
        })
        .method("add", |held, _, args| {
            let workspace = workspace_of(held)?;
            let client = client_of(&args[0])?;
            edit(held.ctx, |st| st.transfer_client(workspace, client))?;
            Ok(Val::None)
        })
        .method("remove", |held, _, args| {
            let workspace = workspace_of(held)?;
            let client = client_of(&args[0])?;
            edit(held.ctx, |st| Ok(st.detach_client(workspace, client))).map(Val::Bool)
        })
        .method("clear", |held, _, _| {
            let workspace = workspace_of(held)?;
            edit(held.ctx, |st| {
                st.clear_clients(workspace);
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("swap", |held, _, args| {
            let workspace = workspace_of(held)?;
            let (a, b) = (client_of(&args[0])?, client_of(&args[1])?);
            edit(held.ctx, |st| st.swap_clients(workspace, a, b))?;
            Ok(Val::None)
        })
        .replay("added", |cx| {
            let me = cx.me();
            let Ok((ctx, id)) = cx.with_state(me, |held: &mut CollectionRef| (held.ctx, held.id)) else {
                return Vec::new();
            };
            // A selection remembers which members its listeners heard of, so a member that stops matching is announced as removed.
            edit(ctx, |st| {
                st.track(id);
                Ok(st.items(id))
            })
            .map(|list| list.into_iter().map(|client| vec![client_val(client)]).collect())
            .unwrap_or_default()
        })
        .replay("removed", |cx| {
            let me = cx.me();
            if let Ok((ctx, id)) = cx.with_state(me, |held: &mut CollectionRef| (held.ctx, held.id)) {
                let _ = edit(ctx, |st| {
                    st.track(id);
                    Ok(())
                });
            }
            Vec::new()
        })
        .function("all", |cx, _| {
            let root = look(cx.ctx(), |st| Ok(st.roots[0]))?;
            collection_val(cx, root)
        });
}

// ---- client_state ---------------------------------------------------------------------------------------------------------------

/// What a `client_state` holds: the client it is about.
struct ClientRef {
    ctx: GnoblinCtx,
    node: NodeId,
}

fn no_client(node: NodeId) -> crate::error::Fail {
    crate::error::Fail::new(abi::E_NO_SUCH_NODE, format!("node {node} is not a client"))
}

/// The record the library keeps of the client.
fn state_of(held: &ClientRef) -> Res<sabi::GnoblinStdClientState> {
    look(held.ctx, |st| {
        st.client_state_of(held.node).ok_or_else(|| no_client(held.node))
    })
}

/// Changes what the library remembers of the client: reads the record, lets `change` edit it, and keeps it.
fn update(held: &ClientRef, change: impl FnOnce(&mut sabi::GnoblinStdClientState) -> Res<()>) -> Res<()> {
    let mut state = state_of(held)?;
    change(&mut state)?;
    edit(held.ctx, |st| st.set_client_state(held.node, &state))
}

fn size_val(width: f64, height: f64) -> Val {
    record(vec![Val::Double(width), Val::Double(height)])
}

/// The width and height of a `size_limit` value: finite numbers.
fn size_of(value: &Val) -> Res<(f64, f64)> {
    let fields = value.record()?;
    let width = number_field(fields, 0, f64::NAN)?;
    let height = number_field(fields, 1, f64::NAN)?;
    if !width.is_finite() || !height.is_finite() {
        return Err(Fail::type_error("a size limit is two finite numbers"));
    }
    Ok((width, height))
}

fn rect_out(rect: &Rect) -> sabi::GnoblinStdRect {
    sabi::GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn register_state(builder: &mut Builder) {
    let _ = builder
        .class::<ClientRef>("client_state")
        .ctor(|_, _| {
            Err(Fail::refused(
                "the state of a client is the one `of` gives for its node",
            ))
        })
        .get("node", |held, _| Ok(client_val(held.node)))
        .get("workspace", |held, cx| {
            let workspace = look(held.ctx, |st| {
                st.rec
                    .get(&held.node)
                    .map(|rec| rec.workspace)
                    .ok_or_else(|| no_client(held.node))
            })?;
            match workspace {
                Some(workspace) => workspace_val(cx, workspace),
                None => Ok(Val::None),
            }
        })
        .get("floating", |held, _| {
            look(held.ctx, |st| {
                st.rec
                    .get(&held.node)
                    .map(|rec| rec.floating)
                    .ok_or_else(|| no_client(held.node))
            })
            .map(Val::Bool)
        })
        .set("floating", |held, _, value| {
            let floating = value.bool()?;
            edit(held.ctx, |st| st.set_floating(held.node, floating))
        })
        .get("inset", |held, _| {
            let state = state_of(held)?;
            Ok(record(vec![
                Val::Double(state.inset_top),
                Val::Double(state.inset_right),
                Val::Double(state.inset_bottom),
                Val::Double(state.inset_left),
            ]))
        })
        .set("inset", |held, _, value| {
            let fields = value.record()?;
            let sides = [
                number_field(fields, 0, 0.0)?,
                number_field(fields, 1, 0.0)?,
                number_field(fields, 2, 0.0)?,
                number_field(fields, 3, 0.0)?,
            ];
            update(held, |state| {
                [state.inset_top, state.inset_right, state.inset_bottom, state.inset_left] = sides;
                Ok(())
            })
        })
        .get("min_size", |held, _| {
            let state = state_of(held)?;
            Ok(if state.flags & sabi::STD_CLIENT_HAS_MIN != 0 {
                size_val(state.min_width, state.min_height)
            } else {
                Val::None
            })
        })
        .set("min_size", |held, _, value| {
            let size = (!value.is_none()).then(|| size_of(&value)).transpose()?;
            update(held, |state| {
                match size {
                    Some((width, height)) => {
                        state.flags |= sabi::STD_CLIENT_HAS_MIN;
                        state.min_width = width;
                        state.min_height = height;
                    }
                    None => state.flags &= !sabi::STD_CLIENT_HAS_MIN,
                }
                Ok(())
            })
        })
        .get("max_size", |held, _| {
            let state = state_of(held)?;
            Ok(if state.flags & sabi::STD_CLIENT_HAS_MAX != 0 {
                size_val(state.max_width, state.max_height)
            } else {
                Val::None
            })
        })
        .set("max_size", |held, _, value| {
            let size = (!value.is_none()).then(|| size_of(&value)).transpose()?;
            update(held, |state| {
                match size {
                    Some((width, height)) => {
                        state.flags |= sabi::STD_CLIENT_HAS_MAX;
                        state.max_width = width;
                        state.max_height = height;
                    }
                    None => state.flags &= !sabi::STD_CLIENT_HAS_MAX,
                }
                Ok(())
            })
        })
        .get("floating_bounds", |held, _| {
            let state = state_of(held)?;
            Ok(if state.flags & sabi::STD_CLIENT_HAS_FLOATING != 0 {
                rect_val(&Rect::new(
                    state.floating.x,
                    state.floating.y,
                    state.floating.width,
                    state.floating.height,
                ))
            } else {
                Val::None
            })
        })
        .set("floating_bounds", |held, _, value| {
            let bounds = (!value.is_none())
                .then(|| checked_rect(&value, "floating_bounds"))
                .transpose()?;
            update(held, |state| {
                match bounds {
                    Some(bounds) => {
                        state.flags |= sabi::STD_CLIENT_HAS_FLOATING;
                        state.floating = rect_out(&bounds);
                    }
                    None => state.flags &= !sabi::STD_CLIENT_HAS_FLOATING,
                }
                Ok(())
            })
        })
        .get("snap", |held, _| {
            look(held.ctx, |st| {
                st.rec
                    .get(&held.node)
                    .map(|rec| rec.snap)
                    .ok_or_else(|| no_client(held.node))
            })
            .map(|snap| {
                snap.map_or(Val::None, |snap| {
                    record(vec![member_val(snap.zone.saturating_sub(1)), rect_val(&snap.floating)])
                })
            })
        })
        .method("move_to_workspace", |held, cx, args| {
            let workspace = cx.with_state(args[0].object()?, |target: &mut WorkspaceRef| target.id())?;
            edit(held.ctx, |st| st.move_client_to(held.node, workspace)).map(Val::Bool)
        })
        .method("snap_to", |held, _, args| {
            let zone = wire_of(&args[0])? + 1;
            let area = checked_rect(&args[1], "the area")?;
            let edge_tiling = if args[2].is_none() { true } else { args[2].bool()? };
            edit(held.ctx, |st| {
                if !st.rec.contains_key(&held.node) || !crate::nodes::alive(&st.g, held.node) {
                    return Err(no_client(held.node));
                }
                Ok(st.snap_client(held.node, zone, &area, edge_tiling))
            })
            .map(Val::Bool)
        })
        .method("unsnap", |held, _, _| {
            edit(held.ctx, |st| st.unsnap_known(held.node, false)).map(Val::Bool)
        })
        .method("forget_snap", |held, _, _| {
            edit(held.ctx, |st| st.unsnap_known(held.node, true)).map(Val::Bool)
        })
        .function("of", |cx, args| {
            let node = client_of(&args[0])?;
            let ctx = cx.ctx();
            look(ctx, |st| {
                st.rec.contains_key(&node).then_some(()).ok_or_else(|| no_client(node))
            })?;
            let obj = cx.spawn("client_state", ClientRef { ctx, node })?;
            cx.handout(obj)
        });
}

/// Implements the client classes.
pub(super) fn register(builder: &mut Builder) {
    register_collection(builder);
    register_state(builder);
}

/// What `restore_snap` of the desks says to the library: where a client was snapped and floated before.
pub(super) fn restore_snap(ctx: GnoblinCtx, client: NodeId, zone: u32, floating: &Val) -> Res<()> {
    let floating = rect_of(floating)?;
    edit(ctx, |st| {
        let rec = st.rec.get_mut(&client).ok_or_else(|| no_client(client))?;
        rec.snap = Some(crate::client_state::Snap { zone, floating });
        Ok(())
    })
}
