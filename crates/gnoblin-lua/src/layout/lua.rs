//! The layout VM: its construction, the shape a layout module must have, the arrange
//! snapshot `ctx`, and the calls into a layout. Everything here runs on the worker
//! thread that owns the VM, because a Lua state is not `Send`.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use super::LayoutReason;
use gnoblin_lua_sandbox::{self as sandbox, Guard, Limits, Profile};
use gnoblin_wm::layout::{Arg, LayoutExtent, LayoutKind, Reply};
use mlua::{Function, Lua, RegistryKey, Table, Value as LuaValue};

use super::data::Data;
use super::reply;
use super::require::{self, LayoutRoots, Modules, message, runtime};
use super::snapshot::{LayoutSnapshot, SnapshotWindow};

/// Why a VM call failed: the reason the host reports, and the Lua message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayoutFault {
    pub(crate) reason: LayoutReason,
    pub(crate) message: String,
}

impl LayoutFault {
    pub(crate) fn new(reason: impl Into<LayoutReason>, message: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            message: message.into(),
        }
    }
}

/// What a layout module declares about itself. It is read once, when the module
/// loads, and the wm's layout contract is answered from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayoutManifest {
    pub(crate) kind: LayoutKind,
    pub(crate) extent: LayoutExtent,
    /// The standard commands the layout's `command` table names.
    pub(crate) commands: BTreeSet<String>,
}

/// What the host asks a layout to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayoutCall {
    /// The batch pass: `arrange(ctx)`.
    Arrange,
    /// One geometry request: `on_request(ctx)`, which answers for the whole tiled set.
    Request,
    /// A named command: `command[name](ctx, args)`, which answers for the whole tiled set.
    /// A layout without that command declines.
    Command { name: String, args: BTreeMap<String, Arg> },
}

/// One layout as the VM holds it.
struct LayoutEntry {
    value: RegistryKey,
    hash: u64,
    /// The layout's option defaults, with the user's values laid over them.
    options: Data,
}

/// What [`Vm::inspect`] found in a layout module: its default options, its manifest
/// and the FNV-1a hash of its source.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Inspected {
    pub(crate) options: Data,
    pub(crate) manifest: LayoutManifest,
    pub(crate) hash: u64,
}

/// The result of one layout call: the reply, the module hash it ran under, and the
/// `ctx.state` the layout left behind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ran {
    pub(crate) reply: Reply,
    pub(crate) hash: u64,
    pub(crate) state: Data,
}

/// A layout VM. Its layouts are registered by name and survive across calls.
pub(crate) struct Vm {
    lua: Lua,
    guard: Rc<Guard>,
    limits: Limits,
    modules: Rc<Modules>,
    layouts: BTreeMap<String, LayoutEntry>,
}

impl Vm {
    pub(crate) fn new(limits: Limits, roots: LayoutRoots) -> mlua::Result<Self> {
        let guard = Rc::new(Guard::new());
        let lua = sandbox::create(Profile::Layout, &limits, guard.clone(), crate::host::script_sink())?;
        let modules = Rc::new(Modules::new(roots));
        require::install(&lua, modules.clone())?;
        Ok(Self {
            lua,
            guard,
            limits,
            modules,
            layouts: BTreeMap::new(),
        })
    }

    /// Load `module` afresh and register it as layout `name` with the user's
    /// `options`. Returns the manifest the module declares.
    pub(crate) fn load(&mut self, name: &str, module: &str, options: &Data) -> Result<LayoutManifest, LayoutFault> {
        self.guard.begin(&self.limits);
        let (value, hash) = self
            .modules
            .reload(&self.lua, module)
            .map_err(|error| self.fault(error, LayoutReason::MissingModule))?;
        let (defaults, manifest) = shape(&value).map_err(|error| self.fault(error, LayoutReason::Error))?;
        let options = merge(defaults, options)?;
        let value = self
            .lua
            .create_registry_value(value)
            .map_err(|error| self.fault(error, LayoutReason::Error))?;
        self.layouts
            .insert(name.to_owned(), LayoutEntry { value, hash, options });
        Ok(manifest)
    }

    /// Read module `module` and report what it declares, without registering a layout.
    ///
    /// A configuration file calls this through a layout reference, so the module is
    /// checked where it is named. The module is cached in this VM, so a later `load`
    /// of the same name does not read the file again.
    pub(crate) fn inspect(&mut self, module: &str) -> Result<Inspected, LayoutFault> {
        self.guard.begin(&self.limits);
        let (value, hash) = self
            .modules
            .fetch(&self.lua, module)
            .map_err(|error| self.fault(error, LayoutReason::MissingModule))?;
        let (options, manifest) = shape(&value).map_err(|error| self.fault(error, LayoutReason::Error))?;
        Ok(Inspected {
            options,
            manifest,
            hash,
        })
    }

    /// Run `call` on layout `name`. `state` is the persisted `ctx.state` and the module
    /// hash it belongs to; it is used only when that hash is still the one loaded.
    pub(crate) fn run(
        &self,
        name: &str,
        call: &LayoutCall,
        snapshot: &LayoutSnapshot,
        state: Option<(u64, Data)>,
    ) -> Result<Ran, LayoutFault> {
        let entry = self
            .layouts
            .get(name)
            .ok_or_else(|| LayoutFault::new(LayoutReason::MissingModule, format!("layout `{name}` did not load")))?;
        let state = match state {
            Some((hash, state)) if hash == entry.hash => state,
            _ => Data::table(),
        };
        let layout: LuaValue = self
            .lua
            .registry_value(&entry.value)
            .map_err(|error| self.fault(error, LayoutReason::Error))?;
        let handler = handler(&layout, call).map_err(|error| self.fault(error, LayoutReason::Error))?;
        let ctx = build_ctx(&self.lua, snapshot, &entry.options, &state)
            .map_err(|error| self.fault(error, LayoutReason::Error))?;
        let reply = match handler {
            None => Reply::Declined,
            Some(handler) => {
                self.guard.begin(&self.limits);
                let value =
                    invoke(&handler, call, &ctx, &self.lua).map_err(|error| self.fault(error, LayoutReason::Error))?;
                reply::read(&value).map_err(|error| self.fault(error, LayoutReason::Error))?
            }
        };
        let state: LuaValue = ctx
            .get("state")
            .map_err(|error| self.fault(error, LayoutReason::Error))?;
        let state = Data::from_lua(&state).map_err(|error| LayoutFault::new(LayoutReason::Error, error.0))?;
        Ok(Ran {
            reply,
            hash: entry.hash,
            state,
        })
    }

    /// Classify a Lua error. A budget the hook stopped is a stall; a memory error is a
    /// memory failure; anything else is `default`.
    fn fault(&self, error: mlua::Error, default: LayoutReason) -> LayoutFault {
        let failure = if self.guard.aborted() {
            LayoutReason::Stalled
        } else {
            classify(&error, default)
        };
        LayoutFault::new(failure, message(error))
    }
}

fn classify(error: &mlua::Error, default: LayoutReason) -> LayoutReason {
    match error {
        mlua::Error::MemoryError(_) => LayoutReason::Memory,
        mlua::Error::CallbackError { cause, .. } => classify(cause, default),
        _ => default,
    }
}

/// Check that a module declares a layout, and read its manifest and option defaults.
///
/// A layout is a table with an `arrange` function, a `kind` (`floating`, `tiling` or
/// `kiosk`) and optionally an `extent` (`output`, the default, or `unbounded`, which only a
/// `tiling` layout may use). It may also have `on_request`, `on_added`, `on_removed`,
/// `command` (a table of functions) and `options` (plain data). A bare function declares
/// no kind, so it is refused.
fn shape(value: &LuaValue) -> mlua::Result<(Data, LayoutManifest)> {
    let LuaValue::Table(table) = value else {
        return Err(runtime(format!(
            "a layout must be a table declaring its `kind`, not {}",
            value.type_name()
        )));
    };
    if table.get::<Option<Function>>("arrange")?.is_none() {
        return Err(runtime("a layout table needs an `arrange` function"));
    }
    let kind = match table.get::<Option<String>>("kind")? {
        Some(name) => LayoutKind::from_name(&name)
            .ok_or_else(|| runtime(format!("layout kind `{name}` must be floating, tiling or kiosk")))?,
        None => return Err(runtime("a layout table needs a `kind`: floating, tiling or kiosk")),
    };
    let extent = match table.get::<Option<String>>("extent")? {
        Some(name) => LayoutExtent::from_name(&name)
            .ok_or_else(|| runtime(format!("layout extent `{name}` must be output or unbounded")))?,
        None => LayoutExtent::default(),
    };
    if !kind.allows_extent(extent) {
        return Err(runtime("layout extent `unbounded` needs kind `tiling`"));
    }
    for key in ["on_request", "on_added", "on_removed"] {
        match table.get::<Option<LuaValue>>(key)? {
            None | Some(LuaValue::Function(_)) => {}
            Some(_) => return Err(runtime(format!("layout `{key}` must be a function"))),
        }
    }
    let mut commands = BTreeSet::new();
    if let Some(table_commands) = table.get::<Option<Table>>("command")? {
        for pair in table_commands.pairs::<LuaValue, LuaValue>() {
            let (key, value) = pair?;
            let LuaValue::String(name) = &key else {
                return Err(runtime("layout commands are named by strings"));
            };
            if !matches!(value, LuaValue::Function(_)) {
                return Err(runtime(format!(
                    "layout command `{}` must be a function",
                    key_name(&key)
                )));
            }
            commands.insert(name.to_string_lossy());
        }
    }
    let options = match table.get::<Option<LuaValue>>("options")? {
        None => Data::table(),
        Some(options @ LuaValue::Table(_)) => Data::from_lua(&options).map_err(|error| runtime(error.0))?,
        Some(_) => return Err(runtime("layout `options` must be a table")),
    };
    Ok((options, LayoutManifest { kind, extent, commands }))
}

fn key_name(key: &LuaValue) -> String {
    match key {
        LuaValue::String(text) => text.to_string_lossy(),
        other => other.type_name().to_owned(),
    }
}

/// The user's option values laid over the layout's defaults, one key at a time.
fn merge(defaults: Data, values: &Data) -> Result<Data, LayoutFault> {
    let mut merged = defaults;
    let (Data::Table(base), Data::Table(user)) = (&mut merged, values) else {
        return Err(LayoutFault::new(LayoutReason::Error, "layout options must be a table"));
    };
    for (key, value) in user {
        base.insert(key.clone(), value.clone());
    }
    Ok(merged)
}

/// The function that answers `call`, or `None` when the layout does not handle it.
fn handler(layout: &LuaValue, call: &LayoutCall) -> mlua::Result<Option<Function>> {
    match layout {
        LuaValue::Table(table) => match call {
            LayoutCall::Arrange => table.get::<Option<Function>>("arrange"),
            LayoutCall::Request => first(table, &["on_request"]),
            LayoutCall::Command { name, .. } => match table.get::<Option<Table>>("command")? {
                Some(commands) => commands.get::<Option<Function>>(name.as_str()),
                None => Ok(None),
            },
        },
        other => Err(runtime(format!("the layout is a {}, not a table", other.type_name()))),
    }
}

fn first(table: &Table, keys: &[&str]) -> mlua::Result<Option<Function>> {
    for key in keys {
        if let Some(function) = table.get::<Option<Function>>(*key)? {
            return Ok(Some(function));
        }
    }
    Ok(None)
}

fn invoke(handler: &Function, call: &LayoutCall, ctx: &Table, lua: &Lua) -> mlua::Result<LuaValue> {
    let ctx = ctx.clone();
    match call {
        LayoutCall::Arrange | LayoutCall::Request => handler.call::<LuaValue>((ctx,)),
        LayoutCall::Command { args, .. } => {
            let table = lua.create_table()?;
            for (key, value) in args {
                match value {
                    Arg::Number(number) => table.set(key.as_str(), *number)?,
                    Arg::Text(text) => table.set(key.as_str(), text.as_str())?,
                    Arg::Bool(flag) => table.set(key.as_str(), *flag)?,
                }
            }
            handler.call::<LuaValue>((ctx, table))
        }
    }
}

/// The `ctx` a layout is called with. It is rebuilt for every call; only `ctx.state`
/// is read back after the call.
fn build_ctx(lua: &Lua, snapshot: &LayoutSnapshot, options: &Data, state: &Data) -> mlua::Result<Table> {
    let ctx = lua.create_table()?;

    let area = lua.create_table()?;
    area.set("x", snapshot.area.x)?;
    area.set("y", snapshot.area.y)?;
    area.set("width", snapshot.area.width)?;
    area.set("height", snapshot.area.height)?;
    ctx.set("area", area)?;

    let output = lua.create_table()?;
    output.set("id", lua_int(snapshot.output.id.get())?)?;
    output.set("name", snapshot.output.name.as_str())?;
    output.set("scale", snapshot.output.scale.as_f64())?;
    ctx.set("output", output)?;

    let windows = lua.create_table()?;
    for (index, window) in snapshot.windows.iter().enumerate() {
        windows.raw_set(index + 1, window_table(lua, window)?)?;
    }
    ctx.set("windows", windows)?;

    let focus = snapshot.focus.map(|id| lua_int(id.get())).transpose()?;
    ctx.set("focus", focus)?;

    // The options come first, so a user option cannot shadow a gap.
    let LuaValue::Table(params) = options.to_lua(lua)? else {
        return Err(runtime("layout options are not a table"));
    };
    params.set("inner", snapshot.gaps.inner)?;
    params.set("outer", snapshot.gaps.outer)?;
    params.set("border", snapshot.gaps.border)?;
    ctx.set("params", params)?;

    ctx.set("state", state.to_lua(lua)?)?;
    Ok(ctx)
}

fn window_table(lua: &Lua, window: &SnapshotWindow) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", lua_int(window.id.get())?)?;
    table.set("app_id", window.app_id.as_str())?;
    table.set("title", window.title.as_str())?;
    table.set("width", window.width)?;
    table.set("height", window.height)?;
    table.set("state", "tiled")?;
    let hints = lua.create_table()?;
    hints.set("min_width", window.size_hints.min_width)?;
    hints.set("min_height", window.size_hints.min_height)?;
    hints.set("max_width", window.size_hints.max_width)?;
    hints.set("max_height", window.size_hints.max_height)?;
    table.set("size_hints", hints)?;
    Ok(table)
}

fn lua_int(id: u64) -> mlua::Result<i64> {
    i64::try_from(id).map_err(|_| runtime(format!("id {id} is too large for a layout")))
}
