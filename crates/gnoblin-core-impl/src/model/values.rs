//! What the model reads of the tree and how it turns it into values of the protocol.
//!
//! A window's or an output's properties are the tree's: this module reads one as the tick sees it (the shell's own pending writes
//! included, what a getter answers) or as the compositor last published it (what an event compares), and falls back to the schema's
//! default for a property nothing has written.

use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Res, Val};

use crate::catalog;
use crate::session;
use crate::values::OwnedValue;

/// The catalog id of the tree class `name` (`window`, `screen`), zero when the catalog has none.
pub(super) fn tree_class(name: &str) -> u32 {
    catalog::tree_class_id(name)
}

/// The schema class of `node`, or none when it is not in the tree.
pub(super) fn class_name(node: u64) -> Option<String> {
    session::with(|session| session.tree_ref().ok().and_then(|tree| tree.class_of(node)))
}

/// Whether the shell has attached to the tree.
pub(super) fn attached() -> bool {
    session::with(|session| session.tree_ref().is_ok())
}

/// The nodes of `class` the compositor has published, ordered by id.
pub(super) fn nodes_of(class: &str) -> Vec<u64> {
    session::with(|session| {
        let Ok(tree) = session.tree_ref() else {
            return Vec::new();
        };
        let mut nodes: Vec<u64> = tree
            .mirror()
            .ids()
            .filter(|id| tree.class_of(*id).as_deref() == Some(class))
            .collect();
        nodes.sort_unstable();
        nodes
    })
}

/// What a property reads as before anything wrote it.
fn default_of(class: u32, key: &str) -> OwnedValue {
    catalog::property_id(class, key)
        .and_then(|id| catalog::property_def(class, id))
        .and_then(|def| OwnedValue::default_of(def.value_kind, def.default))
        .unwrap_or(OwnedValue::Double(0.0))
}

/// `key` of `node` as this tick sees it.
pub(super) fn live(node: u64, class: u32, key: &str) -> OwnedValue {
    session::with(|session| {
        session
            .tree_ref()
            .ok()
            .and_then(|tree| tree.get(node, key).and_then(OwnedValue::from_mirror))
    })
    .unwrap_or_else(|| default_of(class, key))
}

/// `key` of `node` as the compositor last published it.
pub(super) fn published(node: u64, class: u32, key: &str) -> OwnedValue {
    session::with(|session| {
        session
            .tree_ref()
            .ok()
            .and_then(|tree| tree.mirror().value(node, key).and_then(OwnedValue::from_mirror))
    })
    .unwrap_or_else(|| default_of(class, key))
}

impl OwnedValue {
    /// A number, whatever kind of number the tree holds.
    pub(super) fn number(&self) -> f64 {
        match self {
            OwnedValue::Double(value) => *value,
            OwnedValue::Int(value) => *value as f64,
            OwnedValue::Uint(value) | OwnedValue::Node(value) => *value as f64,
            _ => 0.0,
        }
    }

    /// Text, or empty for a value that is not text.
    pub(super) fn string(&self) -> String {
        match self {
            OwnedValue::Text(value) => value.clone(),
            _ => String::new(),
        }
    }

    /// A flag.
    pub(super) fn flag(&self) -> bool {
        matches!(self, OwnedValue::Bool(true))
    }

    /// The node a value names, zero for none.
    pub(super) fn node(&self) -> u64 {
        match self {
            OwnedValue::Node(value) | OwnedValue::Uint(value) => *value,
            _ => 0,
        }
    }

    /// The numbers of a list.
    pub(super) fn numbers(&self) -> Vec<f64> {
        match self {
            OwnedValue::List(values) => values.clone(),
            _ => Vec::new(),
        }
    }
}

/// A record of `class` (a class of this catalog) from fields given by name: the catalog says where each one goes.
pub(super) fn record(cx: &Cx<'_>, class: &str, named: Vec<(&str, Val)>) -> Res<Val> {
    let names = cx.record_fields(class)?;
    let mut fields = vec![Val::None; names.len()];
    for (name, value) in named {
        let at = names
            .iter()
            .position(|held| *held == name)
            .ok_or_else(|| Fail::no_such(format!("the record `{class}` has no field `{name}`")))?;
        fields[at] = value;
    }
    Ok(Val::Record {
        class: cx.class_named(class)?,
        fields,
    })
}

/// The fields of a record value by name, in the order of the catalog.
pub(super) fn fields_of<'a>(cx: &Cx<'_>, class: &str, value: &'a Val) -> Res<Vec<(&'static str, &'a Val)>> {
    let names = cx.record_fields(class)?;
    let fields = value.record()?;
    Ok(names.into_iter().zip(fields.iter()).collect())
}

/// A `rect`.
pub(super) fn rect(cx: &Cx<'_>, x: f64, y: f64, width: f64, height: f64) -> Res<Val> {
    record(
        cx,
        "rect",
        vec![
            ("x", Val::Double(x)),
            ("y", Val::Double(y)),
            ("width", Val::Double(width)),
            ("height", Val::Double(height)),
        ],
    )
}

/// A `size`.
pub(super) fn size(cx: &Cx<'_>, width: f64, height: f64) -> Res<Val> {
    record(
        cx,
        "size",
        vec![("width", Val::Double(width)), ("height", Val::Double(height))],
    )
}

/// A number the shell gave that must be finite.
pub(super) fn finite(value: &Val, target: &str) -> Res<f64> {
    let number = value.double()?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(Fail::type_error(format!("{target}: must be a finite number")))
    }
}

/// The names of the enum `name`'s values in the order the catalog declares them, for a conversion of a value to text.
pub(super) fn enum_name(cx: &Cx<'_>, value: &Val) -> Res<String> {
    let Val::Enum { class, value } = value else {
        return Err(Fail::type_error("an enum value was expected"));
    };
    let def = cx
        .class_def_of(*class)
        .ok_or_else(|| Fail::type_error("the enum is not in the catalog"))?;
    def.member(*value)
        .map(|member| member.name.to_owned())
        .ok_or_else(|| Fail::type_error("the enum has no such value"))
}
