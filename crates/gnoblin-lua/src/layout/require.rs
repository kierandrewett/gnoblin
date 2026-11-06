//! The loader behind a layout VM's `require`. It resolves layout modules only, and
//! only from the two fixed roots in [`LayoutRoots`]: `shipped` for `gnoblin.` names and
//! `user` (the config directory) for every other name. The roots are fixed, not taken
//! from the document being built, so a module is found before that document is
//! complete. A module name is a dot-separated list of identifiers, so it cannot name a
//! path outside its root; the canonical path is checked as well, so a symlink cannot
//! escape either.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Lua, RegistryKey, Value as LuaValue};

/// The largest module file the loader reads.
const MODULE_BYTES: u64 = 256 * 1024;

/// The two fixed directories a layout reference can name. Nothing else is searched, and
/// the roots do not come from the document being built, so a module is found before that
/// document is complete, and not from the last accepted store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutRoots {
    /// The installed `share/gnoblin/lua`. A name with the `gnoblin.` prefix loads from here.
    pub shipped: PathBuf,
    /// The directory of the configuration file being evaluated. Every other name loads
    /// from here. `None` when no configuration file is known, so such a name fails.
    pub user: Option<PathBuf>,
}

impl LayoutRoots {
    /// The directory a module name loads from: `shipped` for a `gnoblin.` name, `user`
    /// for any other.
    pub(crate) fn for_name(&self, name: &str) -> Result<&Path, String> {
        if name.starts_with("gnoblin.") {
            return Ok(&self.shipped);
        }
        self.user
            .as_deref()
            .ok_or_else(|| format!("module `{name}` has no user directory to load from"))
    }
}

/// The modules one VM has loaded, and the module currently loading, for cycle checks.
pub(crate) struct Modules {
    roots: LayoutRoots,
    loaded: RefCell<HashMap<String, Loaded>>,
    loading: RefCell<HashSet<String>>,
}

struct Loaded {
    value: RegistryKey,
    hash: u64,
}

impl Modules {
    pub(crate) fn new(roots: LayoutRoots) -> Self {
        Self {
            roots,
            loaded: RefCell::new(HashMap::new()),
            loading: RefCell::new(HashSet::new()),
        }
    }

    /// Return the value of module `name` and the FNV-1a hash of its source. The
    /// first successful load is cached for the life of this VM.
    pub(crate) fn fetch(&self, lua: &Lua, name: &str) -> mlua::Result<(LuaValue, u64)> {
        if let Some(entry) = self.loaded.borrow().get(name) {
            return Ok((lua.registry_value(&entry.value)?, entry.hash));
        }
        if !self.loading.borrow_mut().insert(name.to_owned()) {
            return Err(runtime(format!("layout module `{name}` requires itself")));
        }
        let loaded = self.load(lua, name);
        self.loading.borrow_mut().remove(name);
        let (value, hash) = loaded?;
        let key = lua.create_registry_value(value.clone())?;
        self.loaded
            .borrow_mut()
            .insert(name.to_owned(), Loaded { value: key, hash });
        Ok((value, hash))
    }

    /// Load module `name` again from its source, replacing the cached value. A layout
    /// is registered this way, so a changed module is the one that runs.
    pub(crate) fn reload(&self, lua: &Lua, name: &str) -> mlua::Result<(LuaValue, u64)> {
        self.loaded.borrow_mut().remove(name);
        self.fetch(lua, name)
    }

    fn load(&self, lua: &Lua, name: &str) -> mlua::Result<(LuaValue, u64)> {
        let path = resolve(&self.roots, name).map_err(runtime)?;
        let source = read_source(&path, name).map_err(runtime)?;
        let hash = fnv1a(source.as_bytes());
        // The chunk is named by its file, so a runtime error points at the source line.
        let value = match lua
            .load(source.as_str())
            .set_name(format!("@{}", path.display()))
            .call::<LuaValue>(())?
        {
            // A module that returns nothing is loaded as `true`, as in Lua's own `require`.
            LuaValue::Nil => LuaValue::Boolean(true),
            value => value,
        };
        Ok((value, hash))
    }
}

/// Install `require` into `lua`, resolving through `modules`.
pub(crate) fn install(lua: &Lua, modules: Rc<Modules>) -> mlua::Result<()> {
    let require = lua.create_function(move |lua, name: String| modules.fetch(lua, &name).map(|(value, _)| value))?;
    lua.globals().set("require", require)
}

/// Whether `name` is a dotted module name, every part an identifier.
pub(crate) fn is_module_name(name: &str) -> bool {
    name.split('.').all(is_component)
}

/// The file a module name stands for, read from the root its name selects. A name with
/// the `gnoblin.` prefix is read from the shipped root and any other name from the user
/// root, and the name is a path under that root: `gnoblin.layouts.grid` is
/// `gnoblin/layouts/grid.lua` under the shipped root. The canonical path is checked
/// against the root, so a symlink cannot lead out of it.
pub(crate) fn resolve(roots: &LayoutRoots, name: &str) -> Result<PathBuf, String> {
    if !name.split('.').all(is_component) {
        return Err(format!("`{name}` is not a layout module name"));
    }
    let dir = roots.for_name(name)?;
    let candidate = dir.join(format!("{}.lua", name.replace('.', "/")));
    if !candidate.is_file() {
        return Err(format!("module `{name}` is not found"));
    }
    let root = dir.canonicalize().map_err(|error| error.to_string())?;
    let real = candidate.canonicalize().map_err(|error| error.to_string())?;
    if !real.starts_with(&root) {
        return Err(format!("module `{name}` resolves outside its directory"));
    }
    Ok(real)
}

fn is_component(part: &str) -> bool {
    let mut chars = part.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn read_source(path: &Path, name: &str) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("module `{name}`: {error}"))?
        .len();
    if size > MODULE_BYTES {
        return Err(format!("module `{name}` is larger than {MODULE_BYTES} bytes"));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("module `{name}`: {error}"))?;
    String::from_utf8(bytes).map_err(|_| format!("module `{name}` is not UTF-8"))
}

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A runtime error with a message, as the VM reports it.
pub(crate) fn runtime(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

/// The plain message of a Lua error, without the error-kind prefix.
pub(crate) fn message(error: mlua::Error) -> String {
    match error {
        mlua::Error::RuntimeError(text) => text,
        other => other.to_string(),
    }
}
