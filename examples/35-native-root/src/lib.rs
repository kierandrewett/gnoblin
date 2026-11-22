//! Example 35: the members of the `gnoblin` root object of `libgnoblin-core`.
//!
//! The root holds the clock and the timers, the build identity, a lookup of live objects by kind, the time an animation takes and the
//! compositor's settings. This shell reads each of them the way a desktop would:
//!
//! - reads the build identity (`version`);
//! - sets a timer with `timers.after`, cancels another, and checks that the first ran after its time on the clock (`timers.now`);
//! - lists the kinds `find` knows, and finds the windows and the outputs (`find::<Client>`, `find::<Output>`);
//! - when a window appears, finds the windows of its application, and reads `motion(200)` and the compositor's `user_shaders`.
//!
//! Every line it logs starts with `root:`, and the TypeScript and Python shells of the same example log the same lines. `find` takes the
//! kind as a type here, so a kind the core does not list is refused when the shell is built: `find::<Workspace>` needs the std package.
//! Build it with `cargo build -p gnoblin-example-native-root` and run it with `gnoblin ./target/debug/libnative_root.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Client, ClientQuery, Finds, ModelRoot, Output};

/// What the shell logs about a window that appeared: what each member of the root answers now.
fn report(g: &Gnoblin, root: &ModelRoot, client: &Client) -> gnoblin::Result<()> {
    let windows = root.find::<Client>(None)?.len();
    let foot = root
        .find::<Client>(Some(&ClientQuery {
            app_id: Some("foot".to_owned()),
            ..ClientQuery::default()
        }))?
        .len();
    // `find` takes no function: the shell filters the list in its own language.
    let titled = root
        .find::<Client>(None)?
        .iter()
        .filter(|found| found.title().unwrap_or_default().starts_with("Term"))
        .count();
    let outputs = root.find::<Output>(None)?.len();
    let motion = root.motion(200.0)?;
    let shaders = match root.compositor()? {
        Some(compositor) => compositor.user_shaders()?,
        None => true,
    };
    g.log(&format!(
        "root: created {}: windows = {windows}, appId foot = {foot}, title starts with Term = {titled}, outputs listed = {}, \
         motion(200) = {motion}, userShaders = {shaders}",
        client.title()?,
        outputs > 0
    ));
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;

    let version = model.version()?;
    g.log(&format!("root: gnoblin {} speaks api {}", version.gnoblin, version.api));

    let (Some(timers), Some(clock)) = (model.timers()?, model.timers()?) else {
        g.log("root: the core has no timers");
        return Ok(());
    };
    let started = clock.now()?;
    timers.after(40.0, move || {
        let moved = clock.now()? - started;
        g.log(&format!("root: the timer fired after its time: {}", moved >= 39.0));
        Ok(())
    })?;
    let never = timers.after(10.0, move || {
        g.log("root: a cancelled timer ran");
        Ok(())
    })?;
    never.cancel()?;
    g.log(&format!("root: a cancelled timer is pending: {}", never.pending()?));

    g.log(&format!(
        "root: find lists the kinds {}, {}",
        <Client as Finds<ModelRoot>>::KIND,
        <Output as Finds<ModelRoot>>::KIND
    ));

    let root = ModelRoot::get(&objects)?;
    model
        .on_client_created(move |client, _| {
            if let Err(error) = report(g, &root, &client) {
                g.log(&format!("root: cannot report a window: {error}"));
            }
        })?
        .keep();
    g.log("root: running");
    Ok(())
}
