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
//! Every line it logs starts with `clients:`, and the other shells log the same lines. Build it with
//! `cargo build -p gnoblin-example-native-clients` and run it with `gnoblin ./target/debug/libnative_clients.so`.

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

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let decorations = Decorations::get(&objects)?;
    let menus = WindowMenus::get(&objects)?;
    menus.start()?;
    let main = Workspace::new(&objects, "main")?;
    Workspaces::get(&objects)?.add(&main)?;

    let model = ModelRoot::get(&g.objects()?)?;

    // The library puts a window on the workspace as it appears and tells the shell its id as text. `get_by_id` is the `Client` of it, which
    // `add` and `contains` take.
    let (Some(members), Some(counted)) = (main.clients()?, main.clients()?) else {
        g.log("clients: the workspace has no clients");
        return Ok(());
    };
    let finder = ModelRoot::get(&g.objects()?)?;
    members
        .on_added(move |id| {
            let Ok(Some(clients)) = finder.clients() else { return };
            let Ok(Some(client)) = clients.get_by_id(&id) else {
                return;
            };
            let title = client.title().unwrap_or_default();
            let in_main = counted.add(&client).and_then(|()| counted.contains(&client));
            g.log(&format!(
                "clients: {title}: it is in main: {}",
                in_main.unwrap_or(false)
            ));
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
    g.log("clients: running");
    Ok(())
}
