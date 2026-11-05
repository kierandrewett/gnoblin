//! Plain data that crosses the host and VM boundary: the persisted `ctx.state` and
//! the layout's option values. A Lua value is read and built only on the worker
//! thread that owns its VM, so the host can hold, clone and compare these freely.

use std::collections::BTreeMap;

use mlua::{Lua, Value as LuaValue};

/// The deepest table nesting a state may have. A self-referential table exceeds it.
const MAX_DEPTH: usize = 32;

/// The most table entries one value may hold. Shared sub-tables would otherwise
/// make the walk grow exponentially with depth.
const MAX_ENTRIES: usize = 65_536;

/// A plain value a layout may persist: no functions, userdata, threads or cycles.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// A boolean.
    Boolean(bool),
    /// A Lua integer.
    Integer(i64),
    /// A Lua float.
    Number(f64),
    /// A string.
    String(String),
    /// A table, ordered by key so that two equal tables compare equal.
    Table(BTreeMap<LayoutKey, Data>),
}

/// A table key. A float key is kept by its bit pattern, because `f64` is not `Ord`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayoutKey {
    /// A boolean key.
    Boolean(bool),
    /// An integer key.
    Integer(i64),
    /// A float key, as `f64::to_bits`.
    Number(u64),
    /// A string key.
    String(String),
}

/// Why a Lua value is not plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataError(pub String);

impl Data {
    /// An empty table.
    pub fn table() -> Self {
        Data::Table(BTreeMap::new())
    }

    /// Read a Lua value as plain data. A function, userdata, thread, a cycle, or a
    /// table nested or sized past the limits is refused.
    pub fn from_lua(value: &LuaValue) -> Result<Self, DataError> {
        let mut budget = MAX_ENTRIES;
        read(value, 0, &mut budget)
    }

    /// Build the Lua value for this data inside `lua`.
    pub(crate) fn to_lua(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        Ok(match self {
            Data::Boolean(value) => LuaValue::Boolean(*value),
            Data::Integer(value) => LuaValue::Integer(*value),
            Data::Number(value) => LuaValue::Number(*value),
            Data::String(value) => LuaValue::String(lua.create_string(value)?),
            Data::Table(map) => {
                let table = lua.create_table()?;
                for (key, value) in map {
                    table.raw_set(key.to_lua(lua)?, value.to_lua(lua)?)?;
                }
                LuaValue::Table(table)
            }
        })
    }
}

impl LayoutKey {
    fn to_lua(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        Ok(match self {
            LayoutKey::Boolean(value) => LuaValue::Boolean(*value),
            LayoutKey::Integer(value) => LuaValue::Integer(*value),
            LayoutKey::Number(bits) => LuaValue::Number(f64::from_bits(*bits)),
            LayoutKey::String(value) => LuaValue::String(lua.create_string(value)?),
        })
    }
}

fn read(value: &LuaValue, depth: usize, budget: &mut usize) -> Result<Data, DataError> {
    Ok(match value {
        LuaValue::Boolean(value) => Data::Boolean(*value),
        LuaValue::Integer(value) => Data::Integer(*value),
        LuaValue::Number(value) => Data::Number(*value),
        LuaValue::String(value) => Data::String(text(value)?),
        LuaValue::Table(table) => {
            if depth >= MAX_DEPTH {
                return Err(DataError("state nests too deeply, or refers to itself".into()));
            }
            let mut map = BTreeMap::new();
            for pair in table.pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair.map_err(|error| DataError(error.to_string()))?;
                *budget = budget
                    .checked_sub(1)
                    .ok_or_else(|| DataError("state holds too many entries".into()))?;
                map.insert(read_key(&key)?, read(&value, depth + 1, budget)?);
            }
            Data::Table(map)
        }
        other => return Err(DataError(format!("state holds a {} value", other.type_name()))),
    })
}

fn read_key(key: &LuaValue) -> Result<LayoutKey, DataError> {
    Ok(match key {
        LuaValue::Boolean(value) => LayoutKey::Boolean(*value),
        LuaValue::Integer(value) => LayoutKey::Integer(*value),
        LuaValue::Number(value) => LayoutKey::Number(value.to_bits()),
        LuaValue::String(value) => LayoutKey::String(text(value)?),
        other => return Err(DataError(format!("state key is a {} value", other.type_name()))),
    })
}

fn text(value: &mlua::LuaString) -> Result<String, DataError> {
    let borrowed = value.to_str().map_err(|error| DataError(error.to_string()))?;
    Ok(borrowed.to_string())
}
