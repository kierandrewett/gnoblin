//! `workspace`, `workspaces` and `desks`: a workspace, the list of them, and which output shows which.
//!
//! `data/js/gnoblin-std/host/workspace.ts`, `workspaces.ts`, `show.ts` and `desks.ts` are the reference. What the TypeScript `Workspace`
//! held for itself (its `Layout` instance, its live `gaps` object, a `MutableCollection` of its clients that the library's list was
//! synced to) is the library's here: the workspace holds the object of its layout, and its clients are the library's own collection.

use std::rc::Rc;

use gnoblin_core::host::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi as sabi;

use super::clients::{collection_val, restore_snap};
use super::layouts::{self, mode_of, step_of};
use super::values::{checked_rect, gaps_of, gaps_val, member_val, rect_val, wire_of};
use super::{LIST_OBJECTS, WORKSPACE_OBJECTS, edit, look};
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode};
use crate::object::client::{client_node, optional_client_node, output_node, output_value};
use crate::object::record::{field, number_field, record, text_field};
use crate::state::{Id, NodeId, State};

/// What a `workspace` object holds: the workspace the library calls `handle`, and the object of the layout it was given.
pub(super) struct WorkspaceRef {
    ctx: GnoblinCtx,
    handle: Id,
    /// The object of the layout the shell gave, held for as long as the workspace has it.
    layout: Option<Obj>,
}

impl WorkspaceRef {
    /// The id the library gave the workspace.
    pub(super) fn id(&self) -> Id {
        self.handle
    }
}

/// The object of the workspace `handle`: the one a shell has, or a new one. Hands it out.
pub(super) fn workspace_val(cx: &mut Cx<'_>, handle: Id) -> Res<Val> {
    let obj = workspace_obj(cx, handle)?;
    cx.handout(obj)
}

/// The object of the workspace `handle` as a plain handle, for the payload of a signal.
pub(super) fn plain_workspace(cx: &mut Cx<'_>, handle: Id) -> Val {
    workspace_obj(cx, handle).map_or(Val::None, |obj| cx.object_value(obj))
}

fn workspace_obj(cx: &mut Cx<'_>, handle: Id) -> Res<Obj> {
    let ctx = cx.ctx();
    let found = WORKSPACE_OBJECTS.with(|held| held.borrow().get(&(ctx, handle)).copied());
    if let Some(obj) = found
        && cx.class_of(obj).is_ok()
    {
        return Ok(obj);
    }
    let obj = cx.spawn(
        "workspace",
        WorkspaceRef {
            ctx,
            handle,
            layout: None,
        },
    )?;
    WORKSPACE_OBJECTS.with(|held| held.borrow_mut().insert((ctx, handle), obj));
    Ok(obj)
}

/// The workspace library id of an object of the class `workspace`.
fn handle_of(cx: &Cx<'_>, value: &Val) -> Res<Id> {
    cx.with_state(value.object()?, |held: &mut WorkspaceRef| held.id())
}

/// The workspace's id as text, for the workspace `handle`.
fn id_of(st: &State, handle: Id) -> crate::error::Res<String> {
    st.workspace(handle).map(|ws| ws.id.clone())
}

// ---- the workspace --------------------------------------------------------------------------------------------------------------

/// Gives the workspace the layout `value` names: the library's layout object, or one the shell implements. The workspace holds the object
/// for as long as it has the layout, and lets the object of the layout it had go.
fn assign_layout(cx: &mut Cx<'_>, held: &mut WorkspaceRef, value: &Val) -> Res<()> {
    let obj = value.object()?;
    // The layout the workspace has now, which is not the one held when a transition that gave it was rolled back.
    let current = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
    if held.layout == Some(obj) && layouts::is_current(cx, obj, &current) {
        return Ok(());
    }
    let layout = layouts::adopt(cx, obj, Some(held.handle))?;
    cx.hold(obj)?;
    if let Err(fail) = crate::table::replace_layout(held.ctx, held.handle, layout) {
        layouts::released(cx, obj);
        cx.unhold(obj);
        return Err(Fail::from(fail));
    }
    layouts::claimed(cx, obj, held.handle);
    if let Some(old) = held.layout.replace(obj) {
        layouts::released(cx, old);
        cx.unhold(old);
    }
    cx.changed("layout");
    Ok(())
}

/// Shows the workspace `handle` as the `show_options` value `options` says (none for the defaults): the call of a workspace's `show` and
/// of the list's.
fn show(ctx: GnoblinCtx, handle: Id, options: &Val) -> Res<()> {
    let fields: &[Val] = if options.is_none() { &[] } else { options.record()? };
    let duration = number_field(fields, 0, 0.0)?;
    let progress = number_field(fields, 1, 0.0)?;
    let output = field(fields, 2)
        .map(Val::uint)
        .transpose()?
        .filter(|output| *output != 0);
    edit(ctx, |st| st.show_workspace(handle, output, duration, progress))
}

fn register_workspace(builder: &mut Builder) {
    let _ = builder
        .class::<WorkspaceRef>("workspace")
        .ctor(|cx, inits| {
            let ctx = cx.ctx();
            let id = match inits.take("id") {
                Some(value) if !value.text()?.is_empty() => value.text()?.to_owned(),
                _ => return Err(Fail::type_error("Workspace options.id must be a non-empty string")),
            };
            let name = inits
                .take("name")
                .map_or(Ok(String::new()), |value| value.text().map(str::to_owned))?;
            let gaps = inits
                .take("gaps")
                .map_or(Ok((0.0, 0.0)), |value| gaps_of(&value, "Workspace options.gaps"))?;
            // The layout is the last thing that can fail: a workspace is made only for a layout that is adopted.
            let given = match inits.take("layout") {
                Some(value) if !value.is_none() => {
                    let obj = value.object()?;
                    Some((obj, layouts::adopt(cx, obj, None)?))
                }
                _ => None,
            };
            let layout = given
                .as_ref()
                .map_or_else(State::default_layout, |(_, layout)| Rc::clone(layout));
            let handle = edit(ctx, |st| Ok(st.new_workspace(id, name, gaps, layout, false)))?;
            let mut held_layout = None;
            if let Some((obj, _)) = given {
                cx.hold(obj)?;
                layouts::claimed(cx, obj, handle);
                held_layout = Some(obj);
            }
            WORKSPACE_OBJECTS.with(|held| held.borrow_mut().insert((ctx, handle), cx.me()));
            Ok(WorkspaceRef {
                ctx,
                handle,
                layout: held_layout,
            })
        })
        .get("id", |held, _| {
            look(held.ctx, |st| id_of(st, held.handle)).map(Val::Text)
        })
        .get("name", |held, _| {
            look(held.ctx, |st| st.workspace(held.handle).map(|ws| ws.name.clone())).map(Val::Text)
        })
        .set_ref("name", |held, cx, value| {
            let name = value.text()?.to_owned();
            edit(held.ctx, |st| {
                st.workspace_mut(held.handle)?.name = name;
                Ok(())
            })?;
            cx.changed("name");
            Ok(())
        })
        .get("gaps", |held, _| {
            look(held.ctx, |st| {
                st.workspace(held.handle).map(|ws| gaps_val(ws.gap_inner, ws.gap_outer))
            })
        })
        .set_ref("gaps", |held, cx, value| {
            let (inner, outer) = gaps_of(&value, "gaps")?;
            edit(held.ctx, |st| {
                let ws = st.workspace_mut(held.handle)?;
                ws.gap_inner = inner;
                ws.gap_outer = outer;
                ws.dirty = true;
                Ok(())
            })?;
            cx.changed("gaps");
            Ok(())
        })
        .get("layout", |held, cx| {
            let layout = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
            layouts::object_of(cx, &layout, held.handle)
        })
        .set("layout", |held, cx, value| assign_layout(cx, held, &value))
        .get("clients", |held, cx| {
            let coll = look(held.ctx, |st| st.workspace(held.handle).map(|ws| ws.coll))?;
            collection_val(cx, coll)
        })
        .get("attached", |held, _| {
            look(held.ctx, |st| st.workspace(held.handle).map(|ws| ws.attached)).map(Val::Bool)
        })
        .get("active", |held, _| {
            look(held.ctx, |st| {
                st.workspace(held.handle).map(|_| st.is_shown(held.handle))
            })
            .map(Val::Bool)
        })
        .get("index", |held, _| {
            look(held.ctx, |st| {
                st.workspace(held.handle)?;
                Ok(st
                    .workspaces
                    .iter()
                    .position(|one| *one == held.handle)
                    .map_or(-1, |at| at as i64))
            })
            .map(Val::Int)
        })
        .get("output", |held, _| {
            look(held.ctx, |st| {
                st.workspace(held.handle)?;
                Ok(st.output_showing(held.handle).unwrap_or(0))
            })
            .map(Val::Uint)
        })
        .method_ref("show", |held, _, args| {
            show(held.ctx, held.handle, &args[0])?;
            Ok(Val::None)
        })
        .method_ref("invalidate", |held, _, _| {
            edit(held.ctx, |st| {
                st.workspace_mut(held.handle)?.dirty = true;
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method_ref("walk", |held, _, args| {
            let step = args[0].int()?;
            edit(held.ctx, |st| st.walk(held.handle, step))?;
            Ok(Val::None)
        })
        .method_ref("find_split", |held, _, args| {
            let (client, x, y) = (
                client_node(held.ctx, "find_split", &args[0])?,
                args[1].double()?,
                args[2].double()?,
            );
            if !x.is_finite() || !y.is_finite() {
                return Err(Fail::type_error("a drag's point must be two finite numbers"));
            }
            // The layout answers outside the library's borrow, so it may read the SDK while it does.
            let layout = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
            Ok(layout
                .find_split(client, x, y)
                .map_or(Val::None, |split| super::shell_layout::split_val(&split)))
        })
        .method_ref("set_split", |held, _, args| {
            let ratio = args[1].double()?;
            let cross = if args[2].is_none() { f64::NAN } else { args[2].double()? };
            if !ratio.is_finite() || cross.is_infinite() {
                return Err(Fail::type_error("a divider's ratio must be a finite number"));
            }
            let layout = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
            layout.set_split(args[0].uint()?, ratio, cross);
            edit(held.ctx, |st| {
                st.workspace_mut(held.handle)?.dirty = true;
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method_ref("reset_split", |held, _, args| {
            let layout = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
            layout.reset_split(args[0].uint()?);
            edit(held.ctx, |st| {
                st.workspace_mut(held.handle)?.dirty = true;
                Ok(())
            })?;
            Ok(Val::None)
        });
}

// ---- the list --------------------------------------------------------------------------------------------------------------------

/// The objects of the workspaces in the list, in order.
fn list_of(cx: &mut Cx<'_>) -> Res<Vec<Val>> {
    let handles = look(cx.ctx(), |st| Ok(st.workspaces.clone()))?;
    handles.into_iter().map(|handle| workspace_val(cx, handle)).collect()
}

/// The object of the workspace `handle`, or none.
fn maybe_workspace(cx: &mut Cx<'_>, handle: Option<Id>) -> Res<Val> {
    match handle {
        Some(handle) => workspace_val(cx, handle),
        None => Ok(Val::None),
    }
}

/// The workspace of the list that has the id `id`, if there is one.
fn workspace_named(ctx: GnoblinCtx, id: &str) -> Res<Option<Id>> {
    look(ctx, |st| {
        Ok(st
            .workspaces
            .iter()
            .copied()
            .find(|handle| st.ws.get(handle).is_some_and(|ws| ws.id == id)))
    })
}

fn register_list(builder: &mut Builder) {
    let _ = builder
        .class::<()>("workspaces")
        .ctor(|cx, _| {
            let ctx = cx.ctx();
            LIST_OBJECTS.with(|held| held.borrow_mut().insert(ctx, cx.me()));
            // The desktop starts with the list: its default workspace, and the adoption of the windows that wait. A shell that has the
            // list is a shell of the classes, whose workspaces the compositor is told of (`State::publish_active_workspace`).
            crate::events::open(ctx).map_err(Fail::from)?;
            edit(ctx, |st| {
                st.publish_active = true;
                Ok(())
            })?;
            Ok(())
        })
        .get("list", |_, cx| list_of(cx).map(Val::List))
        .get("length", |_, cx| {
            look(cx.ctx(), |st| Ok(st.workspaces.len() as u64)).map(Val::Uint)
        })
        .get("area", |_, cx| {
            let areas = look(cx.ctx(), |st| st.client_areas_raw(0))?;
            Ok(rect_option(
                (areas.flags & sabi::STD_AREAS_HAS_AREA != 0).then(|| raw_rect(&areas.area)),
            ))
        })
        .method("show", |_, cx, args| {
            let handle = handle_of(cx, &args[0])?;
            show(cx.ctx(), handle, &args[1])?;
            Ok(Val::None)
        })
        .method("show_by_id", |_, cx, args| {
            let id = args[0].text()?;
            let Some(handle) = workspace_named(cx.ctx(), id)? else {
                return Err(Fail::no_such(format!("there is no workspace \"{id}\" in the list")));
            };
            show(cx.ctx(), handle, &args[1])?;
            Ok(Val::None)
        })
        .method("add", |_, cx, args| {
            let handle = handle_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.add_workspace(handle))?;
            Ok(Val::None)
        })
        .method("remove", |_, cx, args| {
            let handle = handle_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.remove_workspace_checked(handle)).map(Val::Bool)
        })
        .method("clear", |_, cx, _| {
            edit(cx.ctx(), |st| {
                st.clear_workspaces();
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("by_id", |_, cx, args| {
            let found = workspace_named(cx.ctx(), args[0].text()?)?;
            maybe_workspace(cx, found)
        })
        .method("of_client", |_, cx, args| {
            let client = client_node(cx.ctx(), "of_client", &args[0])?;
            let found = look(cx.ctx(), |st| Ok(st.rec.get(&client).and_then(|rec| rec.workspace)))?;
            maybe_workspace(cx, found)
        })
        .method("of_output", |_, cx, args| {
            let found = shown_on_output(cx.ctx(), args[0].uint()?)?;
            maybe_workspace(cx, found)
        });
}

/// The workspace the output shows (the one its span shows, for a spanned output), if any.
fn shown_on_output(ctx: GnoblinCtx, output: NodeId) -> Res<Option<Id>> {
    look(ctx, |st| {
        Ok(st
            .desk_of_output(output)
            .map(|desk| st.desk(desk).and_then(|desk| desk.spanned_by).unwrap_or(desk))
            .and_then(|desk| st.desk(desk))
            .and_then(|desk| desk.shown))
    })
}

/// `client.workspace`: the workspace the core's `Client` `client` belongs to, or none.
pub(crate) fn client_workspace(cx: &mut Cx<'_>, client: &Val) -> Res<Val> {
    let client = client_node(cx.ctx(), "workspace", client)?;
    let found = look(cx.ctx(), |st| Ok(st.rec.get(&client).and_then(|rec| rec.workspace)))?;
    maybe_workspace(cx, found)
}

/// `client.moveToWorkspace(workspace)`: moves the window to `workspace`; false when it is there already or cannot move.
pub(crate) fn move_client(cx: &mut Cx<'_>, client: &Val, workspace: &Val) -> Res<Val> {
    let client = client_node(cx.ctx(), "moveToWorkspace", client)?;
    let workspace = handle_of(cx, workspace)?;
    edit(cx.ctx(), |st| st.move_client_to(client, workspace)).map(Val::Bool)
}

/// The node of the output the core's `Output` `output` is, or none when that output is gone: a member that reads a gone output answers
/// none and a move to it answers false, as the TypeScript library's did. Another value, or an object of another class, is still refused.
fn live_output(ctx: GnoblinCtx, member: &str, output: &Val) -> Res<Option<NodeId>> {
    match output_node(ctx, member, output) {
        Ok(node) => Ok(Some(node)),
        Err(fail) if fail.code == abi::E_NO_SUCH_NODE => Ok(None),
        Err(fail) => Err(fail),
    }
}

/// `output.workspace`: the workspace the core's `Output` `output` shows, or none (also for a gone output).
pub(crate) fn output_workspace(cx: &mut Cx<'_>, output: &Val) -> Res<Val> {
    let Some(output) = live_output(cx.ctx(), "workspace", output)? else {
        return Ok(Val::None);
    };
    let found = shown_on_output(cx.ctx(), output)?;
    maybe_workspace(cx, found)
}

/// `output.show(workspace, options)`: shows `workspace` on the core's `Output` `output`, as `gnoblin.workspaces.show` does with the
/// output given.
pub(crate) fn show_on_output(cx: &mut Cx<'_>, output: &Val, workspace: &Val, options: &Val) -> Res<()> {
    let output = output_node(cx.ctx(), "show", output)?;
    let handle = handle_of(cx, workspace)?;
    let fields: &[Val] = if options.is_none() { &[] } else { options.record()? };
    let duration = number_field(fields, 0, 0.0)?;
    let progress = number_field(fields, 1, 0.0)?;
    edit(cx.ctx(), |st| {
        st.show_workspace(handle, Some(output), duration, progress)
    })
}

/// `client.output`: the core's `Output` that shows the workspace of the core's `Client` `client`, or none while no output shows it.
pub(crate) fn client_output(cx: &mut Cx<'_>, client: &Val) -> Res<Val> {
    let client = client_node(cx.ctx(), "output", client)?;
    let output = look(cx.ctx(), |st| {
        Ok(st
            .rec
            .get(&client)
            .and_then(|rec| rec.workspace)
            .and_then(|workspace| st.output_showing(workspace)))
    })?;
    let Some(output) = output else {
        return Ok(Val::None);
    };
    let value = output_value(cx.ctx(), output)?;
    cx.hand_out(&value)?;
    Ok(value)
}

/// `client.moveToOutput(output)`: moves the window to the workspace the core's `Output` `output` shows. False when the output is gone or
/// shows none, or the window is there already.
pub(crate) fn move_client_to_output(cx: &mut Cx<'_>, client: &Val, output: &Val) -> Res<Val> {
    let client = client_node(cx.ctx(), "moveToOutput", client)?;
    let Some(output) = live_output(cx.ctx(), "moveToOutput", output)? else {
        return Ok(Val::Bool(false));
    };
    let Some(workspace) = shown_on_output(cx.ctx(), output)? else {
        return Ok(Val::Bool(false));
    };
    edit(cx.ctx(), |st| st.move_client_to(client, workspace)).map(Val::Bool)
}

/// `client.frameInset`: the room the window's frame takes round its content, as the decorations told the library; 0 on each side for a
/// window without a frame.
pub(crate) fn client_frame_inset(cx: &mut Cx<'_>, client: &Val) -> Res<Val> {
    let client = client_node(cx.ctx(), "frameInset", client)?;
    let state = look(cx.ctx(), |st| {
        st.client_state_of(client)
            .ok_or_else(|| crate::error::Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")))
    })?;
    Ok(record(vec![
        Val::Double(state.inset_top),
        Val::Double(state.inset_right),
        Val::Double(state.inset_bottom),
        Val::Double(state.inset_left),
    ]))
}

/// `output.area`: what a layout tiles in on the core's `Output` `output`, the output less the room the shell's bars take; none for an
/// output the library has no rectangle of, or one that is gone.
pub(crate) fn output_area(cx: &mut Cx<'_>, output: &Val) -> Res<Val> {
    let Some(output) = live_output(cx.ctx(), "area", output)? else {
        return Ok(Val::None);
    };
    look(cx.ctx(), |st| Ok(rect_option(st.area_of(output))))
}

/// `gnoblin.show(workspace, options)`: shows `workspace` as `gnoblin.workspaces.show` does.
pub(crate) fn show_workspace(cx: &mut Cx<'_>, workspace: &Val, options: &Val) -> Res<()> {
    let handle = handle_of(cx, workspace)?;
    show(cx.ctx(), handle, options)
}

/// `gnoblin.find("Workspace", filter)`: the workspaces of the list whose fields equal every field the `workspace_query` record `query`
/// gives (`id`, `name`, `index`, `active`), in list order; every workspace for none.
pub(crate) fn find_workspaces(cx: &mut Cx<'_>, query: &Val) -> Res<Val> {
    let fields: &[Val] = if query.is_none() { &[] } else { query.record()? };
    let id = field(fields, 0)
        .map(|value| value.text().map(str::to_owned))
        .transpose()?;
    let name = field(fields, 1)
        .map(|value| value.text().map(str::to_owned))
        .transpose()?;
    let index = field(fields, 2).map(Val::int).transpose()?;
    let active = field(fields, 3).map(Val::bool).transpose()?;
    let found = look(cx.ctx(), |st| {
        let mut found = Vec::new();
        for (at, handle) in st.workspaces.iter().copied().enumerate() {
            let workspace = st.workspace(handle)?;
            let holds = id.as_ref().is_none_or(|id| &workspace.id == id)
                && name.as_ref().is_none_or(|name| &workspace.name == name)
                && index.is_none_or(|index| i64::try_from(at).is_ok_and(|at| at == index))
                && active.is_none_or(|active| st.is_shown(handle) == active);
            if holds {
                found.push(handle);
            }
        }
        Ok(found)
    })?;
    found
        .into_iter()
        .map(|handle| workspace_val(cx, handle))
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

// ---- the desks -------------------------------------------------------------------------------------------------------------------

/// The policy that shares the outputs, as the `output_policy` value says it.
fn output_policy_val(st: &State) -> Val {
    record(vec![
        member_val(st.policy.outputs),
        if st.policy.lead == 0 {
            Val::None
        } else {
            Val::Uint(st.policy.lead)
        },
    ])
}

fn placement_wire(mode: PlaceMode) -> u32 {
    match mode {
        PlaceMode::FreeSpace => sabi::STD_PLACE_SMART,
        PlaceMode::Center => sabi::STD_PLACE_CENTER,
        PlaceMode::Cascade => sabi::STD_PLACE_CASCADE,
    }
}

/// The policy that says where workspaces come from, as the `workspace_policy` value says it.
fn workspace_policy_val(st: &State) -> Val {
    let policy = &st.policy;
    record(vec![
        member_val(policy.workspaces),
        Val::Text(policy.prefix.clone()),
        gaps_val(policy.gaps.0, policy.gaps.1),
        member_val(placement_wire(policy.placement)),
        Val::Double(policy.step),
    ])
}

/// Applies the `output_policy` value.
fn apply_output_policy(ctx: GnoblinCtx, value: &Val) -> Res<()> {
    let fields = value.record()?;
    let mode = match field(fields, 0) {
        Some(mode) => wire_of(mode)?,
        None => sabi::STD_OUTPUTS_SEPARATE,
    };
    let lead = field(fields, 1).map(Val::uint).transpose()?.unwrap_or(0);
    edit(ctx, |st| {
        st.policy.outputs = mode;
        st.policy.lead = lead;
        st.apply_policies();
        Ok(())
    })
}

/// Applies the `workspace_policy` value.
fn apply_workspace_policy(ctx: GnoblinCtx, value: &Val) -> Res<()> {
    let fields = value.record()?;
    let mode = match field(fields, 0) {
        Some(mode) => wire_of(mode)?,
        None => sabi::STD_WORKSPACES_MANUAL,
    };
    let prefix = text_field(fields, 1, "")?;
    let gaps = match field(fields, 2) {
        Some(gaps) => gaps_of(gaps, "gaps")?,
        None => (0.0, 0.0),
    };
    let placement = match field(fields, 3) {
        Some(mode) => mode_of(mode)?,
        None => PlaceMode::FreeSpace,
    };
    let step = step_of(number_field(fields, 4, 0.0)?)?;
    edit(ctx, |st| {
        st.policy.workspaces = mode;
        st.policy.prefix = if prefix.is_empty() {
            "workspace-".to_owned()
        } else {
            prefix
        };
        st.policy.gaps = gaps;
        st.policy.placement = placement;
        st.policy.step = if step == 0.0 { DEFAULT_CASCADE_STEP } else { step };
        st.apply_policies();
        Ok(())
    })
}

fn rect_option(rect: Option<Rect>) -> Val {
    rect.as_ref().map_or(Val::None, rect_val)
}

fn raw_rect(raw: &sabi::GnoblinStdRect) -> Rect {
    Rect::new(raw.x, raw.y, raw.width, raw.height)
}

/// What the desk knows about `output`.
fn output_desk(st: &State, output: NodeId) -> crate::error::Res<Val> {
    if !st.tracks_output(output) {
        return Err(crate::error::Fail::new(
            abi::E_NO_SUCH_NODE,
            format!("node {output} is not an output"),
        ));
    }
    let info = st.output_info_of(output);
    let has_rect = info.flags & sabi::STD_OUTPUT_HAS_RECT != 0;
    let workspace = (info.workspace != 0)
        .then(|| st.ws.get(&info.workspace).map(|ws| Val::Text(ws.id.clone())))
        .flatten();
    Ok(record(vec![
        Val::Bool(info.flags & sabi::STD_OUTPUT_HAS_DESK != 0),
        Val::Bool(info.flags & sabi::STD_OUTPUT_PARKED != 0),
        workspace.unwrap_or(Val::None),
        rect_option(has_rect.then(|| raw_rect(&info.rect))),
        rect_option(has_rect.then(|| raw_rect(&info.area))),
    ]))
}

fn register_desks(builder: &mut Builder) {
    let _ = builder
        .class::<()>("desks")
        .ctor(|cx, _| crate::events::open(cx.ctx()).map_err(Fail::from))
        .get("outputs", |_, cx| look(cx.ctx(), |st| Ok(output_policy_val(st))))
        .set("outputs", |_, cx, value| apply_output_policy(cx.ctx(), &value))
        .get("workspaces", |_, cx| look(cx.ctx(), |st| Ok(workspace_policy_val(st))))
        .set("workspaces", |_, cx, value| apply_workspace_policy(cx.ctx(), &value))
        .method("output", |_, cx, args| {
            let output = args[0].uint()?;
            look(cx.ctx(), |st| output_desk(st, output))
        })
        .method("set_insets", |_, cx, args| {
            let (output, top, bottom) = (args[0].uint()?, args[1].double()?, args[2].double()?);
            if !(top.is_finite() && bottom.is_finite() && top >= 0.0 && bottom >= 0.0) {
                return Err(Fail::type_error(
                    "the room bars take must be two finite numbers of at least 0",
                ));
            }
            edit(cx.ctx(), |st| {
                if !st.tracks_output(output) {
                    return Err(crate::error::Fail::new(
                        abi::E_NO_SUCH_NODE,
                        format!("node {output} is not an output"),
                    ));
                }
                st.set_insets(output, top, bottom);
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("span", |_, cx, args| {
            let lead = args[0].uint()?;
            let members = super::values::nodes_of(&args[1])?;
            edit(cx.ctx(), |st| {
                if !st.tracks_output(lead) {
                    return Err(crate::error::Fail::new(
                        abi::E_NO_SUCH_NODE,
                        format!("node {lead} is not an output"),
                    ));
                }
                st.span_desks(lead, Some(&members));
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("areas", |_, cx, args| {
            // No client is the primary output's room: the node 0.
            let client = optional_client_node(cx.ctx(), "areas", args.first())?.unwrap_or(0);
            let areas = look(cx.ctx(), |st| st.client_areas_raw(client))?;
            Ok(record(vec![
                rect_option((areas.flags & sabi::STD_AREAS_HAS_OUTPUT != 0).then(|| raw_rect(&areas.output))),
                rect_option((areas.flags & sabi::STD_AREAS_HAS_AREA != 0).then(|| raw_rect(&areas.area))),
            ]))
        })
        .method("restore_client", |_, cx, args| {
            let workspace = handle_of(cx, &args[0])?;
            let client = client_node(cx.ctx(), "restore_client", &args[1])?;
            let arrived = args[2].bool()?;
            edit(cx.ctx(), |st| st.restore_client(workspace, client, arrived)).map(Val::Bool)
        })
        .method("restore_snap", |_, cx, args| {
            let client = client_node(cx.ctx(), "restore_snap", &args[0])?;
            let zone = wire_of(&args[1])? + 1;
            let area = checked_rect(&args[2], "the floating rectangle")?;
            restore_snap(cx.ctx(), client, zone, &rect_val(&area))?;
            Ok(Val::None)
        });
}

/// Implements the classes of the workspace, the list and the desks.
pub(super) fn register(builder: &mut Builder) {
    register_workspace(builder);
    register_list(builder);
    register_desks(builder);
}
