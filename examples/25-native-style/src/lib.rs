//! Example 25: the styles and transitions of `libgnoblin-std`.
//!
//! A style is what every window, a workspace's windows, a selection or one window looks like. This shell gives every window rounded
//! corners and two shadows, then stages a transition that also changes the opacity of every window and a node of its own, and aborts
//! it. The library puts back the opacity it held at the start, and the shell's own `rollback` is told, before `abort` answers. Every
//! line it logs starts with `style:`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_std::objects::{
    CornerRadius, SceneNode, Shadow, Style, StyleProperty, Transition, TransitionOptions, TransitionValue,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let style = Style::of_clients(&objects)?;
    style.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    style.set_shadows(&[
        Shadow {
            blur: 8.0,
            ..Shadow::default()
        },
        Shadow {
            y: 4.0,
            blur: 20.0,
            ..Shadow::default()
        },
    ])?;
    let radius = style.corner_radius()?.and_then(|radii| radii.all).unwrap_or(0.0);
    let shadows = style.shadows()?;
    g.log(&format!(
        "style: corner radius {radius}, {} shadows, the first blur {}",
        shadows.len(),
        shadows.first().map_or(0.0, |shadow| shadow.blur)
    ));

    let probe = SceneNode::new(&objects)?;
    probe.set_id("probe")?;
    let rolled = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&rolled);
    Transition::begin(
        &objects,
        &TransitionOptions {
            duration: 200.0,
            ..TransitionOptions::default()
        },
        Some(move || {
            counted.set(counted.get() + 1);
            Ok(())
        }),
    )?;
    Transition::set(
        &objects,
        probe.node()?,
        "x",
        &TransitionValue {
            number: Some(40.0),
            ..TransitionValue::default()
        },
    )?;
    style.set_opacity(0.5)?;
    Transition::abort(&objects)?;
    g.log(&format!(
        "style: transition aborted, rollback ran {}, opacity is set {}, x is {}",
        rolled.get(),
        style.is_set(StyleProperty::Opacity)?,
        probe.x()?
    ));
    // The node is retained: it stays in the tree when the shell lets go of its reference.
    Ok(())
}
