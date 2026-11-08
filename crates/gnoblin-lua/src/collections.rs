//! The collections of live objects: `gnoblin.windows`, `gnoblin.outputs`,
//! `gnoblin.workspaces` and `gnoblin.find`.
//!
//! Each one reads the mirror, so an answer is as of the last pump. An object that
//! closes leaves every collection at the same time it leaves the mirror.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_client::Transport;
use gnoblin_model::ObjectId;
use mlua::{Error, Function, Lua, Table, Value};

use crate::bindings::{self, MemberSpec, ObjectSpec, Ty, find_namespace, find_object, holds, instance, to_lua};
use crate::host::Shared;

/// The namespace of the core object types, which sit directly on `gnoblin`.
const CORE: &str = "gnoblin";

/// The object kinds `find` filters, as the name a script passes and the schema's object
/// name. A kind outside this list has no collection in the current schema, so `find`
/// refuses it and names these.
const FIND_KINDS: &[(&str, &str)] = &[("Output", "output"), ("Window", "window"), ("Workspace", "workspace")];

/// Install the collections and `find` on `gnoblin`.
pub(super) fn install<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    gnoblin: &Table,
) -> mlua::Result<()> {
    gnoblin.set("windows", windows(lua, shared)?)?;
    gnoblin.set("outputs", outputs(lua, shared)?)?;
    gnoblin.set("workspaces", workspaces(lua, shared)?)?;
    {
        let shared = Rc::clone(shared);
        gnoblin.set(
            "find",
            lua.create_function(move |lua, (kind, filter): (String, Value)| find(lua, &shared, &kind, filter))?,
        )?;
    }
    Ok(())
}

/// The identities of every mirrored object of one core type, in identity order.
fn ids_of<T: Transport>(shared: &Rc<RefCell<Shared<T>>>, namespace: &str, spec: &ObjectSpec) -> Vec<ObjectId> {
    shared
        .borrow()
        .mirror
        .iter()
        .filter(|(_, view)| view.ty().namespace() == namespace && view.ty().name() == spec.name)
        .map(|(id, _)| *id)
        .collect()
}

/// `gnoblin.windows`: `list()`, `focused()` and `by_id(id)`.
fn windows<T: Transport + 'static>(lua: &Lua, shared: &Rc<RefCell<Shared<T>>>) -> mlua::Result<Table> {
    let spec = core_spec("window")?;
    let table = lua.create_table()?;
    table.set("list", list_function(lua, shared, spec)?)?;
    table.set("focused", first_function(lua, shared, spec, "focused", true)?)?;
    table.set("by_id", by_id_function(lua, shared, spec)?)?;
    Ok(table)
}

/// `gnoblin.outputs`: `list()`, `primary()` and `by_id(name)`. An output's identity is
/// its connector name, so `by_id` takes that name and not a number.
fn outputs<T: Transport + 'static>(lua: &Lua, shared: &Rc<RefCell<Shared<T>>>) -> mlua::Result<Table> {
    let spec = core_spec("output")?;
    let table = lua.create_table()?;
    table.set("list", list_function(lua, shared, spec)?)?;
    table.set("primary", first_function(lua, shared, spec, "primary", true)?)?;
    {
        let shared = Rc::clone(shared);
        table.set(
            "by_id",
            lua.create_function(move |lua, name: String| {
                let wanted = lua.create_string(&name)?;
                first_match(lua, &shared, spec, &[("name", Value::String(wanted))])
            })?,
        )?;
    }
    Ok(table)
}

/// `gnoblin.workspaces`: `list()`, `by_id(id)` and `set_activate(name)`.
fn workspaces<T: Transport + 'static>(lua: &Lua, shared: &Rc<RefCell<Shared<T>>>) -> mlua::Result<Table> {
    let spec = core_spec("workspace")?;
    let table = lua.create_table()?;
    table.set("list", list_function(lua, shared, spec)?)?;
    table.set("by_id", by_id_function(lua, shared, spec)?)?;
    table.set("set_activate", set_activate_function(lua, shared)?)?;
    Ok(table)
}

/// A function that asks the window manager to make the configured workspace set `name` the
/// live set. Refused when the session has not published its `workspace_set` object yet.
fn set_activate_function<T: Transport + 'static>(lua: &Lua, shared: &Rc<RefCell<Shared<T>>>) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, name: String| {
        let spec = core_spec("workspace_set")?;
        let object = ids_of(&shared, CORE, spec).first().copied().ok_or_else(|| {
            Error::runtime("gnoblin.workspaces.set_activate() was refused: the session has no workspace_set object")
        })?;
        bindings::workspace_set_activate(lua, &shared, object, name)
    })
}

/// The spec of a core object type, which the schema always declares.
fn core_spec(name: &str) -> mlua::Result<&'static ObjectSpec> {
    find_namespace(CORE)
        .and_then(|namespace| find_object(namespace, name))
        .ok_or_else(|| Error::runtime(format!("the schema declares no core object `{name}`")))
}

/// A function that answers every live object of one type, in identity order.
fn list_function<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    spec: &'static ObjectSpec,
) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, ()| {
        let ids = ids_of(&shared, CORE, spec);
        list_table(lua, spec, &ids)
    })
}

/// A function that answers the first object whose boolean property equals `wanted`, or nil.
fn first_function<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    spec: &'static ObjectSpec,
    property: &'static str,
    wanted: bool,
) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, ()| first_match(lua, &shared, spec, &[(property, Value::Boolean(wanted))]))
}

/// A function that answers one object by its identity, or nil when the mirror does not hold
/// it with this type. A negative number is not an identity, so it answers nil.
fn by_id_function<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    spec: &'static ObjectSpec,
) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, id: i64| {
        if id < 0 {
            return Ok(Value::Nil);
        }
        let object = ObjectId::new(id as u64);
        if !holds(&shared, CORE, spec, object) {
            return Ok(Value::Nil);
        }
        Ok(Value::Table(instance(lua, CORE, spec, object)?))
    })
}

/// `gnoblin.find(kind, filter?)`: the live objects of one kind that match a filter.
///
/// A table filter keeps the objects whose scalar fields equal every value in it. A function
/// filter keeps the objects it returns a truthy value for; an error inside it is an error of
/// this call.
fn find<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    kind: &str,
    filter: Value,
) -> mlua::Result<Table> {
    let Some(&(_, object)) = FIND_KINDS.iter().find(|(name, _)| *name == kind) else {
        let names: Vec<&str> = FIND_KINDS.iter().map(|(name, _)| *name).collect();
        return Err(Error::runtime(format!(
            "gnoblin.find has no list for the kind `{kind}`; the kinds with a list are {}",
            names.join(", ")
        )));
    };
    let spec = core_spec(object)?;
    let ids = ids_of(shared, CORE, spec);
    let kept = match filter {
        Value::Nil => ids,
        Value::Function(predicate) => keep_where(lua, spec, ids, &predicate)?,
        Value::Table(fields) => {
            let criteria = field_criteria(spec, &fields)?;
            let mut kept = Vec::new();
            for id in ids {
                if keeps(lua, shared, id, &criteria)? {
                    kept.push(id);
                }
            }
            kept
        }
        other => {
            return Err(Error::runtime(format!(
                "gnoblin.find takes a table or a function as its filter, found a {}",
                other.type_name()
            )));
        }
    };
    list_table(lua, spec, &kept)
}

/// The objects a predicate keeps. Lua's truth applies: only nil and false drop an object.
fn keep_where(
    lua: &Lua,
    spec: &'static ObjectSpec,
    ids: Vec<ObjectId>,
    predicate: &Function,
) -> mlua::Result<Vec<ObjectId>> {
    let mut kept = Vec::new();
    for id in ids {
        let verdict: Value = predicate.call(instance(lua, CORE, spec, id)?)?;
        if !matches!(verdict, Value::Nil | Value::Boolean(false)) {
            kept.push(id);
        }
    }
    Ok(kept)
}

/// The criteria of a table filter, checked against the type's scalar fields.
///
/// A field that is not a scalar cannot be filtered by, and a value that is not a string,
/// a number or a boolean cannot be compared. Both are refused by name, before any object is
/// read, so a misspelt filter fails instead of matching nothing.
fn field_criteria(spec: &'static ObjectSpec, fields: &Table) -> mlua::Result<Vec<(&'static str, Value)>> {
    let mut criteria = Vec::new();
    for pair in fields.pairs::<Value, Value>() {
        let (key, wanted) = pair?;
        let name = match &key {
            Value::String(name) => name.to_str()?.to_string(),
            other => {
                return Err(Error::runtime(format!(
                    "gnoblin.find filters by field name, found a {} key",
                    other.type_name()
                )));
            }
        };
        let Some(property) = spec
            .properties
            .iter()
            .find(|property| property.name == name && is_scalar(property))
        else {
            return Err(Error::runtime(format!(
                "{} has no field `{name}` to filter by; the fields that filter are: {}",
                spec.name,
                scalar_names(spec)
            )));
        };
        match wanted {
            Value::Boolean(_) | Value::Integer(_) | Value::Number(_) | Value::String(_) => {
                criteria.push((property.name, wanted));
            }
            other => {
                return Err(Error::runtime(format!(
                    "{}.{name} cannot be filtered by a {}; a filter value is a string, a number or a boolean",
                    spec.name,
                    other.type_name()
                )));
            }
        }
    }
    Ok(criteria)
}

/// Whether a property holds a plain value that a filter can compare.
fn is_scalar(property: &MemberSpec) -> bool {
    matches!(
        property.ty,
        Ty::Bool | Ty::Int | Ty::Uint | Ty::Double | Ty::Text | Ty::Bytes
    )
}

/// The names of the scalar fields, for a refusal to list.
fn scalar_names(spec: &ObjectSpec) -> String {
    spec.properties
        .iter()
        .filter(|property| is_scalar(property))
        .map(|property| property.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The first object that keeps every criterion, as an instance, or nil.
fn first_match<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    spec: &'static ObjectSpec,
    criteria: &[(&str, Value)],
) -> mlua::Result<Value> {
    for id in ids_of(shared, CORE, spec) {
        if keeps(lua, shared, id, criteria)? {
            return Ok(Value::Table(instance(lua, CORE, spec, id)?));
        }
    }
    Ok(Value::Nil)
}

/// Whether one object keeps every criterion. An object with no value for a property keeps
/// nothing.
fn keeps<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    id: ObjectId,
    criteria: &[(&str, Value)],
) -> mlua::Result<bool> {
    for (property, wanted) in criteria {
        let value = shared
            .borrow()
            .mirror
            .get(&id)
            .and_then(|view| view.property(property).cloned());
        let Some(value) = value else {
            return Ok(false);
        };
        let value = to_lua(lua, shared, &value)?;
        if !same(&value, wanted) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether a stored scalar equals a filter value. Integers and numbers compare by value.
fn same(stored: &Value, wanted: &Value) -> bool {
    match (stored, wanted) {
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Integer(left), Value::Integer(right)) => left == right,
        _ => match (number(stored), number(wanted)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
    }
}

/// A Lua number as a float, for a comparison between an integer and a number.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(number) => Some(*number as f64),
        Value::Number(number) => Some(*number),
        _ => None,
    }
}

/// A Lua array of instances, in the order given.
fn list_table(lua: &Lua, spec: &'static ObjectSpec, ids: &[ObjectId]) -> mlua::Result<Table> {
    let list = lua.create_table()?;
    for (position, id) in ids.iter().enumerate() {
        list.raw_seti(position + 1, instance(lua, CORE, spec, *id)?)?;
    }
    Ok(list)
}
