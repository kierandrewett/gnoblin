//! A std shell for the animation tests: every chord runs one step of a script, and the test holds the tree writes of each step to the
//! TypeScript library's for the same step (`examples.rs`' counterpart in `anim_style.rs`).
//!
//! The steps, in the order the chords are pressed (`Super+a` to `Super+r`): a transition over two windows; a transition that is staged
//! held (`autoplay` off) with a delay, then scrubbed, played, paused and reversed, and another that is cancelled; a transition of no
//! duration; a layout change inside a transition; a global style, a selection's, a client's and an unset; a bad style value; a camera,
//! a view and a flat camera; a transition over a flat camera and a field of view; and a shader filter on one window. What a step observed (the end of a transition, an
//! error) is printed as an `anim-log` line on stderr.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::{Client, ClientQuery, Gnoblin};
use gnoblin_std::prelude::*;
use gnoblin_std::{
    BlurFilter, Border, BrightnessFilter, Camera2D, Camera3D, ClientStyle, CornerRadius, Easing, Filter, GridLayout,
    MasterStackLayout, MasterStackLayoutOptions, Objects, Projection, SaturateFilter, Scene, ShaderFilter, Shadow,
    StyleProperty, Transition, TransitionOptions, TransitionValue, View, Workspaces,
};

/// Prints what a step saw.
fn log(text: &str) {
    eprintln!("anim-log {text}");
}

/// What a step works with: both libraries' typed classes.
#[derive(Clone)]
struct World {
    core: gnoblin_core::Objects,
    objects: Objects,
}

impl World {
    /// The window `id` as a client. The scenario opens every window before it presses a chord that names it, so a missing one is the
    /// scenario's fault and the chord's panic is caught and reported.
    fn client(&self, id: u64) -> gnoblin_std::Result<Client> {
        Ok(Gnoblin::get(&self.core)?
            .clients()?
            .get_by_id(&id.to_string())?
            .unwrap_or_else(|| panic!("the scenario has no client {id}")))
    }
}

/// A number the transition stages for a property of a node.
fn number(value: f64) -> TransitionValue {
    TransitionValue {
        number: Some(value),
        ..TransitionValue::default()
    }
}

fn options(duration: f64) -> TransitionOptions {
    TransitionOptions {
        duration,
        ..TransitionOptions::default()
    }
}

/// What a step leaves for a later one.
#[derive(Default)]
struct Kept {
    held: Option<Transition>,
    /// Transitions that run to their end after their step: the object stays alive so its `finished` reaches the step's listener.
    running: Vec<Transition>,
    camera: Option<Camera3D>,
    flat: Option<Camera2D>,
}

type Step = Box<dyn Fn(&World, &mut Kept) -> gnoblin_std::Result<()>>;

fn node_of(client: &Client) -> gnoblin_std::Result<u64> {
    Ok(client.id()?.parse().expect("a client id is a node number"))
}

fn steps() -> Vec<(&'static str, Step)> {
    let basic: Step = Box::new(|world, kept| {
        let (c40, c41) = (world.client(40)?, world.client(41)?);
        let model = Gnoblin::get(&world.core)?;
        let objects = world.objects.clone();
        let t = model.request_transition(
            move || {
                Transition::set(&objects, node_of(&c40)?, "opacity", &number(0.5))?;
                Transition::set(&objects, node_of(&c40)?, "x", &number(300.0))?;
                Transition::set(&objects, node_of(&c41)?, "opacity", &number(0.25))?;
                Ok(())
            },
            Some(&TransitionOptions {
                easing: Easing::EaseInOut,
                ..options(200.0)
            }),
            Some(|| Ok(())),
        )?;
        t.on_finished(|cancelled| log(&format!("basic finished cancelled={cancelled}")))?
            .keep();
        kept.running.push(t);
        Ok(())
    });
    let staged = || -> Step {
        Box::new(|world, kept| {
            let c41 = world.client(41)?;
            let model = Gnoblin::get(&world.core)?;
            let objects = world.objects.clone();
            let t = model.request_transition(
                move || {
                    Transition::set(&objects, node_of(&c41)?, "x", &number(100.0))?;
                    Transition::set(
                        &objects,
                        node_of(&c41)?,
                        "rotation",
                        &TransitionValue {
                            triple: Some(vec![0.0, 0.5, 0.0]),
                            ..TransitionValue::default()
                        },
                    )?;
                    Ok(())
                },
                Some(&TransitionOptions {
                    delay: 50.0,
                    easing: Easing::Linear,
                    autoplay: false,
                    ..options(100.0)
                }),
                Some(|| Ok(())),
            )?;
            t.on_finished(|cancelled| log(&format!("staged finished cancelled={cancelled}")))?
                .keep();
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
    let instant: Step = Box::new(|world, kept| {
        let c42 = world.client(42)?;
        let objects = world.objects.clone();
        let t = Gnoblin::get(&world.core)?.request_transition(
            move || {
                Transition::set(&objects, node_of(&c42)?, "opacity", &number(0.75))?;
                Transition::set(&objects, node_of(&c42)?, "x", &number(20.0))?;
                Ok(())
            },
            Some(&options(0.0)),
            Some(|| Ok(())),
        )?;
        t.on_finished(|cancelled| log(&format!("instant finished cancelled={cancelled}")))?
            .keep();
        kept.running.push(t);
        Ok(())
    });
    let layout: Step = Box::new(|world, _| {
        let workspace = Workspaces::get(&world.objects)?
            .list()?
            .into_iter()
            .next()
            .expect("a default workspace");
        let objects = world.objects.clone();
        Gnoblin::get(&world.core)?.request_transition(
            move || {
                let layout = MasterStackLayout::new_with(
                    &objects,
                    MasterStackLayoutOptions {
                        ratio: Some(0.6),
                        ..MasterStackLayoutOptions::default()
                    },
                )?;
                workspace.set_layout(&layout.as_layout()?)?;
                Ok(())
            },
            Some(&options(300.0)),
            Some(|| Ok(())),
        )?;
        Ok(())
    });
    let style_global: Step = Box::new(|world, _| {
        let style = ClientStyle::of_clients(&world.objects)?;
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
        style.set_backdrop_filter(&[
            BlurFilter { radius: 12.0 }.into(),
            SaturateFilter { amount: 1.2 }.into(),
            BrightnessFilter { amount: 1.1 }.into(),
        ])?;
        style.set_corner_radius(&CornerRadius {
            all: Some(12.0),
            ..CornerRadius::default()
        })?;
        style.set_opacity(0.9)?;
        Ok(())
    });
    let style_selection: Step = Box::new(|world, _| {
        let clients = Gnoblin::get(&world.core)?.clients()?;
        let selected = clients.where_(
            &ClientQuery {
                app_id: Some("nautilus".to_owned()),
                ..ClientQuery::default()
            },
            None,
        )?;
        selected.style()?.set_borders(&[Border {
            width: 3.0,
            color: "#00ff00".to_owned(),
            ..Border::default()
        }])?;
        Ok(())
    });
    let style_client: Step = Box::new(|world, _| {
        let style = world.client(40)?.style()?;
        style.set_opacity(0.5)?;
        style.set_shadows(&[])?;
        Ok(())
    });
    let style_unset: Step = Box::new(|world, _| {
        world.client(40)?.style()?.unset(StyleProperty::Opacity)?;
        let computed = world.client(41)?.computed_style()?;
        log(&format!(
            "computed 41 opacity={} borders={} origin={}",
            computed.opacity,
            computed.borders.len(),
            computed.origin.borders
        ));
        Ok(())
    });
    let style_bad: Step = Box::new(|world, _| {
        match ClientStyle::of_clients(&world.objects)?.set_opacity(2.0) {
            Err(error) => log(&format!("bad opacity: {}", error.message)),
            Ok(()) => log("bad opacity: accepted"),
        }
        let border = Border {
            width: -1.0,
            ..Border::default()
        };
        match world.client(42)?.style()?.set_borders(&[border]) {
            Err(error) => log(&format!("bad border: {}", error.message)),
            Ok(()) => log("bad border: accepted"),
        }
        Ok(())
    });
    let scene: Step = Box::new(|world, kept| {
        let scene = Scene::get(&world.objects)?;
        let camera = Camera3D::new(&world.objects)?;
        camera.set_position(&gnoblin_std::Vec3 { x: 1.0, y: 2.0, z: 3.0 })?;
        camera.set_projection(Projection::Perspective)?;
        camera.set_fov(70.0)?;
        camera.set_near(0.2)?;
        scene.add(&camera.as_scene_node()?)?;
        let c40 = world.client(40)?;
        let view = View::new(&world.objects)?;
        view.set_camera(&camera)?;
        view.set_size(&gnoblin_std::SceneSize {
            width: 400.0,
            height: 300.0,
        })?;
        view.show_client(&c40)?;
        scene.add(&view.as_scene_node()?)?;
        let flat = Camera2D::new(&world.objects)?;
        flat.set_x(10.0)?;
        flat.set_y(20.0)?;
        flat.set_zoom(2.0)?;
        flat.set_roll(0.5)?;
        scene.add(&flat.as_scene_node()?)?;
        log(&format!("camera size={}", camera.size()?));
        if let Err(error) = flat.set_zoom(0.0) {
            log(&format!("bad zoom: {error}"));
        }
        kept.camera = Some(camera);
        kept.flat = Some(flat);
        Ok(())
    });
    let scene_transition: Step = Box::new(|world, kept| {
        let (Some(camera), Some(flat)) = (&kept.camera, &kept.flat) else {
            return Ok(());
        };
        let (camera, flat) = (camera.clone(), flat.clone());
        Gnoblin::get(&world.core)?.request_transition(
            {
                let (camera, flat) = (camera.clone(), flat.clone());
                move || {
                    flat.set_zoom(4.0)?;
                    flat.set_x(30.0)?;
                    flat.set_y(40.0)?;
                    flat.set_roll(1.0)?;
                    camera.set_fov(40.0)?;
                    Ok(())
                }
            },
            Some(&options(100.0)),
            Some(|| Ok(())),
        )?;
        log(&format!("flat zoom={} fov={}", flat.zoom()?, camera.fov()?));
        Ok(())
    });
    let style_shader: Step = Box::new(|world, _| {
        let shader = ShaderFilter {
            params: vec![0.5, 2.0],
            ..ShaderFilter::new(
                "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }"
                    .to_owned(),
            )
        };
        world.client(42)?.style()?.set_filter(&[shader.into()])?;
        Ok(())
    });
    let _ = (GridLayout::new, Filter::Blur);
    vec![
        ("Super+a", basic),
        ("Super+b", staged()),
        ("Super+c", held(|t| Ok(t.set_progress(0.5)?))),
        ("Super+d", held(|t| Ok(t.play()?))),
        ("Super+e", held(|t| Ok(t.pause()?))),
        ("Super+f", held(|t| Ok(t.reverse()?))),
        ("Super+g", staged()),
        ("Super+h", held(|t| Ok(t.cancel()?))),
        ("Super+i", instant),
        ("Super+j", layout),
        ("Super+k", style_global),
        ("Super+l", style_selection),
        ("Super+m", style_client),
        ("Super+n", style_unset),
        ("Super+o", style_bad),
        ("Super+p", scene),
        ("Super+q", scene_transition),
        ("Super+r", style_shader),
    ]
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let world = World {
        core: g.objects()?,
        objects: g.objects()?.std()?,
    };
    let kept = Rc::new(RefCell::new(Kept::default()));
    for (chord, step) in steps() {
        let (world, kept) = (world.clone(), Rc::clone(&kept));
        g.bind(chord, move || {
            if let Err(error) = step(&world, &mut kept.borrow_mut()) {
                log(&format!("{chord} failed: {error}"));
            }
        })?;
    }
    eprintln!("anim shell is running");
    Ok(())
}
