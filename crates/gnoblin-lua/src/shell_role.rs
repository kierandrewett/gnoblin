//! The shell VM as the runtime role runs it.
//!
//! On by default when the stock Lua library is installed at `<prefix>/share/gnoblin/lua/gnoblin`
//! (`GNOBLIN_PREFIX`, default `/usr`). `GNOBLIN_SHELL_VM=0` turns it off; any other non-empty
//! value names the directory the VM loads instead. With no installed library and the variable
//! unset, nothing here runs and the role's loop is the one it always was. When the VM runs, the
//! role connects a [`ShellVm`] to the owner socket through a [`SocketTreeLink`] and runs one frame
//! per turn of its loop. A frame applies the compositor's change batches and sends the VM's writes
//! as one transaction. A failure to start, or a link failure, logs one warning and the role keeps
//! serving without the VM.

use std::ffi::OsStr;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use gnoblin_client::Client;
use gnoblin_client::unix::{UnixTransport, socket_path};

use crate::module_dir::{default_prefix, installed_module_dir};
use crate::role::version;
use crate::shell_vm::{ShellLimits, ShellVm, ShellVmError};
use crate::tree_core::{ClassSpec, tree_classes};
use crate::tree_link::SocketTreeLink;

/// The environment variable that names the shell VM's Lua directory, or turns the VM off with `0`.
pub const SHELL_VM_VARIABLE: &str = "GNOBLIN_SHELL_VM";

/// The value of [`SHELL_VM_VARIABLE`] that turns the shell VM off.
pub const SHELL_VM_OFF: &str = "0";

/// Where the installed stock Lua library lives: `<prefix>/share/gnoblin/lua/gnoblin`, for
/// `GNOBLIN_PREFIX` as `env` gives it, defaulting to `/usr`.
pub fn installed_stock_dir(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let prefix = env("GNOBLIN_PREFIX");
    installed_module_dir(&default_prefix(prefix.as_deref().map(OsStr::new))).join("gnoblin")
}

/// The directory the shell VM loads, or `None` when it should not run.
///
/// `GNOBLIN_SHELL_VM=0` is off. Any other non-empty value is the directory. Unset or empty means
/// the installed stock library at `installed`, and only when that library is there (it has
/// `node.lua`); otherwise the VM stays off.
pub fn shell_vm_dir(env: &dyn Fn(&str) -> Option<String>, installed: &Path) -> Option<PathBuf> {
    match env(SHELL_VM_VARIABLE) {
        Some(value) if value == SHELL_VM_OFF => None,
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => installed.join("node.lua").is_file().then(|| installed.to_path_buf()),
    }
}

/// A running shell VM, driven one frame per turn of the runtime loop.
pub struct ShellRole {
    vm: ShellVm,
    /// Every diagnostic the VM has reported since it booted, oldest first.
    diagnostics: Vec<String>,
    /// The errors the user's shell files raised when they loaded.
    user_errors: Vec<String>,
}

impl ShellRole {
    pub fn new(vm: ShellVm) -> Self {
        Self {
            vm,
            diagnostics: Vec::new(),
            user_errors: Vec::new(),
        }
    }

    /// Run one frame at `now` (seconds on the frame clock). The frame applies any change
    /// batch the compositor has sent, and sends the VM's writes as one transaction when
    /// there are some. The frame is also where `node.lua` gets its wake time, so the loop
    /// calls this every turn and the VM decides whether it has anything to do.
    pub fn tick(&mut self, now: f64) -> Result<(), ShellVmError> {
        let frame = self.vm.frame(now, Vec::new())?;
        for diagnostic in &frame.diagnostics {
            crate::log::warn!(area = "shell-vm", "{diagnostic}");
        }
        self.diagnostics.extend(frame.diagnostics);
        Ok(())
    }

    /// Every diagnostic the VM has reported since it booted, oldest first.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// Run a Lua chunk inside the VM, as a named chunk.
    pub fn run(&mut self, name: &str, src: &str) -> Result<Option<String>, ShellVmError> {
        self.vm.run(name, src)
    }

    /// Loads the stock shell and the user's shell files into the VM (see [`ShellVm::load_shell`]). The errors the
    /// user's files raised are kept for [`ShellRole::user_file_errors`].
    pub fn load_shell(&mut self, stock: &Path, user: Option<&Path>) -> Result<Vec<String>, ShellVmError> {
        let errors = self.vm.load_shell(stock, user)?;
        self.user_errors.clone_from(&errors);
        Ok(errors)
    }

    /// The errors the user's shell files raised when they loaded, in load order.
    pub fn user_file_errors(&self) -> &[String] {
        &self.user_errors
    }
}

/// The shell VM's slot in the runtime role: empty when the VM is off or ended.
pub struct ShellSlot {
    role: Option<ShellRole>,
}

impl ShellSlot {
    /// The slot for `env`. `installed` is the stock Lua directory the VM runs from by default.
    /// `connect` boots the VM on the directory `env` and `installed` select; a boot failure is
    /// logged as a warning, and the slot stays empty so the role still serves.
    pub fn from_env(
        env: &dyn Fn(&str) -> Option<String>,
        installed: &Path,
        connect: impl FnOnce(&Path) -> Result<ShellRole, ShellVmError>,
    ) -> Self {
        let Some(dir) = shell_vm_dir(env, installed) else {
            return Self { role: None };
        };
        match connect(&dir) {
            Ok(role) => Self { role: Some(role) },
            Err(error) => {
                crate::log::warn!(
                    area = "shell-vm",
                    "shell VM did not start from {}: {error:?}",
                    dir.display()
                );
                Self { role: None }
            }
        }
    }

    pub fn running(role: ShellRole) -> Self {
        Self { role: Some(role) }
    }

    pub fn is_running(&self) -> bool {
        self.role.is_some()
    }

    /// Run one frame, when there is a VM. A failure ends the VM: the slot is emptied, the
    /// error comes back named, and the next tick is a no-op.
    pub fn tick(&mut self, now: f64) -> Result<(), ShellVmError> {
        let Some(role) = self.role.as_mut() else {
            return Ok(());
        };
        let result = role.tick(now);
        if result.is_err() {
            self.role = None;
        }
        result
    }
}

/// Boot a shell VM over the owner socket `stream`, with the tree's `classes` and the budgets `limits`.
pub fn connect_shell(
    dir: &Path,
    stream: UnixStream,
    classes: Vec<ClassSpec>,
    limits: ShellLimits,
) -> Result<ShellRole, ShellVmError> {
    let link = SocketTreeLink::new(stream).map_err(|error| ShellVmError::Link(error.to_string()))?;
    Ok(ShellRole::new(ShellVm::with_limits(
        Box::new(link),
        dir,
        classes,
        limits,
    )?))
}

/// Connect to the owner socket `env` names, and boot the shell VM over `dir` on it, under the default budgets.
///
/// The path is the one every client reads ([`socket_path`]: `GNOBLIN_IPC_SOCKET`, else the
/// runtime directory's default socket). The handshake is the role's own: the version this
/// build serves, sent as a client. The compositor grants the owner connection by the peer's
/// credentials, so the session owner's second connection is the one that can attach the tree.
pub fn connect_owner_socket(env: &dyn Fn(&str) -> Option<String>, dir: &Path) -> Result<ShellRole, ShellVmError> {
    connect_owner_socket_with_limits(env, dir, ShellLimits::default())
}

/// Connects like [`connect_owner_socket`], under the budgets `limits` (see [`ShellLimits`] for what each bounds).
///
/// The production entry takes the default budgets, and loading the stock shell and the user's files has its own,
/// wide `load` budget, so a slow machine does not fail the start. A caller narrows or widens a budget only for a
/// test of that budget, or for one that stresses handlers on a machine it knows is busy.
pub fn connect_owner_socket_with_limits(
    env: &dyn Fn(&str) -> Option<String>,
    dir: &Path,
    limits: ShellLimits,
) -> Result<ShellRole, ShellVmError> {
    let path = socket_path(env).map_err(ShellVmError::Link)?;
    let transport = UnixTransport::connect(&path)
        .map_err(|error| ShellVmError::Link(format!("cannot connect to {}: {error}", path.display())))?;
    let client = Client::connect(transport, version()).map_err(|error| ShellVmError::Link(error.to_string()))?;
    let stream = client
        .into_inner()
        .into_stream()
        .map_err(|error| ShellVmError::Link(error.to_string()))?;
    connect_shell(dir, stream, tree_classes(), limits)
}

/// What the runtime role runs: [`connect_owner_socket`], then the stock shell and the user's shell files.
///
/// `user` is the config directory the user's `shell.lua` and `shell/*.lua` live in. An error loading the stock
/// shell fails the connect, so the slot stays empty and the role serves without a shell VM.
pub fn connect_session_shell(
    env: &dyn Fn(&str) -> Option<String>,
    dir: &Path,
    user: Option<&Path>,
) -> Result<ShellRole, ShellVmError> {
    connect_session_shell_with_limits(env, dir, user, ShellLimits::default())
}

/// Connects like [`connect_session_shell`], under the budgets `limits`.
pub fn connect_session_shell_with_limits(
    env: &dyn Fn(&str) -> Option<String>,
    dir: &Path,
    user: Option<&Path>,
    limits: ShellLimits,
) -> Result<ShellRole, ShellVmError> {
    let mut role = connect_owner_socket_with_limits(env, dir, limits)?;
    role.load_shell(dir, user)?;
    Ok(role)
}
