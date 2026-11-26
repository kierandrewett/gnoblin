#![allow(unsafe_code)]
//! A shell closure that a call of a stateful class runs, and that calls the same object back (lane x92p).
//!
//! Written before the fix: `Library::with_native` held a unique borrow of an object's state for the whole call, so the closure's read of the
//! object the call was about was refused with `E_BUSY`, though the member only reads its state (a handle). The menu's cases are in
//! `menus_api.rs`. Each test runs over a desktop with no fixture.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::{Fail, Val};
use rig::{Rig, W, d, t};

/// What a layout of the shell read of the workspace, by the hook that read it.
type Heard = Rc<RefCell<Vec<(String, Result<Val, Fail>)>>>;

#[test]
fn a_layouts_set_split_run_by_the_workspaces_set_split_reads_the_workspace() {
    let rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let list = rig.make("workspaces", &[]);
    let slot: Rc<Cell<Option<W>>> = Rc::new(Cell::new(None));
    let seen: Heard = Rc::new(RefCell::new(Vec::new()));
    let (workspace_for, seen_for) = (Rc::clone(&slot), Rc::clone(&seen));
    // The layout of the shell: it places nothing, and when a divider is moved or put back it reads the workspace that asked.
    let layout = rig.implement("layout", &["compute", "set_split", "reset_split"], move |name, _| {
        if name == "compute" {
            return Ok(Val::List(Vec::new()));
        }
        let workspace = workspace_for.get().expect("the workspace is made");
        let id = workspace
            .api
            .member(workspace.class, abi::MEMBER_PROPERTY, "id")
            .expect("a property");
        seen_for
            .borrow_mut()
            .push((name.to_owned(), workspace.api.get(workspace.obj, id)));
        Ok(Val::None)
    });
    let workspace = rig.make("workspace", &[("id", t("main")), ("layout", layout)]);
    slot.set(Some(workspace));
    list.call("add", &[workspace.val()]);
    workspace.call("set_split", &[Val::Uint(1), d(0.6), Val::None]);
    workspace.call("reset_split", &[Val::Uint(1)]);
    let seen = seen.borrow();
    assert_eq!(seen.len(), 2, "the layout heard set_split and reset_split");
    for (name, read) in seen.iter() {
        match read {
            Ok(Val::Text(id)) => assert_eq!(id, "main", "{name}"),
            other => panic!("the layout's {name} could not read the workspace that asked: {other:?}"),
        }
    }
}
