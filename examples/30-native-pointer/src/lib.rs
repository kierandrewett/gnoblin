//! Example 30: pointer drags, clicks and touchpad swipes on the input context of `libgnoblin-core`.
//!
//! A shell binds a trigger on the seat's input context, and the library claims it. A pointer button held with `Super` is a click or a drag:
//! a press that is let go before the pointer has gone 4 pixels is a click, and one that goes further is a drag. This shell binds:
//!
//! - `Super+MouseLeft+Drag`: one function for the whole drag. It is called with `phase` `start` (the pointer is where the press was; answer
//!   `false` to leave the drag to the next binding), then `update` for each move, then `end` when the button is let go, or `cancel` when
//!   the window under the drag closes, the binding is removed, or the shell is replaced while the drag is going;
//! - `Super+MouseRight`: a click;
//! - `Swipe3Left`: a three-finger swipe to the left. A function runs once, when the swipe ends having gone far enough.
//!
//! Every line it logs starts with `pointer:`. Build it with `cargo build -p gnoblin-example-native-pointer` and run it with
//! `gnoblin ./target/debug/libnative_pointer.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::{BindingEvent, BindingPhase, Gnoblin};

/// Which generation of the shell this is. A reload test builds the example again with another number.
const GENERATION: u32 = 1;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Gnoblin::get(&objects)?;
    let input = model.input()?;
    let context = input.primary()?;

    // One function follows the drag from its start to its end. Any answer but `false` at the start takes the drag.
    context.bind(
        "Super+MouseLeft+Drag",
        Some(move |event: BindingEvent| {
            let phase = event.phase.map_or("none", BindingPhase::as_str);
            g.log(&format!(
                "pointer: generation {GENERATION} drag {phase} dx={} dy={}",
                event.dx.unwrap_or(0.0),
                event.dy.unwrap_or(0.0)
            ));
            Ok(true)
        }),
        None,
    )?;

    // A click is a press that did not become a drag.
    context.bind(
        "Super+MouseRight",
        Some(move |_| {
            g.log(&format!("pointer: generation {GENERATION} click"));
            Ok(true)
        }),
        None,
    )?;

    // A swipe: the function runs when the fingers leave the pad, if they went far enough to the left.
    context.bind(
        "Swipe3Left",
        Some(move |event: BindingEvent| {
            g.log(&format!(
                "pointer: generation {GENERATION} swipe left, {:.2} of the way",
                event.progress.unwrap_or(0.0)
            ));
            Ok(true)
        }),
        None,
    )?;

    // The drag and the click of one button stand on one claim: two claims for the buttons and one for the swipe.
    g.log(&format!(
        "pointer: generation {GENERATION} holds {} claims for {} bindings",
        context.claims()?.len(),
        context.describe_bindings()?.len()
    ));
    g.log(&format!("pointer: generation {GENERATION} is running"));
    Ok(())
}
