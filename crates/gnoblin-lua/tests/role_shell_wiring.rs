//! The runtime role's shell VM wiring: unset means nothing changes, a set variable that cannot
//! be wired is refused by name, and a failed connect leaves the role serving.

use std::path::Path;

use gnoblin_lua::shell_role::{SHELL_VM_VARIABLE, ShellSlot};
use gnoblin_lua::shell_vm::ShellVmError;

#[test]
fn unset_variable_leaves_the_slot_empty_and_never_connects() {
    let mut connected = false;
    let mut slot = ShellSlot::from_env(&|_: &str| None, |_: &Path| {
        connected = true;
        Err(ShellVmError::Link("must not run".to_owned()))
    });
    assert!(!slot.is_running(), "an unset variable must not start a shell VM");
    assert!(slot.tick(0.0).is_ok(), "an empty slot ticks as a no-op");
    assert!(!connected, "the connector must not run when the variable is unset");
}

#[test]
fn a_set_variable_that_cannot_connect_leaves_the_slot_empty() {
    let mut slot = ShellSlot::from_env(
        &|name: &str| (name == SHELL_VM_VARIABLE).then(|| "/tmp/shell-vm".to_owned()),
        |dir: &Path| Err(ShellVmError::Link(format!("refused {}", dir.display()))),
    );
    assert!(!slot.is_running(), "a failed connect must leave the slot empty");
    assert!(slot.tick(0.0).is_ok(), "the role keeps serving after a failed connect");
}
