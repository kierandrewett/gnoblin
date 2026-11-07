//! The loader behind `require`. A name is a dot-separated list of identifiers, and `a.b.c`
//! is the file `a/b/c.lua` under one of the two roots in [`ModuleRoots`]. A name that starts
//! with `gnoblin.` is read from the shipped root alone, so that namespace is reserved. Any
//! other name is read from the user root first and then from the shipped root. The roots are
//! fixed, not taken from the document being built, so a module is found before that document
//! is complete. The canonical path is checked against its root, so a symlink cannot escape.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Lua, RegistryKey, Value as LuaValue};

/// The largest module file the loader reads.
const MODULE_BYTES: u64 = 256 * 1024;

/// The two fixed directories a module name can resolve under. Nothing else is searched,
/// and the roots do not come from the document being built, so a module is found before
/// that document is complete, and not from the last accepted store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRoots {
    /// The installed `share/gnoblin/lua`. A `gnoblin.` name loads only from here.
    pub shipped: PathBuf,
    /// The directory of the configuration file being evaluated. Names other than
    /// `gnoblin.` names are searched here first. `None` when no configuration file is
    /// known, so such names search the shipped root alone.
    pub user: Option<PathBuf>,
}

/// The root a module was found under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Root {
    Shipped,
    User,
}

impl Root {
    /// The name a layout reference records for this root.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::User => "user",
        }
    }
}

/// A module name resolved to its file, and the root the file was found under.
pub(crate) struct Resolved {
    pub(crate) root: Root,
    pub(crate) path: PathBuf,
}

/// The modules one VM has loaded, and the module currently loading, for cycle checks.
pub(crate) struct Modules {
    roots: ModuleRoots,
    loaded: RefCell<HashMap<String, Loaded>>,
    loading: RefCell<HashSet<String>>,
}

struct Loaded {
    value: RegistryKey,
    hash: u64,
}

impl Modules {
    pub(crate) fn new(roots: ModuleRoots) -> Self {
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
            return Err(runtime(format!("module `{name}` requires itself")));
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
        let resolved = resolve(&self.roots, name).map_err(runtime)?;
        let source = read_source(&resolved.path, name).map_err(runtime)?;
        let hash = fnv1a(source.as_bytes());
        // The chunk is named by its file, so a runtime error points at the source line.
        let value = match lua
            .load(source.as_str())
            .set_name(format!("@{}", resolved.path.display()))
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

/// The file a module name stands for, and the root it was found under.
///
/// A `gnoblin.` name is looked up under the shipped root alone. Any other name is looked
/// up under the user root first, so a user file shadows a shipped one. The canonical path
/// is checked against the root it was found under, so a symlink cannot lead out of it.
pub(crate) fn resolve(roots: &ModuleRoots, name: &str) -> Result<Resolved, String> {
    if !is_module_name(name) {
        return Err(format!("`{name}` is not a module name"));
    }
    let reserved = name.starts_with("gnoblin.");
    let mut searched: Vec<(Root, &Path)> = Vec::new();
    if let Some(user) = roots.user.as_deref().filter(|_| !reserved) {
        searched.push((Root::User, user));
    }
    searched.push((Root::Shipped, roots.shipped.as_path()));
    let relative = format!("{}.lua", name.replace('.', "/"));
    for &(root, dir) in &searched {
        let candidate = dir.join(&relative);
        if !candidate.is_file() {
            continue;
        }
        let base = dir.canonicalize().map_err(|error| error.to_string())?;
        let path = candidate.canonicalize().map_err(|error| error.to_string())?;
        if !path.starts_with(&base) {
            return Err(format!("module `{name}` resolves outside its root `{}`", dir.display()));
        }
        return Ok(Resolved { root, path });
    }
    let places = searched
        .iter()
        .map(|(_, dir)| format!("`{}`", dir.display()))
        .collect::<Vec<_>>()
        .join(" then ");
    let reserved_note = if reserved {
        "; the `gnoblin.` namespace is reserved for shipped modules"
    } else {
        ""
    };
    Err(format!(
        "unknown module `{name}`: no extension namespace or module has that name{reserved_note}; searched {places}"
    ))
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
