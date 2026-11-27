//! The shell of the specification's section 3.1, in the form that compiles.
//!
//! One workspace, `main`, that every window opened after the shell started joins, and a plus-form chord that starts a terminal. Making
//! a workspace, listening for windows and claiming a chord can each fail, so each result is unwrapped: a shell has no business carrying
//! on without them. The closure the compositor calls later cannot borrow from the entry function's frame, so the one that uses the
//! workspace is `move`. The test that runs it against the fake compositor is `example_2_spec_text` in
//! `crates/gnoblin-std-impl/tests/examples.rs`.
//!
//! Build it with `cargo build -p gnoblin-example-std-prelude --example spec_text`.

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::Workspace;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) {
    let objects = g.std().and_then(|std| std.objects()).expect("the std library opens");
    let workspace = Workspace::new(&objects, "main").expect("the workspace `main` can be made");
    let clients = workspace
        .clients()
        .expect("the workspace has windows")
        .expect("the workspace has a window list");
    ModelRoot::get(&g.objects().expect("the core objects open"))
        .and_then(|model| model.clients())
        .and_then(|all| {
            all.on_added(move |client, _| {
                let _ = clients.add(&client);
            })
        })
        .expect("the shell can listen for windows")
        .keep();
    g.bind("Super+Return", || g.spawn("foot"))
        .expect("the chord can be claimed");
}
