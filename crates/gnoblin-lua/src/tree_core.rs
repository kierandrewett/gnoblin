//! The Lua `core` table: the nine calls `data/lua/gnoblin/node.lua` makes into the shell's
//! tree (design/node-core.md, section 4 and slice 10).
//!
//! The table is built over a [`TreeCore`]: the mirror the tick started from, the transaction
//! builder that gathers this tick's writes, and the class schema the shell was given. Reads
//! see this tick's writes first. Writes are checked against the schema here, so a refused
//! write never reaches the transaction: an unknown class or key, a readonly property, a value
//! of the wrong kind, a bad animation, and every refusal the builder makes (`no node`,
//! `already exists`, `cycle`, `root`).
//!
//! Node ids cross into Lua as integers. An id the shell allocated (bit 63 clear) round-trips
//! unchanged. Nil is `0` for a node reference and `nil` for a missing property.
//!
//! The class list is data the caller supplies, parent first. The compositor's own schema
//! reader cannot be a dependency of this crate, so the caller reads the tree schema and hands
//! the classes in.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use crate::tree_mirror::{LuaMirror, LuaTransaction, LuaTxBuilder, LuaValue, NIL, NodeId};

/// The bit an epoch occupies above the per-connection counter in a shell id.
const EPOCH_SHIFT: u32 = 32;
/// Epochs stay below this, so every shell id stays below bit 63.
const EPOCH_LIMIT: u64 = 1 << 31;

/// The wire kind of a property, as the schema names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeValueKind {
    Bool,
    Int,
    Uint,
    Double,
    Text,
    Node,
    Nodes,
    ListDouble,
}

impl TreeValueKind {
    /// The name the schema and `core.classes()` use for this kind.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Uint => "uint",
            Self::Double => "double",
            Self::Text => "text",
            Self::Node => "node",
            Self::Nodes => "nodes",
            Self::ListDouble => "list_double",
        }
    }
}

/// One property a class declares itself. Inherited properties are found through `base`.
#[derive(Clone, Debug, PartialEq)]
pub struct PropSpec {
    pub name: String,
    pub kind: TreeValueKind,
    pub readonly: bool,
    pub default: Option<LuaValue>,
}

/// One class of the tree schema. Its members are its own; `base` names the parent class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassSpec {
    pub name: String,
    pub base: Option<String>,
    pub props: Vec<PropSpec>,
    pub signals: Vec<String>,
    pub methods: Vec<String>,
    /// The methods that return a signal instead of a value.
    pub slow: Vec<String>,
}

/// The shell's side of the tree: what it has seen, what it has written this tick, and the
/// schema it writes against.
#[derive(Debug)]
pub struct TreeCore {
    mirror: LuaMirror,
    tick: LuaTxBuilder,
    classes: Vec<ClassSpec>,
    counter: u64,
    reports: Vec<String>,
}

/// The state shared by the `core` table's functions.
pub type Shared = Rc<RefCell<TreeCore>>;

impl TreeCore {
    /// The core over `mirror`, with `classes` parent first. A class declared twice, or a base
    /// that is not declared before its subclass, is an error naming the class.
    pub fn shared(mirror: LuaMirror, classes: Vec<ClassSpec>) -> Result<Shared, String> {
        for (index, class) in classes.iter().enumerate() {
            if classes[..index].iter().any(|earlier| earlier.name == class.name) {
                return Err(format!("class {} is declared twice", class.name));
            }
            if let Some(base) = &class.base
                && !classes[..index].iter().any(|earlier| &earlier.name == base)
            {
                return Err(format!(
                    "class {} has base {base}, which is not declared before it",
                    class.name
                ));
            }
        }
        Ok(Rc::new(RefCell::new(Self {
            mirror,
            tick: LuaTxBuilder::new(),
            classes,
            counter: 0,
            reports: Vec::new(),
        })))
    }

    /// The mirror as the last applied batch left it.
    pub const fn mirror(&self) -> &LuaMirror {
        &self.mirror
    }

    /// The mirror, for applying a change batch or a snapshot. Call it only between ticks: the
    /// writes gathered in a tick read through the mirror the tick started from.
    pub fn mirror_mut(&mut self) -> &mut LuaMirror {
        &mut self.mirror
    }

    /// Take the writes of this tick as one transaction stamped with `serial`, and start the
    /// next tick against the same mirror.
    pub fn take_transaction(&mut self, serial: u64) -> LuaTransaction {
        let tick = std::mem::take(&mut self.tick);
        tick.into_transaction(&self.mirror, serial)
    }

    /// The messages `core.report` received since the last call, oldest first.
    pub fn take_reports(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reports)
    }

    fn class(&self, name: &str) -> Option<&ClassSpec> {
        self.classes.iter().find(|class| class.name == name)
    }

    /// The property `key` of `class`, searched through its bases.
    fn prop(&self, class: &str, key: &str) -> Option<&PropSpec> {
        let mut spec = self.class(class);
        while let Some(current) = spec {
            if let Some(prop) = current.props.iter().find(|prop| prop.name == key) {
                return Some(prop);
            }
            spec = current.base.as_deref().and_then(|base| self.class(base));
        }
        None
    }

    /// Whether `class` or one of its bases declares `method`.
    fn has_method(&self, class: &str, method: &str) -> bool {
        let mut spec = self.class(class);
        while let Some(current) = spec {
            if current.methods.iter().any(|name| name == method) {
                return true;
            }
            spec = current.base.as_deref().and_then(|base| self.class(base));
        }
        false
    }

    fn class_of(&self, node: NodeId) -> Result<String, String> {
        self.tick
            .node(&self.mirror, node)
            .map(|record| record.class.clone())
            .ok_or_else(|| format!("no node {node}"))
    }

    fn new_node(&mut self, class: &str) -> Result<NodeId, String> {
        if self.class(class).is_none() {
            return Err(format!("no class {class}"));
        }
        let epoch = self.mirror.epoch();
        if epoch >= EPOCH_LIMIT {
            return Err(format!("epoch {epoch} is past the shell's id range"));
        }
        self.counter += 1;
        if self.counter >= 1 << EPOCH_SHIFT {
            return Err("this shell has used up its node ids".to_owned());
        }
        let id = (epoch << EPOCH_SHIFT) | self.counter;
        self.tick.new_node(&self.mirror, id, class).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// The writable property `key` of `node`. A readonly property or an unknown key is refused.
    fn writable(&self, node: NodeId, key: &str) -> Result<PropSpec, String> {
        let class = self.class_of(node)?;
        let prop = self
            .prop(&class, key)
            .ok_or_else(|| format!("{class} has no property {key}"))?;
        if prop.readonly {
            return Err(format!("{class}.{key} is read-only"));
        }
        Ok(prop.clone())
    }

    /// A property as this tick sees it. `class` and `parent` are read from the node itself;
    /// an unset property answers with its schema default.
    fn get(&self, node: NodeId, key: &str) -> Result<Option<LuaValue>, String> {
        let record = self
            .tick
            .node(&self.mirror, node)
            .ok_or_else(|| format!("no node {node}"))?;
        match key {
            "class" => Ok(Some(LuaValue::Text(record.class.clone()))),
            "parent" => Ok(Some(LuaValue::Node(record.parent))),
            _ => {
                if let Some(value) = self.tick.get(&self.mirror, node, key) {
                    return Ok(Some(value.clone()));
                }
                Ok(self.prop(&record.class, key).and_then(|prop| prop.default.clone()))
            }
        }
    }

    /// The children of `node` when `key` names a `nodes` property; `None` for any other key.
    fn nodes_value(&self, node: NodeId, key: &str) -> Result<Option<Vec<NodeId>>, String> {
        let class = self.class_of(node)?;
        if self
            .prop(&class, key)
            .is_none_or(|prop| prop.kind != TreeValueKind::Nodes)
        {
            return Ok(None);
        }
        self.tick
            .children(&self.mirror, node)
            .map(|ids| Some(ids.to_vec()))
            .ok_or_else(|| format!("no node {node}"))
    }

    fn set(
        &mut self,
        node: NodeId,
        key: &str,
        value: LuaValue,
        seconds: Option<f64>,
        samples: Option<Vec<f64>>,
    ) -> Result<(), String> {
        self.tick
            .set(&self.mirror, node, key, value, seconds, samples)
            .map_err(|e| e.to_string())
    }

    fn insert(&mut self, node: NodeId, parent: NodeId, index: i64) -> Result<(), String> {
        self.tick
            .insert(&self.mirror, node, parent, index)
            .map_err(|e| e.to_string())
    }

    fn destroy(&mut self, node: NodeId) -> Result<(), String> {
        self.tick
            .destroy(&self.mirror, node)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn call(&mut self, node: NodeId, method: &str, args: Vec<u8>) -> Result<(), String> {
        let class = self.class_of(node)?;
        if !self.has_method(&class, method) {
            return Err(format!("{class} has no method {method}"));
        }
        self.tick
            .call(&self.mirror, node, method, args)
            .map_err(|e| e.to_string())
    }
}

fn lua_err(message: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::RuntimeError(message.to_string())
}

/// A Lua integer, or a float with no fraction, as an `i64`.
fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Integer(i) => Some(*i),
        Value::Number(f) if f.fract() == 0.0 && f.abs() < 9.0e18 => Some(*f as i64),
        _ => None,
    }
}

/// A Lua value as a property of `kind`, or the error the write is refused with.
fn from_lua(kind: TreeValueKind, value: Value) -> mlua::Result<LuaValue> {
    let wrong = || lua_err(format!("expected a {}", kind.name()));
    Ok(match kind {
        TreeValueKind::Bool => match value {
            Value::Boolean(b) => LuaValue::Bool(b),
            _ => return Err(wrong()),
        },
        TreeValueKind::Int => LuaValue::Int(integer(&value).ok_or_else(wrong)?),
        TreeValueKind::Uint => LuaValue::Uint(u64::try_from(integer(&value).ok_or_else(wrong)?).map_err(|_| wrong())?),
        TreeValueKind::Double => match value {
            Value::Number(f) => LuaValue::Double(f),
            Value::Integer(i) => LuaValue::Double(i as f64),
            _ => return Err(wrong()),
        },
        TreeValueKind::Text => match value {
            Value::String(s) => LuaValue::Text(s.to_str()?.to_string()),
            _ => return Err(wrong()),
        },
        TreeValueKind::Node => match value {
            Value::Nil => LuaValue::Node(NIL),
            other => LuaValue::Node(u64::try_from(integer(&other).ok_or_else(wrong)?).map_err(|_| wrong())?),
        },
        TreeValueKind::Nodes => return Err(wrong()),
        TreeValueKind::ListDouble => match value {
            Value::Table(table) => LuaValue::List(
                table
                    .sequence_values::<f64>()
                    .collect::<mlua::Result<Vec<f64>>>()
                    .map_err(|_| wrong())?,
            ),
            _ => return Err(wrong()),
        },
    })
}

/// A stored value as Lua sees it.
fn to_lua(lua: &Lua, value: &LuaValue) -> mlua::Result<Value> {
    Ok(match value {
        LuaValue::Bool(b) => Value::Boolean(*b),
        LuaValue::Int(i) => Value::Integer(*i),
        LuaValue::Uint(u) => Value::Integer(i64::try_from(*u).map_err(|_| lua_err("uint above i64::MAX"))?),
        LuaValue::Double(d) => Value::Number(*d),
        LuaValue::Text(s) => Value::String(lua.create_string(s)?),
        LuaValue::Node(NIL) => Value::Nil,
        LuaValue::Node(id) => Value::Integer(i64::try_from(*id).map_err(|_| lua_err("node id above i64::MAX"))?),
        LuaValue::List(items) => {
            let table = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                table.raw_set(index + 1, *item)?;
            }
            Value::Table(table)
        }
    })
}

/// The animation arguments of `core.set`: `seconds` a finite, non-negative number, and
/// `samples` at least two values in `[0, 1]` (design/node-core.md, section 6.1).
fn check_animation(seconds: Option<f64>, samples: Option<&[f64]>) -> Result<(), String> {
    if let Some(seconds) = seconds
        && !(seconds.is_finite() && seconds >= 0.0)
    {
        return Err(format!("seconds must be a finite, non-negative number, not {seconds}"));
    }
    if let Some(samples) = samples {
        if samples.len() < 2 {
            return Err("samples need at least two values".to_owned());
        }
        if samples.iter().any(|s| !(0.0..=1.0).contains(s)) {
            return Err("samples must lie in [0, 1]".to_owned());
        }
    }
    Ok(())
}

/// Build the `core` table over `core`. `node.lua` receives it as its argument.
pub fn install(lua: &Lua, core: &Shared) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    let shared = Rc::clone(core);
    table.set(
        "new",
        lua.create_function(move |_, class: String| {
            shared
                .borrow_mut()
                .new_node(&class)
                .map(|id| id as i64)
                .map_err(lua_err)
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "get",
        lua.create_function(move |lua, (node, key): (i64, String)| {
            let node = node_arg(node)?;
            let nodes = shared.borrow().nodes_value(node, &key).map_err(lua_err)?;
            if let Some(ids) = nodes {
                let ids = ids
                    .into_iter()
                    .map(|id| i64::try_from(id).map_err(lua_err))
                    .collect::<mlua::Result<Vec<i64>>>()?;
                return lua.create_sequence_from(ids).map(Value::Table);
            }
            let value = shared.borrow().get(node, &key).map_err(lua_err)?;
            value.map_or(Ok(Value::Nil), |value| to_lua(lua, &value))
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "set",
        lua.create_function(
            move |_, (node, key, value, seconds, samples): (i64, String, Value, Option<f64>, Option<Vec<f64>>)| {
                let node = node_arg(node)?;
                let prop = shared.borrow().writable(node, &key).map_err(lua_err)?;
                let value = from_lua(prop.kind, value)?;
                check_animation(seconds, samples.as_deref()).map_err(lua_err)?;
                shared
                    .borrow_mut()
                    .set(node, &key, value, seconds, samples)
                    .map_err(lua_err)
            },
        )?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "insert",
        lua.create_function(move |_, (parent, node, index): (i64, i64, Option<i64>)| {
            shared
                .borrow_mut()
                .insert(node_arg(node)?, node_arg(parent)?, index.unwrap_or(-1))
                .map_err(lua_err)
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "destroy",
        lua.create_function(move |_, node: i64| shared.borrow_mut().destroy(node_arg(node)?).map_err(lua_err))?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "call",
        lua.create_function(move |_, (node, method, args): (i64, String, Option<mlua::LuaString>)| {
            let args = args.map_or_else(Vec::new, |bytes| bytes.as_bytes().to_vec());
            shared
                .borrow_mut()
                .call(node_arg(node)?, &method, args)
                .map_err(lua_err)
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "root",
        lua.create_function(move |_, ()| {
            i64::try_from(shared.borrow().mirror().root()).map_err(|_| lua_err("root id above i64::MAX"))
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "classes",
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;
            let state = shared.borrow();
            for (index, class) in state.classes.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", class.name.as_str())?;
                if let Some(base) = &class.base {
                    entry.set("base", base.as_str())?;
                }
                let props = lua.create_table()?;
                for prop in &class.props {
                    let spec = lua.create_table()?;
                    spec.set("type", prop.kind.name())?;
                    spec.set("readonly", prop.readonly)?;
                    if let Some(default) = &prop.default {
                        spec.set("default", to_lua(lua, default)?)?;
                    }
                    props.set(prop.name.as_str(), spec)?;
                }
                entry.set("props", props)?;
                entry.set(
                    "signals",
                    lua.create_sequence_from(class.signals.iter().map(String::as_str))?,
                )?;
                entry.set(
                    "methods",
                    lua.create_sequence_from(class.methods.iter().map(String::as_str))?,
                )?;
                let slow = lua.create_table()?;
                for method in &class.slow {
                    slow.set(method.as_str(), true)?;
                }
                entry.set("slow", slow)?;
                list.raw_set(index + 1, entry)?;
            }
            Ok(list)
        })?,
    )?;

    let shared = Rc::clone(core);
    table.set(
        "report",
        lua.create_function(move |_, message: String| {
            shared.borrow_mut().reports.push(message);
            Ok(())
        })?,
    )?;

    Ok(table)
}

/// A Lua node id as the shell's `u64`. A negative id names no node.
fn node_arg(value: i64) -> mlua::Result<NodeId> {
    u64::try_from(value).map_err(|_| lua_err(format!("no node {value}")))
}
