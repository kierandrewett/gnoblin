//! A layout an object of the shell implements: the interface `layout` of the object protocol, reached from the layout pass.
//!
//! The library calls these methods from its pass, outside any borrow of its state and never inside a call of the shell's, so a layout may
//! call back into the library (`invalidate`, a read of the workspace) while it runs. A method the shell did not define is never called
//! across the boundary (`ShellImpl::has`): the library does what it does for a layout with no such hook. What a layout answers is checked
//! as the library checks a layout of the C table (`layout.rs`), with the same words, so a placement for a node that is not one of the
//! workspace's clients, or with a number that is not finite, refuses the whole answer before anything is written.

use std::rc::Rc;

use gnoblin_core::{LogLevel, Rect};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Cx, ShellImpl};
use gnoblin_object::{Fail, MemberId, Res, Val};
use gnoblin_std_abi::{GnoblinStdSplit, STD_AXIS_X, STD_AXIS_Y, STD_SPLIT_CROSS};

use super::values::{client_of, client_val, member_val, rect_field, rect_of, rect_val, wire_of};
use crate::error::{Fail as StdFail, Res as StdRes};
use crate::layout::{LayoutContext, Placement};
use crate::object::record::{field, flag_field, number_field, record};
use crate::state::NodeId;

/// The members of the interface `layout`, by id.
#[derive(Clone, Copy, Debug)]
pub(super) struct LayoutMemberIds {
    pub(super) compute: MemberId,
    pub(super) client_added: MemberId,
    pub(super) client_removed: MemberId,
    pub(super) area_changed: MemberId,
    pub(super) clients_swapped: MemberId,
    pub(super) revision: MemberId,
    pub(super) find_split: MemberId,
    pub(super) set_split: MemberId,
    pub(super) reset_split: MemberId,
}

impl LayoutMemberIds {
    /// The ids of the interface's methods.
    pub(super) fn of(cx: &Cx<'_>) -> Res<LayoutMemberIds> {
        let class = cx.class_named("layout")?;
        let method = |name: &str| cx.member_of(class, gnoblin_core_abi::MEMBER_METHOD, name);
        Ok(LayoutMemberIds {
            compute: method("compute")?,
            client_added: method("client_added")?,
            client_removed: method("client_removed")?,
            area_changed: method("area_changed")?,
            clients_swapped: method("clients_swapped")?,
            revision: method("revision")?,
            find_split: method("find_split")?,
            set_split: method("set_split")?,
            reset_split: method("reset_split")?,
        })
    }
}

/// A layout the shell implemented, as a workspace holds it.
pub(crate) struct ObjectLayout {
    ctx: GnoblinCtx,
    shell: Rc<ShellImpl>,
    ids: LayoutMemberIds,
}

fn failed(fail: Fail) -> StdFail {
    StdFail::from(fail)
}

impl ObjectLayout {
    pub(super) fn new(ctx: GnoblinCtx, shell: Rc<ShellImpl>, ids: LayoutMemberIds) -> ObjectLayout {
        ObjectLayout { ctx, shell, ids }
    }

    /// Whether the shell implemented `member`.
    fn has(&self, member: MemberId) -> bool {
        self.shell.has(member)
    }

    /// Calls a hook the shell implemented. A hook that fails is the log's business: the layout pass goes on, as it does for a layout of
    /// the C table.
    fn hook(&self, member: MemberId, what: &str, args: &[Val]) {
        if !self.has(member) {
            return;
        }
        if let Err(fail) = self.shell.call(member, args)
            && let Ok(g) = gnoblin_core::private::gnoblin_for(self.ctx)
        {
            g.log_at(
                LogLevel::Warn,
                &format!("gnoblin-std: a layout's {what} failed: {}", fail.message),
            );
        }
    }

    /// What the layout is told, as a `layout_context` value.
    fn context(&self, cx: &LayoutContext) -> Val {
        context_val(self.ctx, cx)
    }

    /// Where the tiled clients go: the shell's `compute`.
    pub(crate) fn compute(&self, cx: &LayoutContext) -> StdRes<Vec<Placement>> {
        let answer = self.shell.call(self.ids.compute, &[self.context(cx)]).map_err(|fail| {
            let message = format!("the layout's compute failed: {}", fail.message);
            StdFail::from(fail).with_message(message)
        })?;
        placements_of(&answer, &cx.children).map_err(failed)
    }

    /// A client joined the workspace.
    pub(crate) fn client_added(&self, cx: &LayoutContext, client: NodeId) {
        self.hook(
            self.ids.client_added,
            "client_added",
            &[self.context(cx), client_val(client)],
        );
    }

    /// A client left the workspace.
    pub(crate) fn client_removed(&self, cx: &LayoutContext, client: NodeId) {
        self.hook(
            self.ids.client_removed,
            "client_removed",
            &[self.context(cx), client_val(client)],
        );
    }

    /// The workspace's rectangle changed.
    pub(crate) fn area_changed(&self, before: &Rect, now: &Rect) {
        self.hook(
            self.ids.area_changed,
            "area_changed",
            &[rect_val(before), rect_val(now)],
        );
    }

    /// Two clients traded places in the workspace's order.
    pub(crate) fn clients_swapped(&self, a: NodeId, b: NodeId) {
        self.hook(
            self.ids.clients_swapped,
            "clients_swapped",
            &[client_val(a), client_val(b)],
        );
    }

    /// The number that changes when the layout's own options do. A layout with no `revision` never changes by itself.
    pub(crate) fn revision(&self) -> u64 {
        if !self.has(self.ids.revision) {
            return 0;
        }
        self.shell
            .call(self.ids.revision, &[])
            .and_then(|answer| answer.uint())
            .unwrap_or(0)
    }

    /// Whether the shell implemented `client_removed`, so the layout hands the keyboard on itself.
    pub(crate) fn handles_removal(&self) -> bool {
        self.has(self.ids.client_removed)
    }

    /// The divider a drag that pressed `client` at (`x`, `y`) would move.
    pub(crate) fn find_split(&self, client: NodeId, x: f64, y: f64) -> Option<GnoblinStdSplit> {
        if !self.has(self.ids.find_split) {
            return None;
        }
        let answer = self
            .shell
            .call(
                self.ids.find_split,
                &[client_val(client), Val::Double(x), Val::Double(y)],
            )
            .ok()?;
        // The answer is a list of one divider, or an empty list when the point has none.
        split_raw(answer.list().ok()?.first()?).ok()
    }

    /// Moves the dividers `find_split` returned; `cross_ratio` is NaN when the drag has no second one.
    pub(crate) fn set_split(&self, token: u64, ratio: f64, cross_ratio: f64) {
        self.hook(
            self.ids.set_split,
            "set_split",
            &[
                Val::Uint(token),
                Val::Double(ratio),
                if cross_ratio.is_nan() {
                    Val::None
                } else {
                    Val::Double(cross_ratio)
                },
            ],
        );
    }

    /// Puts the dividers `find_split` returned back as they were.
    pub(crate) fn reset_split(&self, token: u64) {
        self.hook(self.ids.reset_split, "reset_split", &[Val::Uint(token)]);
    }
}

// ---- values ---------------------------------------------------------------------------------------------------------------------

/// The text id of the workspace the library calls `id`, or empty when it has no such workspace (a layout asked outside a workspace).
fn workspace_text(ctx: GnoblinCtx, id: crate::state::Id) -> String {
    crate::state::find(ctx)
        .and_then(|std| {
            std.st
                .try_borrow()
                .ok()
                .and_then(|st| st.ws.get(&id).map(|workspace| workspace.id.clone()))
        })
        .unwrap_or_default()
}

/// What a layout is told, as a `layout_context` value.
pub(super) fn context_val(ctx: GnoblinCtx, cx: &LayoutContext) -> Val {
    record(vec![
        Val::Text(workspace_text(ctx, cx.workspace)),
        Val::List(cx.children.iter().map(|child| client_val(*child)).collect()),
        rect_val(&cx.area),
        Val::Double(cx.gap_inner),
        Val::Double(cx.gap_outer),
        cx.focused.map_or(Val::None, client_val),
    ])
}

/// What a `layout_context` value says, for a call that gives a layout a context by hand.
pub(super) fn context_of(value: &Val) -> Res<LayoutContext> {
    let fields = value.record()?;
    let children = field(fields, 1)
        .ok_or_else(|| Fail::type_error("a layout context needs its `children`"))?
        .list()?
        .iter()
        .map(client_of)
        .collect::<Res<Vec<_>>>()?;
    let area = field(fields, 2).ok_or_else(|| Fail::type_error("a layout context needs its `area`"))?;
    Ok(LayoutContext {
        workspace: 0,
        children,
        area: rect_of(area)?,
        gap_inner: number_field(fields, 3, 0.0)?,
        gap_outer: number_field(fields, 4, 0.0)?,
        focused: field(fields, 5).map(client_of).transpose()?,
        pointer: None,
    })
}

/// A placement as a `layout_placement` value.
pub(super) fn placement_val(placement: &Placement) -> Val {
    record(vec![
        client_val(placement.node),
        placement.bounds.as_ref().map_or(Val::None, rect_val),
        Val::Bool(placement.visible),
        Val::Bool(placement.tiled),
        Val::Double(placement.z),
        Val::List(placement.rotation.iter().map(|angle| Val::Double(*angle)).collect()),
        Val::List(placement.scale.iter().map(|scale| Val::Double(*scale)).collect()),
        placement.offset.map_or(Val::None, |(x, _)| Val::Double(x)),
        placement.offset.map_or(Val::None, |(_, y)| Val::Double(y)),
    ])
}

fn triple(value: Option<&Val>, default: [f64; 3], node: NodeId, what: &str) -> Res<[f64; 3]> {
    let Some(value) = value else { return Ok(default) };
    let parts = value.list()?;
    let [x, y, z] = parts else {
        return Err(Fail::type_error(format!(
            "placement for node {node}: {what} must be three finite numbers"
        )));
    };
    let numbers = [x.double()?, y.double()?, z.double()?];
    if !numbers.iter().all(|number| number.is_finite()) {
        return Err(Fail::type_error(format!(
            "placement for node {node}: {what} must be three finite numbers"
        )));
    }
    Ok(numbers)
}

/// The placements a layout answered, each checked against the clients it was asked about.
pub(super) fn placements_of(answer: &Val, children: &[NodeId]) -> Res<Vec<Placement>> {
    let items = answer.list().map_err(|_| {
        Fail::type_error(format!(
            "compute must answer a list of placements, got {}",
            answer.kind_name()
        ))
    })?;
    let mut placements = Vec::with_capacity(items.len());
    for item in items {
        let fields = item.record()?;
        let node =
            client_of(field(fields, 0).ok_or_else(|| Fail::type_error("a placement needs the `node` it is for"))?)?;
        if !children.contains(&node) {
            return Err(Fail::type_error(format!(
                "placement for node {node}, which is not one of the workspace's clients"
            )));
        }
        let bounds = rect_field(fields, 1)?;
        if let Some(bounds) = &bounds {
            if ![bounds.x, bounds.y, bounds.width, bounds.height]
                .iter()
                .all(|n| n.is_finite())
            {
                return Err(Fail::type_error(format!(
                    "placement for node {node}: bounds must be finite numbers"
                )));
            }
            if bounds.width < 0.0 || bounds.height < 0.0 {
                return Err(Fail::type_error(format!(
                    "placement for node {node}: bounds must have a width and a height of at least 0"
                )));
            }
        }
        let z = number_field(fields, 4, 0.0)?;
        let offset = match (field(fields, 7), field(fields, 8)) {
            (None, None) => None,
            (x, y) => Some((x.map_or(Ok(0.0), Val::double)?, y.map_or(Ok(0.0), Val::double)?)),
        };
        let numbers = [z, offset.map_or(0.0, |(x, _)| x), offset.map_or(0.0, |(_, y)| y)];
        if !numbers.iter().all(|number| number.is_finite()) {
            return Err(Fail::type_error(format!(
                "placement for node {node}: z, rotation, scale and offset must be finite numbers"
            )));
        }
        placements.push(Placement {
            node,
            bounds,
            visible: flag_field(fields, 2, true)?,
            tiled: flag_field(fields, 3, false)?,
            z,
            rotation: triple(field(fields, 5), [0.0; 3], node, "rotation")?,
            scale: triple(field(fields, 6), [1.0; 3], node, "scale")?,
            offset,
        });
    }
    Ok(placements)
}

/// A divider as a `layout_split` value.
pub(super) fn split_val(split: &GnoblinStdSplit) -> Val {
    let axis = |wire: u32| member_val(if wire == STD_AXIS_Y { 1 } else { 0 });
    let cross = split.flags & STD_SPLIT_CROSS != 0;
    record(vec![
        Val::Uint(split.token),
        axis(split.axis),
        Val::Int(i64::from(split.direction)),
        Val::Double(split.span),
        Val::Double(split.min),
        Val::Double(split.max),
        Val::Double(split.ratio),
        if cross { axis(split.cross_axis) } else { Val::None },
        Val::Int(i64::from(split.cross_direction)),
        Val::Double(split.cross_span),
        Val::Double(split.cross_min),
        Val::Double(split.cross_max),
        Val::Double(split.cross_ratio),
    ])
}

/// The divider a `layout_split` value says.
pub(super) fn split_raw(value: &Val) -> Res<GnoblinStdSplit> {
    let fields = value.record()?;
    let axis = |at: usize| -> Res<u32> {
        match field(fields, at) {
            Some(axis) => Ok(if wire_of(axis)? == 1 { STD_AXIS_Y } else { STD_AXIS_X }),
            None => Ok(STD_AXIS_X),
        }
    };
    let direction = |at: usize| -> Res<i32> {
        Ok(match field(fields, at) {
            Some(direction) if direction.int()? < 0 => -1,
            _ => 1,
        })
    };
    let cross = field(fields, 7).is_some();
    Ok(GnoblinStdSplit {
        size: size_of::<GnoblinStdSplit>() as u32,
        flags: if cross { STD_SPLIT_CROSS } else { 0 },
        token: field(fields, 0).map_or(Ok(0), Val::uint)?,
        axis: axis(1)?,
        direction: direction(2)?,
        span: number_field(fields, 3, 1.0)?,
        min: number_field(fields, 4, 0.0)?,
        max: number_field(fields, 5, 1.0)?,
        ratio: number_field(fields, 6, 0.5)?,
        cross_axis: axis(7)?,
        cross_direction: direction(8)?,
        cross_span: number_field(fields, 9, 1.0)?,
        cross_min: number_field(fields, 10, 0.0)?,
        cross_max: number_field(fields, 11, 1.0)?,
        cross_ratio: number_field(fields, 12, 0.5)?,
    })
}
