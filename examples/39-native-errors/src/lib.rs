//! Example 39: why a call failed, and which call it was.
//!
//! A call the library refuses gives a `Fail` with a `code`, the same set of words every shell sees (`already_exists`, `not_found`), and a
//! `target`, the object and member of the call (`Client#7.close`). This shell makes each of three failures and logs what it reads:
//!
//! - a context whose name is taken is refused with `already_exists`, by `InputManager::add_context`;
//! - a window that has closed is still a `Client`, and a call on it is refused with `not_found`, by `Client#<id>.close`;
//! - a context with no name is a bad argument: it has no code at all (`Fail::reason` is none), which TypeScript and Python raise as
//!   a `TypeError` and not as a `GnoblinError`.
//!
//! Every line it logs starts with `errors:`, and the TypeScript and Python shells of the same example log the same lines. Build it with
//! `cargo build -p gnoblin-example-native-errors` and run it with `gnoblin ./target/debug/libnative_errors.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{InputContext, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;

    {
        let input = model.input()?;
        // The first context takes the name; a second with the same name is refused.
        let first = InputContext::new(&objects, "errors")?;
        input.add_context(&first)?;
        let second = InputContext::new(&objects, "errors")?;
        match input.add_context(&second) {
            Ok(()) => g.log("errors: a taken name was accepted"),
            Err(fail) => g.log(&format!(
                "errors: a taken name is refused: code={} target={}",
                fail.code(),
                fail.target().unwrap_or("none")
            )),
        }

        // A context with no name is a bad argument: the failure has no code.
        let nameless = InputContext::new(&objects, "")?;
        match input.add_context(&nameless) {
            Ok(()) => g.log("errors: an empty name was accepted"),
            Err(fail) => g.log(&format!(
                "errors: a bad argument is refused: TypeError={}",
                fail.reason().is_none()
            )),
        }
        // The list holds the contexts from here on.
        std::mem::forget(first);
        std::mem::forget(second);
        std::mem::forget(nameless);
    }

    // A window the shell has been told of is always told closed. The model reads the tree a batch at a time, so a window that opens
    // and closes within a single batch is told of neither way.
    model
        .on_client_created(move |_client, _| g.log("errors: a window appeared"))?
        .keep();

    // A window that has closed is still a `Client`; a call on it says it is gone.
    model
        .on_client_closed(move |client| {
            let title = client.title().unwrap_or_default();
            match client.close() {
                Ok(()) => g.log(&format!("errors: {title}: a closed window was answered")),
                Err(fail) => g.log(&format!(
                    "errors: {title}: a closed window is refused: code={} target={}",
                    fail.code(),
                    fail.target().unwrap_or("none")
                )),
            }
        })?
        .keep();
    g.log("errors: running");
    Ok(())
}
