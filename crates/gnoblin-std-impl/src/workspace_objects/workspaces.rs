//! `workspace`, `workspaces` and `desks`: a workspace, the list of them, and which output shows which.
//!
//! `data/js/gnoblin-std/host/workspace.ts`, `workspaces.ts`, `show.ts` and `desks.ts` are the reference. What the TypeScript `Workspace`
//! held for itself (its `Layout` instance, its live `gaps` object, a `MutableCollection` of its clients that the library's list was
//! synced to) is the library's here: the workspace holds the object of its layout, and its clients are the library's own collection.

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi as sabi;

use super::clients::{collection_val, restore_snap};
use super::layouts::{self, mode_of, step_of};
use super::values::{checked_rect, gaps_of, gaps_val, member_val, rect_val, wire_of};
use super::{LIST_OBJECTS, WORKSPACE_OBJECTS, edit, look};
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode};
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
    if held.layout == Some(obj) {
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
        .set("name", |held, cx, value| {
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
        .set("gaps", |held, cx, value| {
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
        .method("show", |held, _, args| {
            let fields: &[Val] = if args[0].is_none() { &[] } else { args[0].record()? };
            let duration = number_field(fields, 0, 0.0)?;
            let progress = number_field(fields, 1, 0.0)?;
            let output = field(fields, 2)
                .map(Val::uint)
                .transpose()?
                .filter(|output| *output != 0);
            edit(held.ctx, |st| {
                st.show_workspace(held.handle, output, duration, progress)
            })?;
            Ok(Val::None)
        })
        .method("invalidate", |held, _, _| {
            edit(held.ctx, |st| {
                st.workspace_mut(held.handle)?.dirty = true;
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("walk", |held, _, args| {
            let step = args[0].int()?;
            edit(held.ctx, |st| st.walk(held.handle, step))?;
            Ok(Val::None)
        })
        .method("find_split", |held, _, args| {
            let (client, x, y) = (args[0].uint()?, args[1].double()?, args[2].double()?);
            if !x.is_finite() || !y.is_finite() {
                return Err(Fail::type_error("a drag's point must be two finite numbers"));
            }
            // The layout answers outside the library's borrow, so it may read the SDK while it does.
            let layout = look(held.ctx, |st| st.workspace(held.handle).map(|ws| Rc::clone(&ws.layout)))?;
            Ok(layout
                .find_split(client, x, y)
                .map_or(Val::None, |split| super::shell_layout::split_val(&split)))
        })
        .method("set_split", |held, _, args| {
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
        .method("reset_split", |held, _, args| {
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

fn register_list(builder: &mut Builder) {
    let _ = builder
        .class::<()>("workspaces")
        .ctor(|cx, _| {
            let ctx = cx.ctx();
            LIST_OBJECTS.with(|held| held.borrow_mut().insert(ctx, cx.me()));
            // The desktop starts with the list: its default workspace, and the adoption of the windows that wait.
            crate::events::open(ctx).map_err(Fail::from)?;
            Ok(())
        })
        .get("list", |_, cx| list_of(cx).map(Val::List))
        .get("length", |_, cx| {
            look(cx.ctx(), |st| Ok(st.workspaces.len() as u64)).map(Val::Uint)
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
            let id = args[0].text()?.to_owned();
            let found = look(cx.ctx(), |st| {
                Ok(st
                    .workspaces
                    .iter()
                    .copied()
                    .find(|handle| st.ws.get(handle).is_some_and(|ws| ws.id == id)))
            })?;
            maybe_workspace(cx, found)
        })
        .method("of_client", |_, cx, args| {
            let client = args[0].uint()?;
            let found = look(cx.ctx(), |st| Ok(st.rec.get(&client).and_then(|rec| rec.workspace)))?;
            maybe_workspace(cx, found)
        })
        .method("of_output", |_, cx, args| {
            let output = args[0].uint()?;
            let found = look(cx.ctx(), |st| {
                Ok(st
                    .desk_of_output(output)
                    .map(|desk| st.desk(desk).and_then(|desk| desk.spanned_by).unwrap_or(desk))
                    .and_then(|desk| st.desk(desk))
                    .and_then(|desk| desk.shown))
            })?;
            maybe_workspace(cx, found)
        });
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
            let client = args[0].uint()?;
            let areas = look(cx.ctx(), |st| st.client_areas_raw(client))?;
            Ok(record(vec![
                rect_option((areas.flags & sabi::STD_AREAS_HAS_OUTPUT != 0).then(|| raw_rect(&areas.output))),
                rect_option((areas.flags & sabi::STD_AREAS_HAS_AREA != 0).then(|| raw_rect(&areas.area))),
            ]))
        })
        .method("restore_client", |_, cx, args| {
            let workspace = handle_of(cx, &args[0])?;
            let client = args[1].uint()?;
            let arrived = args[2].bool()?;
            edit(cx.ctx(), |st| st.restore_client(workspace, client, arrived)).map(Val::Bool)
        })
        .method("restore_snap", |_, cx, args| {
            let client = args[0].uint()?;
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
