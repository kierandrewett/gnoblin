//! Example 28: the input matcher of `libgnoblin-core`.
//!
//! A shell binds a trigger on the seat's input context. The library claims the chord on the seat, shares one claim between the bindings of
//! one trigger, and runs the right binding when the chord is pressed. This shell binds:
//!
//! - `Super+a` twice: a binding of priority 5 that declines the press (it answers `false`), and a binding of priority 0 that handles it,
//!   so a press runs the first and then the second;
//! - `Super+b`: a binding that says which generation of the shell handles it, so a reload that hands the chord to the next generation is
//!   seen: the compositor never holds the chord for nobody and never for both.
//!
//! Every line it logs starts with `input:`. Build it with `cargo build -p gnoblin-example-native-input` and run it with
//! `gnoblin ./target/debug/libnative_input.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{BindingOptions, ModelRoot};

/// Which generation of the shell this is. A reload test builds the example again with another number.
const GENERATION: u32 = 1;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(input) = model.input()? else {
        return Ok(());
    };
    let Some(context) = input.primary()? else {
        g.log("input: the compositor reports no seat");
        return Ok(());
    };

    // A press runs the binding of the higher priority first. It declines, so the press goes on to the next.
    context.bind(
        "Super+a",
        Some(move |_| {
            g.log("input: high declines Super+a");
            Ok(false)
        }),
        Some(&BindingOptions {
            id: Some("high".to_owned()),
            priority: Some(5.0),
            ..BindingOptions::default()
        }),
    )?;
    context.bind(
        "Super+a",
        Some(move |_| {
            g.log("input: low handles Super+a");
            Ok(true)
        }),
        Some(&BindingOptions {
            id: Some("low".to_owned()),
            ..BindingOptions::default()
        }),
    )?;

    // The chord the next generation of the shell is handed.
    context.bind(
        "Super+b",
        Some(move |_| {
            g.log(&format!("input: generation {GENERATION} handles Super+b"));
            Ok(true)
        }),
        None,
    )?;

    // Three bindings stand on two claims: the two of `Super+a` share one.
    g.log(&format!(
        "input: generation {GENERATION} holds {} claims for {} bindings",
        context.claims()?.len(),
        context.describe_bindings()?.len()
    ));
    g.log(&format!("input: generation {GENERATION} is running"));
    Ok(())
}
