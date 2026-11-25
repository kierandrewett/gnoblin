//! Example 22: the UI toolkit of `libgnoblin-std`.
//!
//! A canvas is a node of the compositor's tree that paints a list of drawing operations, and the toolkit's classes (`Text`, `Entry`,
//! `UiBox`, `Bar`, `Toast`) are served by the library: layout, text measurement, the paint list's encoding, pointer and key delivery and
//! the keyboard focus are native, and a shell sets properties and hears signals. This shell:
//!
//! - puts a `Text` over the windows;
//! - puts an `Entry` over the windows, gives it the keyboard, and listens for Return: the keys the compositor sends the focused canvas
//!   type into it, and `<Return>` calls the closure with the text.
//!
//! Every line it logs starts with `ui:`. Build it with `cargo build -p gnoblin-example-native-ui` and run it with
//! `gnoblin ./target/debug/libnative_ui.so`, then type and press Return.

use gnoblin_std::objects::{Entry, Text};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;

    let label = Text::new(&objects)?;
    label.set_text("Type, then press Return")?;
    label.set_x(20.0)?;
    label.set_y(20.0)?;
    label.mount_top()?;

    let entry = Entry::new(&objects)?;
    entry.set_x(20.0)?;
    entry.set_y(48.0)?;
    entry.set_width(320.0)?;
    entry.set_height(36.0)?;
    entry.set_fill("#313244")?;
    entry.mount_top()?;
    // `focus` asks for the keyboard; the compositor gives it in a later frame, and `focus_changed` says when it did. Keys typed
    // before that go to whatever held the keyboard, so the shell says it holds the keyboard only then.
    let node = entry.node()?;
    entry
        .on_focus_changed(move |focused| {
            if focused {
                g.log(&format!("ui: the entry is node {node} and holds the keyboard"));
            }
        })?
        .keep();
    entry.focus()?;
    entry.set_on_submit(move |text| {
        g.log(&format!("ui: submitted {text:?}"));
        Ok(())
    })?;

    // The canvases are retained: they stay on screen when the shell lets go of its references.
    Ok(())
}
