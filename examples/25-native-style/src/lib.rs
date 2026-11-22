//! Example 25: the styles and transitions of `libgnoblin-std`.
//!
//! A style is what every window, a workspace's windows, a selection or one window looks like. This shell gives every window rounded
//! corners and two shadows, then stages a transition that also changes the opacity of every window and a node of its own, and aborts
//! it. The library puts back the opacity it held at the start, and the shell's own `rollback` is told, before `abort` answers. Every
//! line it logs starts with `style:`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_std::objects::{
    BlurFilter, Border, BrightnessFilter, ClientStyle, ContrastFilter, CornerRadius, Filter, SaturateFilter, SceneNode,
    ShaderFilter, Shadow, StyleProperty, Transition, TransitionOptions, TransitionValue,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let style = ClientStyle::of_clients(&objects)?;
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

    // The value classes: every one is built with its defaults and with values of its own, and the library hands back what it was given.
    let shader = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }";
    style.set_borders(&[
        Border::default(),
        Border {
            width: 3.0,
            color: "#ff0000".to_owned(),
            offset: -2.0,
            inset: true,
        },
    ])?;
    style.set_shadows(&[
        Shadow::default(),
        Shadow {
            x: 2.0,
            y: 4.0,
            blur: 20.0,
            spread: 3.0,
            color: "#112233".to_owned(),
            inset: true,
        },
    ])?;
    style.set_backdrop_filter(&[
        BlurFilter::default().into(),
        BlurFilter { radius: 20.0 }.into(),
        SaturateFilter::default().into(),
        SaturateFilter { amount: 1.5 }.into(),
        BrightnessFilter::default().into(),
        BrightnessFilter { amount: 0.5 }.into(),
        ContrastFilter::default().into(),
        ContrastFilter { amount: 1.25 }.into(),
    ])?;
    style.set_filter(&[
        ShaderFilter::new(shader.to_owned()).into(),
        ShaderFilter {
            params: vec![0.5, 2.0],
            ..ShaderFilter::new(shader.to_owned())
        }
        .into(),
    ])?;
    let describe = |filter: &Filter| match filter {
        Filter::Blur(blur) => format!("blur {}", blur.radius),
        Filter::Saturate(grade) => format!("saturate {}", grade.amount),
        Filter::Brightness(grade) => format!("brightness {}", grade.amount),
        Filter::Contrast(grade) => format!("contrast {}", grade.amount),
        Filter::Shader(shader) => {
            let params: Vec<String> = shader.params.iter().map(f64::to_string).collect();
            format!(
                "shader {}",
                if params.is_empty() {
                    "-".to_owned()
                } else {
                    params.join(",")
                }
            )
        }
    };
    let borders: Vec<String> = style
        .borders()?
        .iter()
        .map(|b| format!("{} {} {} {}", b.width, b.color, b.offset, b.inset))
        .collect();
    g.log(&format!("style: value classes borders {}", borders.join(" | ")));
    let shadows: Vec<String> = style
        .shadows()?
        .iter()
        .map(|s| format!("{} {} {} {} {} {}", s.x, s.y, s.blur, s.spread, s.color, s.inset))
        .collect();
    g.log(&format!("style: value classes shadows {}", shadows.join(" | ")));
    let backdrop: Vec<String> = style.backdrop_filter()?.iter().map(describe).collect();
    g.log(&format!(
        "style: value classes backdrop filters {}",
        backdrop.join(" | ")
    ));
    let own: Vec<String> = style.filter()?.iter().map(describe).collect();
    g.log(&format!("style: value classes filters {}", own.join(" | ")));

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
