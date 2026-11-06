//! Layout references for a configuration VM.
//!
//! `require("gnoblin.layouts.grid")` in a configuration file returns a layout
//! reference, not a layout. The reference names the module and carries what the
//! module declares: its source hash and its default options. The module is read and
//! checked in a sandboxed layout VM, so a broken layout fails at `require` time, in
//! the configuration that names it.
//!
//! One layout VM serves a whole configuration evaluation. It is created on the first
//! layout `require` and dropped when the next evaluation resets the loader, so a
//! changed module on disk is read again by the next evaluation and never served from
//! a stale cache. Within one evaluation a module is read once and then reused.

use mlua::{UserData, UserDataFields, UserDataMethods, Value as LuaValue};

use super::data::{Data, LayoutKey};
use super::lua::Vm;
use super::require::{LayoutRoots, is_module_name, resolve, runtime};
use gnoblin_lua_sandbox::Limits;

/// A layout module named by a configuration file.
///
/// Scripts see it as a userdata with read-only fields. `root` says which fixed
/// its source as sixteen lowercase hex digits, and `options` are its declared defaults.
#[derive(Clone, Debug)]
pub(crate) struct LayoutRef {
    /// `shipped` for a `gnoblin.` name, `user` for any other.
    pub(crate) root: &'static str,
    pub(crate) module: String,
    pub(crate) hash: u64,
    pub(crate) options: Data,
}

impl LayoutRef {
    /// A copy of this reference with some option values replaced. Only options the
    /// module declares may be set: a name it does not declare is an error, so a typo
    /// fails in the configuration that makes it. The merge is shallow: a value given
    /// for a declared option replaces the whole declared value.
    pub(crate) fn with(&self, overrides: &LuaValue) -> Result<Self, String> {
        let Data::Table(overrides) = Data::from_lua(overrides).map_err(|error| error.0)? else {
            return Err(format!("options for layout `{}` must be a table", self.module));
        };
        let Data::Table(declared) = &self.options else {
            return Err(format!("layout `{}` declares no options", self.module));
        };
        let mut merged = declared.clone();
        for (key, value) in overrides {
            if !declared.contains_key(&key) {
                return Err(format!("layout `{}` has no option `{}`", self.module, key_name(&key)));
            }
            merged.insert(key, value);
        }
        Ok(Self {
            root: self.root,
            module: self.module.clone(),
            hash: self.hash,
            options: Data::Table(merged),
        })
    }
}

/// A table key as the name a configuration writes it with.
fn key_name(key: &LayoutKey) -> String {
    match key {
        LayoutKey::String(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

impl UserData for LayoutRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("root", |_, this| Ok(this.root));
        fields.add_field_method_get("module", |_, this| Ok(this.module.clone()));
        // A string, because a 64-bit hash does not survive a round trip through a Lua number.
        // Sixteen lowercase hex digits, the same spelling the configuration writes.
        fields.add_field_method_get("hash", |_, this| Ok(format!("{:016x}", this.hash)));
        fields.add_field_method_get("options", |lua, this| this.options.to_lua(lua));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // `ref:with(options)` returns a new reference; the original is unchanged.
        methods.add_method("with", |_, this, options: LuaValue| {
            this.with(&options).map_err(runtime)
        });
    }
}

/// Builds layout references for one configuration VM.
pub(crate) struct LayoutModuleLoader {
    limits: Limits,
    roots: Option<LayoutRoots>,
    vm: Option<Vm>,
}

impl LayoutModuleLoader {
    /// A loader with no roots yet; every reference fails until [`LayoutModuleLoader::reset`] runs.
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            roots: None,
            vm: None,
        }
    }

    /// Start a new configuration evaluation with `roots`. The shared layout VM is
    /// dropped here, so the evaluation starts from the files as they are now.
    pub(crate) fn reset(&mut self, roots: LayoutRoots) {
        self.roots = Some(roots);
        self.vm = None;
    }

    /// The reference for layout module `name`, reading and checking it on first use.
    ///
    /// A name that is not a layout module under either root is reported as an unknown
    /// module, the same message an unloaded extension namespace gives.
    pub(crate) fn reference(&mut self, name: &str) -> Result<LayoutRef, String> {
        let unknown = || format!("unknown module `{name}`: no extension namespace or layout module has that name");
        let roots = self.roots.clone().ok_or_else(unknown)?;
        if !is_module_name(name) || resolve(&roots, name).is_err() {
            return Err(unknown());
        }
        let root = if name.starts_with("gnoblin.") {
            "shipped"
        } else {
            "user"
        };
        if self.vm.is_none() {
            let vm = Vm::new(self.limits.clone(), roots).map_err(|error| error.to_string())?;
            self.vm = Some(vm);
        }
        let vm = self.vm.as_mut().ok_or_else(unknown)?;
        let inspected = vm.inspect(name).map_err(|fault| fault.message)?;
        Ok(LayoutRef {
            root,
            module: name.to_owned(),
            hash: inspected.hash,
            options: inspected.options,
        })
    }
}
