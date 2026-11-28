//! Example 27: the window frame of `libgnoblin-std`.
//!
//! The decorations give every window a frame and call this shell's `frame` callback to fill it. A window the compositor negotiated
//! server-side decorations for gets a server frame, and this shell fills it with the stock titlebar (`Titlebar::install`): the title, the
//! three buttons and the drag area, which the library keeps up to date and paints. A window that draws its own frame gets a frame with no
//! edges, which the shell leaves empty. The window titled `close me` is closed the way its close button closes it: its actions fade the
//! window out and then ask the client to close. Every line it logs starts with `frame:`.
//!
//! Build it with `cargo build -p gnoblin-example-native-frame` and run it with `gnoblin ./target/debug/libnative_frame.so`.

use gnoblin_std::{Decorations, Titlebar};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let decorations = Decorations::get(&objects)?;
    let (shell, library) = (g.clone(), objects.clone());
    decorations.set_frame(move |client, frame| {
        if !frame.server()? {
            shell.log(&format!("frame: {} draws its own frame", client.app_id));
            return Ok(true);
        }
        Titlebar::install(&library, &frame, None)?;
        let taken = match frame.top()? {
            Some(top) => top.thickness()?,
            None => 0.0,
        };
        shell.log(&format!(
            "frame: {} has a titlebar and its top edge takes {taken}",
            client.app_id
        ));
        if client.title == "close me"
            && let Some(actions) = frame.actions()?
        {
            shell.log(&format!("frame: closing {} as its close button does", client.app_id));
            actions.close()?;
        }
        Ok(true)
    })?;
    decorations.start()?;
    g.log("frame: running");
    Ok(())
}
