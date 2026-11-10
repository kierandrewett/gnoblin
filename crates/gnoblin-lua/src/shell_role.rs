//! The shell VM as the runtime role runs it, when `GNOBLIN_SHELL_VM` names a directory.
//!
//! Opt-in and experimental. With the variable unset, nothing here runs and the role's loop
//! is the one it always was. With it set, the role connects a [`ShellVm`] to the owner
//! socket through a [`SocketTreeLink`] and runs one frame per turn of its loop. A frame
//! applies the compositor's change batches and sends the VM's writes as one transaction.
//! A link failure ends the VM with a named error; the role keeps serving without it.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use gnoblin_client::Client;
use gnoblin_client::unix::{UnixTransport, socket_path};

use crate::role::version;
use crate::shell_vm::{ShellVm, ShellVmError};
use crate::tree_core::{ClassSpec, tree_classes};
use crate::tree_link::SocketTreeLink;

/// The environment variable that turns the shell VM on, and names its Lua directory.
pub const SHELL_VM_VARIABLE: &str = "GNOBLIN_SHELL_VM";

/// The directory the shell VM loads, when the variable is set and not empty.
pub fn shell_vm_dir(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env(SHELL_VM_VARIABLE).filter(|dir| !dir.is_empty()).map(PathBuf::from)
}

/// A running shell VM, driven one frame per turn of the runtime loop.
pub struct ShellRole {
    vm: ShellVm,
    /// Every diagnostic the VM has reported since it booted, oldest first.
    diagnostics: Vec<String>,
}

impl ShellRole {
    pub fn new(vm: ShellVm) -> Self {
        Self {
            vm,
            diagnostics: Vec::new(),
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
}

/// The shell VM's slot in the runtime role: empty when the variable is unset or the VM ended.
pub struct ShellSlot {
    role: Option<ShellRole>,
}

impl ShellSlot {
    /// The slot for `env`. When the variable names a directory, `connect` boots the VM on it;
    /// a boot failure is logged, and the slot stays empty so the role still serves.
    pub fn from_env(
        env: &dyn Fn(&str) -> Option<String>,
        connect: impl FnOnce(&Path) -> Result<ShellRole, ShellVmError>,
    ) -> Self {
        let Some(dir) = shell_vm_dir(env) else {
            return Self { role: None };
        };
        match connect(&dir) {
            Ok(role) => Self { role: Some(role) },
            Err(error) => {
                crate::log::error!(
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

/// Boot a shell VM over the owner socket `stream`, with the tree's `classes`.
pub fn connect_shell(dir: &Path, stream: UnixStream, classes: Vec<ClassSpec>) -> Result<ShellRole, ShellVmError> {
    let link = SocketTreeLink::new(stream).map_err(|error| ShellVmError::Link(error.to_string()))?;
    Ok(ShellRole::new(ShellVm::new(Box::new(link), dir, classes)?))
}

/// Connect to the owner socket `env` names, and boot the shell VM over `dir` on it.
///
/// The path is the one every client reads ([`socket_path`]: `GNOBLIN_IPC_SOCKET`, else the
/// runtime directory's default socket). The handshake is the role's own: the version this
/// build serves, sent as a client. The compositor grants the owner connection by the peer's
/// credentials, so the session owner's second connection is the one that can attach the tree.
pub fn connect_owner_socket(env: &dyn Fn(&str) -> Option<String>, dir: &Path) -> Result<ShellRole, ShellVmError> {
    let path = socket_path(env).map_err(ShellVmError::Link)?;
    let transport = UnixTransport::connect(&path)
        .map_err(|error| ShellVmError::Link(format!("cannot connect to {}: {error}", path.display())))?;
    let client = Client::connect(transport, version()).map_err(|error| ShellVmError::Link(error.to_string()))?;
    let stream = client
        .into_inner()
        .into_stream()
        .map_err(|error| ShellVmError::Link(error.to_string()))?;
    connect_shell(dir, stream, tree_classes())
}
