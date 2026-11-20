//! The values the classes of the workspace module move across the protocol: rectangles, gaps, and what a notice of the library says to
//! the objects (`announce`).

use gnoblin_core::Rect;
use gnoblin_object::{Fail, Res, Val};
use gnoblin_std_abi as abi;

use crate::object::record::{field, number_field, record};
use crate::state::{Id, NodeId, Notice, State};
use crate::workspace::check_gap;

/// A notice that no listener of the table hears: the workspace `member` was shown on the output `output`.
pub(crate) const NOTICE_SHOWN: u32 = 0x100;
/// A notice that no listener of the table hears: the workspace `member` stopped being shown on the output `output`.
pub(crate) const NOTICE_HIDDEN: u32 = 0x101;

/// A client as the value it has in the catalog: its node number as text, as the core's `client.id` is. A node number has the top bit set
/// and a script's number (a double in JavaScript) holds 53 bits, so a number would round it.
pub(super) fn client_val(node: NodeId) -> Val {
    Val::Text(node.to_string())
}

/// The node a client value names.
pub(super) fn client_of(value: &Val) -> Res<NodeId> {
    let text = value.text()?;
    text.parse::<NodeId>()
        .map_err(|_| Fail::type_error(format!("a client is the number of its node as text, got `{text}`")))
}

/// A `layout_rect` as a value.
pub(super) fn rect_val(rect: &Rect) -> Val {
    record(vec![
        Val::Double(rect.x),
        Val::Double(rect.y),
        Val::Double(rect.width),
        Val::Double(rect.height),
    ])
}

/// The rectangle of a `layout_rect` value; a field left out is 0. The numbers are not checked.
pub(super) fn rect_of(value: &Val) -> Res<Rect> {
    let fields = value.record()?;
    Ok(Rect::new(
        number_field(fields, 0, 0.0)?,
        number_field(fields, 1, 0.0)?,
        number_field(fields, 2, 0.0)?,
        number_field(fields, 3, 0.0)?,
    ))
}

/// The rectangle of a `layout_rect` value, whose four numbers are finite and whose size is at least nothing.
pub(super) fn checked_rect(value: &Val, what: &str) -> Res<Rect> {
    let rect = rect_of(value)?;
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|number| number.is_finite())
        || rect.width < 0.0
        || rect.height < 0.0
    {
        return Err(Fail::type_error(format!(
            "{what} must be a rectangle of four finite numbers with a width and a height of at least 0"
        )));
    }
    Ok(rect)
}

/// A `gaps` value.
pub(super) fn gaps_val(inner: f64, outer: f64) -> Val {
    record(vec![Val::Double(inner), Val::Double(outer)])
}

/// The two gaps of a `gaps` value, each a finite number of at least 0; one left out is 0.
pub(super) fn gaps_of(value: &Val, what: &str) -> Res<(f64, f64)> {
    let fields = value.record()?;
    let inner = check_gap(number_field(fields, 0, 0.0)?, &format!("{what}.inner")).map_err(Fail::from)?;
    let outer = check_gap(number_field(fields, 1, 0.0)?, &format!("{what}.outer")).map_err(Fail::from)?;
    Ok((inner, outer))
}

/// A value of an enum by the number the library's table gives it, which counts from zero where a catalog enum counts from one.
pub(super) fn member_val(wire: u32) -> Val {
    Val::Enum {
        class: 0,
        value: wire + 1,
    }
}

/// The number the library's table gives the member of an enum value.
pub(super) fn wire_of(value: &Val) -> Res<u32> {
    value
        .enum_value()?
        .checked_sub(1)
        .ok_or_else(|| Fail::type_error("not a value of the enum"))
}

/// The nodes in a list of numbers.
pub(super) fn nodes_of(value: &Val) -> Res<Vec<NodeId>> {
    value.list()?.iter().map(Val::uint).collect()
}

/// A field of a record that is a rectangle, when it is given.
pub(super) fn rect_field(fields: &[Val], at: usize) -> Res<Option<Rect>> {
    field(fields, at).map(rect_of).transpose()
}

// ---- what a notice says to the objects ------------------------------------------------------------------------------------------

/// The object a signal is told on.
pub(super) enum SignalHome {
    Workspace(Id),
    Collection(Id),
    /// The list of workspaces.
    List,
}

/// One field of a signal's payload.
pub(super) enum SignalField {
    Plain(Val),
    /// The object of a workspace, made when a shell has none yet.
    Workspace(Id),
}

/// The signals a notice is, on which objects, with what payload.
pub(super) fn told_by(st: &State, notice: &Notice) -> Vec<(SignalHome, &'static str, Vec<SignalField>)> {
    let mut told = Vec::new();
    match notice.kind {
        abi::STD_EVENT_ADDED | abi::STD_EVENT_REMOVED => {
            let added = notice.kind == abi::STD_EVENT_ADDED;
            for holder in &notice.holders {
                let Some(coll) = st.colls.get(holder) else { continue };
                if coll.kind == abi::STD_COLLECTION_CLIENTS {
                    told.push((
                        SignalHome::Collection(*holder),
                        if added { "added" } else { "removed" },
                        vec![SignalField::Plain(client_val(notice.member))],
                    ));
                    if let Some(workspace) = st
                        .ws
                        .iter()
                        .find_map(|(id, workspace)| (workspace.coll == *holder).then_some(*id))
                    {
                        told.push((
                            SignalHome::Workspace(workspace),
                            if added { "client_added" } else { "client_removed" },
                            vec![SignalField::Plain(client_val(notice.member))],
                        ));
                    }
                } else if coll.kind == abi::STD_COLLECTION_WORKSPACES && *holder == st.roots[2] {
                    told.push((
                        SignalHome::List,
                        if added { "added" } else { "removed" },
                        vec![SignalField::Workspace(notice.member)],
                    ));
                }
            }
        }
        NOTICE_SHOWN => told.push((
            SignalHome::Workspace(notice.member),
            "shown",
            vec![SignalField::Plain(Val::Uint(notice.output))],
        )),
        NOTICE_HIDDEN => told.push((SignalHome::Workspace(notice.member), "hidden", Vec::new())),
        _ => {}
    }
    told
}
