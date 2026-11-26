//! Example 36: a window as the core's `Client`, passed to the classes of `libgnoblin-std` that take one.
//!
//! When a window appears this shell asks `Decorations` for the room of the window's frame (`inset_of`), opens the window menu on it
//! (`WindowMenus::show`), closes the menu, and adds the window to a workspace (`clients.add`). Every one of those takes the window's
//! `Client`, which the core made, and not its node number as text: the library asks the core what the handle is before it uses it. A window
//! that has closed is still a `Client` whose last values can be read, and the library says it is gone.
//!
//! The Rust types make the other two refusals a build error: text where a `Client` is taken, and an object of another class. The
//! TypeScript and Python shells of the same example log those refusals at run time.
//!
//! For the first window it then reads the members the library adds to a window and an output, through the extension traits
//! `ClientStdExt` and `OutputStdExt` of the prelude: the window's `workspace`, `move_to_workspace`, `style`, `style_warnings` and
//! `move_with_keys`, and the output's `workspace` and `show`.
//!
//! Every line it logs starts with `clients:`, and the other shells log the same lines. Build it with
//! `cargo build -p gnoblin-example-native-clients` and run it with `gnoblin ./target/debug/libnative_clients.so`.

use std::cell::Cell;

use gnoblin_core::objects::{Client, ModelRoot};
use gnoblin_std::objects::{Decorations, WindowMenus, Workspace, Workspaces};
use gnoblin_std::prelude::*;

/// What the shell does with a window that appeared: each member takes the `Client` as it is.
fn arrived(g: &Gnoblin, decorations: &Decorations, menus: &WindowMenus, client: &Client) -> gnoblin_std::Result<()> {
    let title = client.title()?;
    let inset = decorations.inset_of(client)?;
    g.log(&format!(
        "clients: {title}: the frame takes {} {} {} {}",
        inset.top, inset.right, inset.bottom, inset.left
    ));
    let menu = menus.show(client, Some(10.0), Some(10.0))?;
    let for_the_window = match menu {
        Some(menu) => menu.client()? == client.id()?,
        None => false,
    };
    g.log(&format!(
        "clients: {title}: the menu is for the window: {for_the_window}"
    ));
    menus.close()?;
    Ok(())
}

/// The members the library adds to the core's `Client` and `Output`, read once, for the first window.
fn check_members(g: &Gnoblin, workspaces: &Workspaces, client: &Client) -> gnoblin_std::Result<()> {
    let title = client.title()?;
    let say = |text: String| g.log(&format!("clients: {title}: {text}"));
    let (Some(main), Some(side)) = (workspaces.by_id("main")?, workspaces.by_id("side")?) else {
        say("the workspaces are missing".to_owned());
        return Ok(());
    };
    let is = |workspace: Option<Workspace>, id: &str| {
        workspace.and_then(|workspace| workspace.id().ok()).as_deref() == Some(id)
    };
    say(format!("its workspace is main: {}", is(client.workspace()?, "main")));
    let moved = client.move_to_workspace(&side)?;
    say(format!(
        "moved to side: {moved}, now in side: {}",
        is(client.workspace()?, "side")
    ));
    let back = client.move_to_workspace(&main)?;
    say(format!(
        "moved back: {back}, again refused: {}",
        !client.move_to_workspace(&main)?
    ));
    client.style()?.set_opacity(0.8)?;
    say(format!(
        "its own style holds 0.8: {}",
        (client.style()?.opacity()? - 0.8).abs() < 1e-6
    ));
    say(format!(
        "its style warnings are a list: {}",
        client.style_warnings().is_ok()
    ));
    say(format!(
        "keyboard move before Repositioning starts: {}",
        client.move_with_keys()?
    ));
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(output) = model.outputs()?.and_then(|outputs| outputs.at(0).ok().flatten()) else {
        say("there is no output".to_owned());
        return Ok(());
    };
    say(format!("the output shows main: {}", is(output.workspace()?, "main")));
    output.show(&side, None)?;
    let showed = is(output.workspace()?, "side");
    output.show(&main, None)?;
    say(format!("the output showed side: {showed}"));
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let decorations = Decorations::get(&objects)?;
    let menus = WindowMenus::get(&objects)?;
    menus.start()?;
    let main = Workspace::new(&objects, "main")?;
    let side = Workspace::new(&objects, "side")?;
    Workspaces::get(&objects)?.add(&main)?;
    Workspaces::get(&objects)?.add(&side)?;

    let model = ModelRoot::get(&g.objects()?)?;

    // The library puts a window on the workspace as it appears and tells the shell of it with the core's `Client`, which `add` and
    // `contains` take as they are.
    let (Some(members), Some(counted)) = (main.clients()?, main.clients()?) else {
        g.log("clients: the workspace has no clients");
        return Ok(());
    };
    let finder = ModelRoot::get(&g.objects()?)?;
    let workspaces = Workspaces::get(&objects)?;
    let checked = Cell::new(false);
    members
        .on_added(move |client: Client| {
            let title = client.title().unwrap_or_default();
            let in_main = counted.add(&client).and_then(|()| counted.contains(&client));
            g.log(&format!(
                "clients: {title}: it is in main: {}",
                in_main.unwrap_or(false)
            ));
            // Two values of one object are equal: the library's `Client` is the one the core's own collection gives.
            let same = finder
                .clients()
                .ok()
                .flatten()
                .and_then(|clients| clients.get_by_id(&client.id().ok()?).ok().flatten())
                .is_some_and(|core| core == client);
            g.log(&format!("clients: {title}: the library's client is the core's: {same}"));
            if !checked.replace(true)
                && let Err(error) = check_members(g, &workspaces, &client)
            {
                g.log(&format!("clients: {title}: the members failed: {error}"));
            }
        })?
        .keep();
    model
        .on_client_created(move |client, _| {
            if let Err(error) = arrived(g, &decorations, &menus, &client) {
                g.log(&format!("clients: cannot use a window: {error}"));
            }
        })?
        .keep();

    // A window that has closed is still a `Client`, and its last values can be read; the library says it is gone.
    let closing = Decorations::get(&objects)?;
    model
        .on_client_closed(move |client| {
            let title = client.title().unwrap_or_default();
            match closing.inset_of(&client) {
                Ok(_) => g.log(&format!("clients: {title}: a closed window was answered")),
                Err(error) => g.log(&format!("clients: {title}: a closed window is refused: {error}")),
            }
        })?
        .keep();
    // The list holds the workspace from here on.
    std::mem::forget(main);
    std::mem::forget(side);
    g.log("clients: running");
    Ok(())
}
