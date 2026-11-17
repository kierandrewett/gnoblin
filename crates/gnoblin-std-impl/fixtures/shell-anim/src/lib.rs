//! A std shell for the animation tests: every chord runs one step of a script, and the test holds the tree writes of each step to the
//! TypeScript library's for the same step (`examples.rs`' counterpart in `anim_style.rs`).
//!
//! The steps, in the order the chords are pressed (`Super+a` to `Super+q`): a transition over two windows; a transition that is staged
//! held (`autoplay` off) with a delay, then scrubbed, played, paused and reversed, and another that is cancelled; a transition of no
//! duration; a layout change inside a transition; a global style, a selection's, a client's and an unset; a bad style value; a camera,
//! a view and a flat camera; and a transition over a flat camera and a field of view. What a step observed (the end of a transition, an
//! error) is printed as an `anim-log` line on stderr.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_std::prelude::*;
use gnoblin_std::{Border, Camera3DOptions, CornerRadius, Filter, Shadow, StyleProperty};

/// Prints what a step saw.
fn log(text: &str) {
    eprintln!("anim-log {text}");
}

/// The window `id` as a client. The scenario opens every window before it presses a chord that names it, so a missing one is the
/// scenario's fault and the chord's panic is caught and reported.
fn client(std: &Std, id: u64) -> gnoblin_std::Result<Client> {
    Ok(std
        .clients()?
        .get(&id.to_string())?
        .unwrap_or_else(|| panic!("the scenario has no client {id}")))
}

/// What a step leaves for a later one.
#[derive(Default)]
struct Kept {
    held: Option<Transition>,
    camera: Option<Camera3D>,
    flat: Option<Camera2D>,
}

type Step = Box<dyn Fn(&Std, &mut Kept) -> gnoblin_std::Result<()>>;

fn steps() -> Vec<(&'static str, Step)> {
    let basic: Step = Box::new(|std, _| {
        let (c40, c41) = (client(std, 40)?, client(std, 41)?);
        let t = std.transition(
            TransitionOptions::new().duration_ms(200.0).easing(Easing::EaseInOut),
            |tx| {
                tx.set(c40.window().id(), "opacity", 0.5)?;
                tx.set(c40.window().id(), "x", 300.0)?;
                tx.set(c41.window().id(), "opacity", 0.25)
            },
        )?;
        t.on_finished(|result| log(&format!("basic finished cancelled={}", result.cancelled)))?;
        Ok(())
    });
    let staged = || -> Step {
        Box::new(|std, kept| {
            let c41 = client(std, 41)?;
            let t = std.transition(
                TransitionOptions::new()
                    .duration_ms(100.0)
                    .delay_ms(50.0)
                    .easing(Easing::Linear)
                    .autoplay(false),
                |tx| {
                    tx.set(c41.window().id(), "x", 100.0)?;
                    tx.set(c41.window().id(), "rotation", [0.0, 0.5, 0.0])
                },
            )?;
            t.on_finished(|result| log(&format!("staged finished cancelled={}", result.cancelled)))?;
            kept.held = Some(t);
            Ok(())
        })
    };
    let held = |act: fn(&Transition) -> gnoblin_std::Result<()>| -> Step {
        Box::new(move |_, kept| match &kept.held {
            Some(t) => act(t),
            None => Ok(()),
        })
    };
    let instant: Step = Box::new(|std, _| {
        let c42 = client(std, 42)?;
        let t = std.transition(TransitionOptions::new().duration_ms(0.0), |tx| {
            tx.set(c42.window().id(), "opacity", 0.75)?;
            tx.set(c42.window().id(), "x", 20.0)
        })?;
        t.on_finished(|result| log(&format!("instant finished cancelled={}", result.cancelled)))?;
        Ok(())
    });
    let layout: Step = Box::new(|std, _| {
        let workspace = std.workspaces()?.at(0)?.expect("a default workspace");
        std.transition(TransitionOptions::new().duration_ms(300.0), |_| {
            workspace.set_layout(MasterStackLayout::new().ratio(0.6))
        })?;
        Ok(())
    });
    let style_global: Step = Box::new(|std, _| {
        let style = std.clients()?.style()?;
        style.set_shadows(&[
            Shadow {
                y: 8.0,
                blur: 24.0,
                color: "#00000066".to_owned(),
                ..Shadow::default()
            },
            Shadow {
                blur: 4.0,
                color: "#0004".to_owned(),
                ..Shadow::default()
            },
        ])?;
        style.set_borders(&[
            Border {
                width: 2.0,
                color: "#ff8800".to_owned(),
                ..Border::default()
            },
            Border {
                offset: 3.0,
                ..Border::default()
            },
        ])?;
        style.set_backdrop_filter(&[Filter::Blur(12.0), Filter::Saturate(1.2), Filter::Brightness(1.1)])?;
        style.set_corner_radius(CornerRadius::All(12.0))?;
        style.set_opacity(0.9)
    });
    let style_selection: Step = Box::new(|std, _| {
        let selected = std.clients()?.where_(&Query::new().is("app_id", "nautilus"))?;
        selected.style()?.set_borders(&[Border {
            width: 3.0,
            color: "#00ff00".to_owned(),
            ..Border::default()
        }])
    });
    let style_client: Step = Box::new(|std, _| {
        let style = client(std, 40)?.style()?;
        style.set_opacity(0.5)?;
        style.set_shadows(&[])
    });
    let style_unset: Step = Box::new(|std, _| {
        client(std, 40)?.style()?.unset(StyleProperty::Opacity)?;
        let computed = client(std, 41)?.computed_style()?;
        log(&format!(
            "computed 41 opacity={} borders={} origin={}",
            computed.opacity,
            computed.borders.len(),
            computed.origin_of(StyleProperty::Borders)
        ));
        Ok(())
    });
    let style_bad: Step = Box::new(|std, _| {
        match std.clients()?.style()?.set_opacity(2.0) {
            Err(error) => log(&format!("bad opacity: {}", error.message())),
            Ok(()) => log("bad opacity: accepted"),
        }
        let border = Border {
            width: -1.0,
            ..Border::default()
        };
        match client(std, 42)?.style()?.set_borders(&[border]) {
            Err(error) => log(&format!("bad border: {}", error.message())),
            Ok(()) => log("bad border: accepted"),
        }
        Ok(())
    });
    let scene: Step = Box::new(|std, kept| {
        let scene = std.scene()?;
        let camera = Camera3D::new(
            std,
            Camera3DOptions {
                position: Some([1.0, 2.0, 3.0]),
                projection: Some(gnoblin_std::Projection::Perspective),
                fov: Some(70.0),
                near: Some(0.2),
                ..Camera3DOptions::default()
            },
        )?;
        scene.root().add(&camera)?;
        let c40 = client(std, 40)?;
        let view = View::new(
            std,
            &ViewOptions {
                camera: Some(&camera),
                width: Some(400.0),
                height: Some(300.0),
                content: Some(&c40),
            },
        )?;
        scene.root().add(&view)?;
        let flat = Camera2D::new(
            std,
            Camera2DOptions {
                center: Some((10.0, 20.0)),
                zoom: Some(2.0),
                roll: Some(0.5),
            },
        )?;
        scene.root().add(&flat)?;
        log(&format!("camera size={}", camera.size()?));
        if let Err(error) = flat.set_zoom(0.0) {
            log(&format!("bad zoom: {}", error.message()));
        }
        kept.camera = Some(camera);
        kept.flat = Some(flat);
        Ok(())
    });
    let scene_transition: Step = Box::new(|std, kept| {
        let (Some(camera), Some(flat)) = (&kept.camera, &kept.flat) else {
            return Ok(());
        };
        std.transition(TransitionOptions::new().duration_ms(100.0), |_| {
            flat.set_zoom(4.0)?;
            flat.set_center(30.0, 40.0)?;
            flat.set_roll(1.0)?;
            camera.set_fov(40.0)
        })?;
        log(&format!("flat zoom={} fov={}", flat.zoom()?, camera.fov()?));
        Ok(())
    });
    vec![
        ("Super+a", basic),
        ("Super+b", staged()),
        ("Super+c", held(|t| t.set_progress(0.5))),
        ("Super+d", held(Transition::play)),
        ("Super+e", held(Transition::pause)),
        ("Super+f", held(Transition::reverse)),
        ("Super+g", staged()),
        ("Super+h", held(Transition::cancel)),
        ("Super+i", instant),
        ("Super+j", layout),
        ("Super+k", style_global),
        ("Super+l", style_selection),
        ("Super+m", style_client),
        ("Super+n", style_unset),
        ("Super+o", style_bad),
        ("Super+p", scene),
        ("Super+q", scene_transition),
    ]
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    let kept = Rc::new(RefCell::new(Kept::default()));
    for (chord, step) in steps() {
        let (std, kept) = (std.clone(), Rc::clone(&kept));
        g.bind(chord, move || {
            if let Err(error) = step(&std, &mut kept.borrow_mut()) {
                log(&format!("{chord} failed: {error}"));
            }
        })?;
    }
    eprintln!("anim shell is running");
    Ok(())
}
