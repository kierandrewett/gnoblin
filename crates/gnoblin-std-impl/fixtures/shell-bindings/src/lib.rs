//! A std shell for the binding tests: `Super+Button1` moves the window under the pointer, `Alt+Button3` resizes it by the nearest side,
//! and `Super+Button2` is a drag binding of the shell's own that raises the window it landed on and counts what it was told.

use gnoblin_std::prelude::*;
use gnoblin_std::{ClientMoveBinding, ClientResizeBinding, DragEvent, ResizeGrip};

/// Raises the window under the press when the pointer has travelled far enough, and logs each step.
struct Raiser;

impl DragBinding for Raiser {
    fn trigger(&self) -> String {
        "Super+Button2".to_owned()
    }

    fn threshold(&self) -> f64 {
        10.0
    }

    fn on_start(&mut self, event: &DragEvent) -> bool {
        let Some(client) = &event.client else {
            return false;
        };
        client.window().gnoblin().log(&format!(
            "raiser start {} at {},{} local {:?}",
            client.window().id().0,
            event.position.x,
            event.position.y,
            event.local_position.map(|p| (p.x, p.y))
        ));
        let _ = client.window().raise();
        true
    }

    fn on_update(&mut self, event: &DragEvent) {
        if let Some(client) = &event.client {
            client
                .window()
                .gnoblin()
                .log(&format!("raiser update {},{}", event.position.x, event.position.y));
        }
    }

    fn on_end(&mut self, event: &DragEvent) {
        if let Some(client) = &event.client {
            client.window().gnoblin().log("raiser end");
        }
    }

    fn on_cancel(&mut self, event: &DragEvent) {
        if let Some(client) = &event.client {
            client.window().gnoblin().log("raiser cancel");
        }
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    std.bind_drag(ClientMoveBinding::new())?;
    std.bind_drag(
        ClientResizeBinding::new()
            .trigger("Alt+Button3")
            .grip(ResizeGrip::Nearest),
    )?;
    std.bind_drag(Raiser)?;
    g.log("bindings shell is running");
    Ok(())
}
