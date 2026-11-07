//! Extension namespaces as Lua modules.
//!
//! `require("example.ext")` returns the namespace's module. The module holds one table
//! per object type the host can see (the schema with its secret members removed), and
//! each object table holds its methods and an `on` subscription:
//!
//! ```lua
//! local ext = require("example.ext")
//! local reply = ext.probe.lookup{ id = 1 }   -- Object.Method, fields in, fields out
//! ext.probe.on("changed", function(event) print(event.value) end)
//! ```
//!
//! An extension call has no object identity on the wire: the member is `Object.Method`
//! and the arguments are the method's declared parameters, so the table is a namespace
//! of functions and not of instances. A namespace is bound only when the compositor
//! granted it to this connection, and the schema decides every argument and every
//! signal field a script can see.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gnoblin_client::Transport;
use gnoblin_model::{Fields, Value as ModelValue};
use gnoblin_schema::{Field, ObjectType, Type};
use mlua::{AnyUserData, Error, Function, Lua, Table, Value};

use crate::bindings::{MODULES, Subscription, callbacks};
use crate::extensions::ExtensionRegistry;
use crate::host::{Shared, Watching};
use crate::layout::is_layout_name;

/// The name a method or signal table may not use: `on` is the subscription entry.
const RESERVED: &str = "on";

/// Bind every granted, loaded namespace and replace `require` with the extension loader.
///
/// Calling this again replaces the modules, so a host is bound from the registry the
/// compositor built and never from a stale one.
pub(super) fn bind<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    extensions: &ExtensionRegistry,
    granted: &BTreeSet<String>,
) -> mlua::Result<()> {
    let modules = lua.create_table()?;
    lua.set_named_registry_value(MODULES, modules.clone())?;
    let mut loaded = BTreeSet::new();
    for (name, schema) in extensions.iter() {
        loaded.insert(name.to_owned());
        if !granted.contains(name) {
            continue;
        }
        let module = lua.create_table()?;
        for object in &schema.objects {
            let Some(visible) = object.without_secrets() else {
                continue;
            };
            module.set(visible.name.as_str(), class(lua, shared, name, &visible)?)?;
        }
        modules.set(name, module)?;
    }
    let granted = granted.clone();
    let layouts = Rc::clone(shared);
    lua.globals().set(
        "require",
        lua.create_function(move |lua, name: String| {
            let modules: Table = lua.named_registry_value(MODULES)?;
            if let Some(module) = modules.raw_get::<Option<Table>>(name.as_str())? {
                return Ok(Value::Table(module));
            }
            if !loaded.contains(&name) {
                // Not an extension namespace. A layout name answers with a reference; any
                // other name is a plain module, run once per evaluation.
                if is_layout_name(&name) {
                    let reference = layouts.borrow_mut().layouts.reference(&name).map_err(Error::runtime)?;
                    return Ok(Value::UserData(lua.create_userdata(reference)?));
                }
                let loader = layouts.borrow().modules.clone();
                let loader = loader.ok_or_else(|| {
                    Error::runtime(format!("unknown module `{name}`: no configuration is being evaluated"))
                })?;
                let (value, _) = loader.fetch(lua, &name)?;
                return Ok(value);
            }
            if !granted.contains(&name) {
                return Err(Error::runtime(format!("extension not granted: {name}")));
            }
            Err(Error::runtime(format!(
                "extension `{name}` has no object types to bind"
            )))
        })?,
    )?;
    Ok(())
}

/// One object type's table: a function per method and the `on` subscription.
fn class<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    namespace: &str,
    object: &ObjectType,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for method in &object.methods {
        if method.name == RESERVED {
            return Err(Error::runtime(format!(
                "{namespace}.{}: method `{RESERVED}` is reserved for signal subscriptions",
                object.name
            )));
        }
        let member = format!("{}.{}", object.name, method.name);
        let params = method.params.clone();
        let shared = Rc::clone(shared);
        let namespace = namespace.to_owned();
        table.set(
            method.name.as_str(),
            lua.create_function(move |lua, arguments: Option<Table>| {
                call(lua, &shared, &namespace, &member, &params, arguments)
            })?,
        )?;
    }
    let signals: Vec<String> = object.signals.iter().map(|signal| signal.name.clone()).collect();
    let shared = Rc::clone(shared);
    let namespace = namespace.to_owned();
    let object_name = object.name.clone();
    table.set(
        RESERVED,
        lua.create_function(move |lua, (signal, callback): (String, Function)| {
            subscribe(lua, &shared, &namespace, &object_name, &signals, signal, callback)
        })?,
    )?;
    Ok(table)
}

/// Call one method: convert the argument table by the declared parameters, send it,
/// and return the reply as a table.
fn call<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    namespace: &str,
    member: &str,
    params: &[Field],
    arguments: Option<Table>,
) -> mlua::Result<Table> {
    let label = format!("{namespace}.{member}");
    let supplied = match arguments {
        Some(table) => table,
        None => lua.create_table()?,
    };
    for pair in supplied.pairs::<Value, Value>() {
        let (key, _) = pair?;
        let name = match &key {
            Value::String(text) => text.to_str()?.to_string(),
            _ => return Err(Error::runtime(format!("{label} takes named arguments only"))),
        };
        if !params.iter().any(|param| param.name == name) {
            return Err(Error::runtime(format!(
                "{label} has no argument `{name}`; arguments: {}",
                parameter_names(params)
            )));
        }
    }
    let mut fields = Fields::new();
    for param in params {
        if matches!(param.ty, Type::ObjectRef(_) | Type::List(_)) {
            return Err(Error::runtime(format!(
                "{label} argument `{}` is an object reference or a list; extension calls take scalar arguments only",
                param.name
            )));
        }
        let value: Value = supplied.get(param.name.as_str())?;
        if matches!(value, Value::Nil) {
            return Err(Error::runtime(format!("{label} needs argument `{}`", param.name)));
        }
        fields.insert(param.name.clone(), argument(&param.ty, value, &label, &param.name)?);
    }
    let reply = shared
        .borrow_mut()
        .client
        .call_extension(namespace, member, &fields)
        .map_err(|error| Error::runtime(format!("{label}: {error}")))?;
    let table = lua.create_table()?;
    for (name, value) in &reply {
        table.set(name.as_str(), reply_value(lua, value)?)?;
    }
    Ok(table)
}

/// The argument names a method takes, for a refusal.
fn parameter_names(params: &[Field]) -> String {
    if params.is_empty() {
        return "none".to_owned();
    }
    params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// One Lua argument as the typed value the schema declares for it.
fn argument(ty: &Type, value: Value, label: &str, name: &str) -> mlua::Result<ModelValue> {
    let refuse = |expected: &str| {
        Error::runtime(format!(
            "{label} argument `{name}` expects {expected}, found {}",
            value.type_name()
        ))
    };
    Ok(match (ty, &value) {
        (Type::Bool, Value::Boolean(flag)) => ModelValue::Bool(*flag),
        (Type::Int, Value::Integer(number)) => ModelValue::Int(*number),
        (Type::Uint, Value::Integer(number)) => match u64::try_from(*number) {
            Ok(number) => ModelValue::Uint(number),
            Err(_) => return Err(refuse("an unsigned integer")),
        },
        (Type::Double, Value::Number(number)) => ModelValue::Double(*number),
        (Type::Double, Value::Integer(number)) => ModelValue::Double(*number as f64),
        (Type::Text, Value::String(text)) => ModelValue::Text(text.to_str()?.to_string()),
        (Type::Bytes, Value::String(text)) => ModelValue::Bytes(text.as_bytes().to_vec()),
        (Type::Bool, _) => return Err(refuse("a boolean")),
        (Type::Int, _) => return Err(refuse("an integer")),
        (Type::Uint, _) => return Err(refuse("an unsigned integer")),
        (Type::Double, _) => return Err(refuse("a number")),
        (Type::Text, _) => return Err(refuse("a string")),
        (Type::Bytes, _) => return Err(refuse("a byte string")),
        (Type::ObjectRef(_) | Type::List(_), _) => {
            return Err(Error::runtime(format!(
                "{label} argument `{name}` is an object reference or a list; extension calls take scalar arguments only"
            )));
        }
    })
}

/// One typed reply value as Lua sees it.
fn reply_value(lua: &Lua, value: &ModelValue) -> mlua::Result<Value> {
    Ok(match value {
        ModelValue::Bool(flag) => Value::Boolean(*flag),
        ModelValue::Int(number) => Value::Integer(*number),
        ModelValue::Uint(number) => match i64::try_from(*number) {
            Ok(number) => Value::Integer(number),
            Err(_) => {
                return Err(Error::runtime(format!(
                    "extension reply carries {number}, above the largest integer Lua can read"
                )));
            }
        },
        ModelValue::Double(number) => Value::Number(*number),
        ModelValue::Text(text) => Value::String(lua.create_string(text)?),
        ModelValue::Bytes(bytes) => Value::String(lua.create_string(bytes)?),
        ModelValue::List(items) => {
            let table = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                table.raw_set(index + 1, reply_value(lua, item)?)?;
            }
            Value::Table(table)
        }
        ModelValue::Object(_) => {
            return Err(Error::runtime(
                "an extension payload carries an object reference, which Lua cannot bind yet",
            ));
        }
    })
}

/// Register one `on` handler for a declared signal of an object.
fn subscribe<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    namespace: &str,
    object: &str,
    signals: &[String],
    signal: String,
    callback: Function,
) -> mlua::Result<AnyUserData> {
    if !signals.contains(&signal) {
        return Err(Error::runtime(format!(
            "{namespace}.{object} has no signal `{signal}`; signals: {}",
            if signals.is_empty() {
                "none".to_owned()
            } else {
                signals.join(", ")
            }
        )));
    }
    let handler = shared.borrow_mut().allocate();
    callbacks(lua)?.raw_set(handler, callback)?;
    shared.borrow_mut().watch(
        handler,
        Watching::Extension {
            namespace: namespace.to_owned(),
            member: format!("{object}.{signal}"),
        },
    );
    lua.create_userdata(Subscription {
        shared: Rc::clone(shared),
        handler,
    })
}

/// The fields a signal may deliver to Lua: the schema's declared, non-secret fields.
///
/// `member` is `Object.Signal`. A signal the registry does not declare, or one whose
/// object is secret, answers `None`, so the host drops it rather than guessing a shape.
pub(super) fn signal_fields(extensions: &ExtensionRegistry, namespace: &str, member: &str) -> Option<Vec<Field>> {
    let (object, signal) = member.split_once('.')?;
    let schema = extensions.schema(namespace)?;
    let visible = schema
        .objects
        .iter()
        .find(|candidate| candidate.name == object)?
        .without_secrets()?;
    visible
        .signals
        .iter()
        .find(|declared| declared.name == signal)
        .map(|declared| declared.fields.clone())
}

/// The argument table a signal handler receives: only the declared fields, in the
/// shape the schema gives them.
pub(super) fn signal_table(lua: &Lua, declared: &[Field], fields: &Fields) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for field in declared {
        if let Some(value) = fields.get(&field.name) {
            table.set(field.name.as_str(), reply_value(lua, value)?)?;
        }
    }
    Ok(table)
}
