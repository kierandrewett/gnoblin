//! The runtime role's shell VM wiring: with no installed stock library and the variable unset,
//! nothing changes; `GNOBLIN_SHELL_VM=0` turns it off even when the library is installed; a set
//! variable that cannot be wired is refused; a failed connect leaves the role serving.

use std::path::Path;

use gnoblin_lua::shell_role::{SHELL_VM_VARIABLE, ShellSlot};
use gnoblin_lua::shell_vm::ShellVmError;

/// A stock directory that is not installed: no `node.lua` is there.
const ABSENT_STOCK: &str = "/nonexistent/gnoblin-stock-for-test";

#[test]
fn unset_variable_with_no_installed_library_never_connects() {
    let mut connected = false;
    let mut slot = ShellSlot::from_env(&|_: &str| None, Path::new(ABSENT_STOCK), |_: &Path| {
        connected = true;
        Err(ShellVmError::Link("must not run".to_owned()))
    });
    assert!(
        !slot.is_running(),
        "no installed library and no variable must not start a shell VM"
    );
    assert!(slot.tick(0.0).is_ok(), "an empty slot ticks as a no-op");
    assert!(!connected, "the connector must not run without a shell VM directory");
}

#[test]
fn the_variable_zero_turns_the_shell_vm_off_even_with_an_installed_library() {
    let installed = std::env::temp_dir().join("gnoblin-wiring-installed-stock");
    std::fs::create_dir_all(&installed).expect("create installed stock dir");
    std::fs::write(installed.join("node.lua"), "").expect("write node.lua");
    let mut connected = false;
    let slot = ShellSlot::from_env(
        &|name: &str| (name == SHELL_VM_VARIABLE).then(|| "0".to_owned()),
        &installed,
        |_: &Path| {
            connected = true;
            Err(ShellVmError::Link("must not run".to_owned()))
        },
    );
    assert!(!slot.is_running(), "GNOBLIN_SHELL_VM=0 must keep the shell VM off");
    assert!(!connected, "the connector must not run when the variable is 0");
    std::fs::remove_dir_all(&installed).ok();
}

#[test]
fn a_set_variable_that_cannot_connect_leaves_the_slot_empty() {
    let mut slot = ShellSlot::from_env(
        &|name: &str| (name == SHELL_VM_VARIABLE).then(|| "/tmp/shell-vm".to_owned()),
        Path::new(ABSENT_STOCK),
        |dir: &Path| Err(ShellVmError::Link(format!("refused {}", dir.display()))),
    );
    assert!(!slot.is_running(), "a failed connect must leave the slot empty");
    assert!(slot.tick(0.0).is_ok(), "the role keeps serving after a failed connect");
}
