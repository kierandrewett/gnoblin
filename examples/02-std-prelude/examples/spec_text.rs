//! The shell of the specification's section 3.1, in the form that compiles.
//!
//! The text is: `use gnoblin_std::prelude::*; #[gnoblin::main] fn main(g: &Gnoblin) { let workspace = Workspace::new("main");
//! g.on_client_created(|client| workspace.add(client)); g.bind("Super+Return", || g.spawn("foot")); }`. It is illustrative, and
//! three things in it are not Rust as written: `Workspace::new` can fail (the workspace may exist already, and a shell has no
//! business carrying on without one), so the result is unwrapped; a closure the compositor calls later cannot borrow `workspace`
//! from the entry function's frame, so the one that uses it is `move`; and `g.on_client_created(..)` and `g.bind(..)` return a
//! `Result` too (the compositor can refuse a claim). Everything else is the text: the prelude, the attribute, `fn main(g: &Gnoblin)`
//! with no return type, the plus-form chord, and callbacks that return what the call they make returns (`workspace.add(client)`
//! and `g.spawn("foot")` are `Result`s, which the SDK logs when they are errors). The test that runs it against the fake compositor
//! is `example_2_spec_text` in `crates/gnoblin-std-impl/tests/examples.rs`.
//!
//! Build it with `cargo build -p gnoblin-example-std-prelude --example spec_text`.

use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) {
    let workspace = Workspace::new("main").expect("the workspace `main` can be made");
    g.on_client_created(move |client| workspace.add(client))
        .expect("the shell can listen for windows");
    g.bind("Super+Return", || g.spawn("foot"))
        .expect("the chord can be claimed");
}
