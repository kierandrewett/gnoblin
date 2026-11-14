//! The runtime role's shell VM wiring: with no installed stock library and the variable unset,
//! nothing changes; `GNOBLIN_SHELL_VM=0` turns it off even when the library is installed; a set
//! variable that cannot be wired is refused; a failed connect leaves the role serving. `js` and `js:<dir>` select
//! the JavaScript shell and never the Lua VM; unset keeps Lua.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gnoblin_lua::shell_role::{SHELL_VM_VARIABLE, ShellFrame, ShellSlot, js_shell_request, shell_vm_dir};
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

fn selecting(value: &'static str) -> impl Fn(&str) -> Option<String> {
    move |name: &str| (name == SHELL_VM_VARIABLE).then(|| value.to_owned())
}

#[test]
fn js_selects_the_javascript_shell_with_an_optional_stock_directory_and_never_the_lua_vm() {
    assert_eq!(js_shell_request(&selecting("js")), Some(None));
    assert_eq!(
        js_shell_request(&selecting("js:/opt/stock")),
        Some(Some(PathBuf::from("/opt/stock")))
    );
    assert_eq!(js_shell_request(&selecting("js:")), Some(None));
    assert_eq!(js_shell_request(&|_: &str| None), None, "unset keeps Lua");
    assert_eq!(js_shell_request(&selecting("0")), None);
    assert_eq!(js_shell_request(&selecting("/tmp/shell-vm")), None);
    assert_eq!(
        js_shell_request(&selecting("jsx")),
        None,
        "only `js` and `js:<dir>` select it"
    );

    let installed = std::env::temp_dir().join("gnoblin-wiring-js-installed-stock");
    std::fs::create_dir_all(&installed).expect("create installed stock dir");
    std::fs::write(installed.join("node.lua"), "").expect("write node.lua");
    assert_eq!(
        shell_vm_dir(&selecting("js"), &installed),
        None,
        "the Lua VM does not take `js` for a directory, even when the library is installed"
    );
    assert_eq!(shell_vm_dir(&selecting("js:/opt/stock"), &installed), None);
    assert_eq!(
        shell_vm_dir(&|_: &str| None, &installed).as_deref(),
        Some(installed.as_path()),
        "unset still runs the installed Lua library"
    );
    std::fs::remove_dir_all(&installed).ok();
}

struct Counting {
    ticks: Rc<Cell<u32>>,
    fail_after: u32,
}

impl ShellFrame for Counting {
    fn tick(&mut self, _now: f64) -> Result<(), String> {
        self.ticks.set(self.ticks.get() + 1);
        if self.ticks.get() > self.fail_after {
            return Err("the link failed".to_owned());
        }
        Ok(())
    }
}

#[test]
fn the_js_variable_starts_the_js_shell_through_the_launcher_and_the_lua_connector_never_runs() {
    let ticks = Rc::new(Cell::new(0));
    let launched = Rc::clone(&ticks);
    let stock_seen = Cell::new(false);
    let launch = |_: &dyn Fn(&str) -> Option<String>, stock: Option<&Path>| -> Result<Box<dyn ShellFrame>, String> {
        stock_seen.set(stock == Some(Path::new("/opt/stock")));
        Ok(Box::new(Counting {
            ticks: Rc::clone(&launched),
            fail_after: 2,
        }))
    };
    let mut lua_connected = false;
    let mut slot = ShellSlot::from_env_or_js(
        &selecting("js:/opt/stock"),
        Path::new(ABSENT_STOCK),
        Some(&launch),
        |_: &Path| {
            lua_connected = true;
            Err(ShellVmError::Link("must not run".to_owned()))
        },
    );
    assert!(slot.is_running(), "the JavaScript shell runs in the slot");
    assert!(stock_seen.get(), "the launcher is told the stock directory");
    assert!(slot.tick(0.0).is_ok());
    assert!(slot.tick(0.016).is_ok());
    assert_eq!(ticks.get(), 2, "one frame per tick");
    // A failure ends the shell: the slot empties, names the error, and the next tick is a no-op.
    assert!(slot.tick(0.032).is_err());
    assert!(!slot.is_running());
    assert!(slot.tick(0.048).is_ok());
    assert_eq!(ticks.get(), 3);
    assert!(!lua_connected, "the Lua connector must not run under `js`");
}

#[test]
fn the_js_variable_without_a_launcher_or_with_a_failing_one_leaves_the_slot_empty() {
    let slot = ShellSlot::from_env_or_js(&selecting("js"), Path::new(ABSENT_STOCK), None, |_: &Path| {
        Err(ShellVmError::Link("must not run".to_owned()))
    });
    assert!(
        !slot.is_running(),
        "a build with no JavaScript shell serves without one"
    );

    let refuse = |_: &dyn Fn(&str) -> Option<String>, _: Option<&Path>| -> Result<Box<dyn ShellFrame>, String> {
        Err("no socket".to_owned())
    };
    let slot = ShellSlot::from_env_or_js(&selecting("js"), Path::new(ABSENT_STOCK), Some(&refuse), |_: &Path| {
        Err(ShellVmError::Link("must not run".to_owned()))
    });
    assert!(!slot.is_running(), "a launcher that fails leaves the role serving");
}

#[test]
fn any_other_value_is_the_lua_vms_and_the_launcher_does_not_run() {
    let launched = Cell::new(false);
    let launch = |_: &dyn Fn(&str) -> Option<String>, _: Option<&Path>| -> Result<Box<dyn ShellFrame>, String> {
        launched.set(true);
        Err("must not run".to_owned())
    };
    let slot = ShellSlot::from_env_or_js(
        &selecting("/tmp/shell-vm"),
        Path::new(ABSENT_STOCK),
        Some(&launch),
        |dir: &Path| Err(ShellVmError::Link(format!("refused {}", dir.display()))),
    );
    assert!(!slot.is_running());
    assert!(
        !launched.get(),
        "unset or a directory never starts the JavaScript shell"
    );
}
