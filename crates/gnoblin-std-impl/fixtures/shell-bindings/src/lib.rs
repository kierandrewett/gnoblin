//! A std shell for the binding tests: `Super+Button1` moves the window under the pointer, `Alt+Button3` resizes it by the nearest side,
//! and `Super+Button2` is a drag binding of the shell's own that raises the window it landed on and counts what it was told.

use gnoblin_core::objects::{BindingEvent, BindingOptions, BindingPhase, ModelRoot};
use gnoblin_std::objects::{MoveBinding, ResizeBinding, ResizeGrip};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let moving = MoveBinding::new(&objects)?;
    moving.install(None)?;
    std::mem::forget(moving);
    let resizing = ResizeBinding::new(&objects)?;
    resizing.set_trigger("Alt+MouseRight+Drag")?;
    resizing.set_grip(ResizeGrip::Nearest)?;
    resizing.install(None)?;
    std::mem::forget(resizing);

    // The shell's own drag: asked to start only past ten pixels, told where the press was, and it raises the window it started on.
    let context = ModelRoot::get(&g.objects()?)?.input()?.primary()?;
    let shell = g.clone();
    context.bind(
        "Super+MouseMiddle+Drag",
        Some(move |event: BindingEvent| {
            let position = (event.position.x, event.position.y);
            match event.phase {
                Some(BindingPhase::Start) => {
                    let Some(client) = &event.client else {
                        return Ok(false);
                    };
                    shell.log(&format!(
                        "raiser start {} at {},{} local {:?}",
                        client.id()?,
                        position.0,
                        position.1,
                        event.local_position.as_ref().map(|p| (p.x, p.y))
                    ));
                    let _ = client.raise();
                }
                Some(BindingPhase::Update) => {
                    if event.client.is_some() {
                        shell.log(&format!("raiser update {},{}", position.0, position.1));
                    }
                }
                Some(BindingPhase::End) => {
                    if event.client.is_some() {
                        shell.log("raiser end");
                    }
                }
                Some(BindingPhase::Cancel) => {
                    if event.client.is_some() {
                        shell.log("raiser cancel");
                    }
                }
                None => {}
            }
            Ok(true)
        }),
        Some(&BindingOptions {
            threshold: Some(10.0),
            ..BindingOptions::default()
        }),
    )?;
    g.log("bindings shell is running");
    Ok(())
}
