//! Example 37: the members the std library adds to the `gnoblin` root of the core.
//!
//! `use gnoblin_std::prelude::*;` brings the extension traits `ModelRootStdExt` and `OutputCollectionStdExt` into scope, which give the
//! core's typed root `workspaces`, `displays`, `scene`, `gestures` and `request_transition`, and its output collections `profiles`. A shell
//! that does not use the std prelude (example 35) has none of them, and links no std library. This shell:
//!
//! - adds two workspaces to the root's workspaces, reads the scene and the gestures, and runs a transition through `request_transition`;
//! - when the library has a display for an output: reads the area windows are laid out in, shows each workspace with `show` and
//!   `show_by_id`, and tries what the checks refuse (a workspace that is not in the list, an id that is not there);
//! - adds an output profile to the outputs' profiles, adds a display of its own over it, swaps it for another with `displays.replace`,
//!   and tries to replace a display of the library's, which is refused.
//!
//! Every line it logs starts with `stdroot:`, and the TypeScript and Python shells of the same example log the same lines. Build it with
//! `cargo build -p gnoblin-example-native-std-root` and run it with `gnoblin ./target/debug/libnative_std_root.so`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Display, DisplaySelector, OutputMatch, OutputProfile, TransitionOptions, Workspace};
use gnoblin_std::prelude::*;

/// The options of a transition of `duration` milliseconds.
fn lasting(duration: f64) -> TransitionOptions {
    TransitionOptions {
        duration,
        ..TransitionOptions::default()
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let workspaces = Rc::new(model.workspaces()?);
    let displays = Rc::new(model.displays()?);
    let Some(outputs) = model.outputs()? else {
        g.log("stdroot: the core has no outputs");
        return Ok(());
    };
    let profiles = Rc::new(outputs.profiles()?);

    let main = Rc::new(Workspace::new(&objects, "main")?);
    let side = Rc::new(Workspace::new(&objects, "side")?);
    let ghost = Rc::new(Workspace::new(&objects, "ghost")?);
    workspaces.add(&main)?;
    workspaces.add(&side)?;
    let held: Vec<String> = workspaces
        .list()?
        .iter()
        .map(|workspace| workspace.id().unwrap_or_default())
        .collect();
    g.log(&format!("stdroot: workspaces hold {}", held.join(", ")));

    g.log(&format!(
        "stdroot: scene metres per unit is 1: {}",
        model.scene()?.metres_per_unit()? == 1.0
    ));
    g.log(&format!(
        "stdroot: gestures swipe threshold is positive: {}",
        model.gestures()?.swipe_threshold()? > 0.0
    ));

    let ran = Rc::new(Cell::new(false));
    let marker = Rc::clone(&ran);
    model.request_transition(
        move || {
            marker.set(true);
            Ok(())
        },
        Some(&lasting(0.0)),
        Some(|| Ok(())),
    )?;
    g.log(&format!("stdroot: requestTransition ran its work: {}", ran.get()));
    match model.request_transition(|| Ok(()), Some(&lasting(-1.0)), Some(|| Ok(()))) {
        Ok(_) => g.log("stdroot: requestTransition refused: no"),
        Err(error) => g.log(&format!("stdroot: requestTransition refused: {error}")),
    }

    let done = Rc::new(Cell::new(false));
    let on_changed = {
        let (workspaces, displays, profiles, objects, done) = (
            Rc::clone(&workspaces),
            Rc::clone(&displays),
            Rc::clone(&profiles),
            objects.clone(),
            Rc::clone(&done),
        );
        move || {
            if done.get() || displays.length().unwrap_or(0) == 0 {
                return;
            }
            done.set(true);
            let say = |text: String| g.log(&format!("stdroot: {text}"));
            let area = workspaces.area().ok().flatten();
            say(format!(
                "the area is known: {}",
                area.is_some_and(|area| area.width > 0.0 && area.height > 0.0)
            ));

            let _ = workspaces.show(&side, None);
            say("show side: true".to_owned());
            say(format!("side is active: {}", side.active().unwrap_or(false)));
            let _ = workspaces.show_by_id("main", None);
            say("show main by id: true".to_owned());
            say(format!("main is active: {}", main.active().unwrap_or(false)));
            match workspaces.show(&ghost, None) {
                Ok(()) => say("refused ghost: no".to_owned()),
                Err(error) => say(format!("refused ghost: {error}")),
            }
            match workspaces.show_by_id("nope", None) {
                Ok(()) => say("refused nope: no".to_owned()),
                Err(error) => say(format!("refused nope: {error}")),
            }

            let ids = || {
                let mut ids: Vec<String> = displays
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .map(|display| display.id().unwrap_or_default())
                    .collect();
                ids.sort();
                ids.join(", ")
            };
            let library = displays.list().unwrap_or_default();
            let Some(library) = library.first() else { return };
            say(format!("displays hold {}", displays.length().unwrap_or(0)));
            let id = library.id().unwrap_or_default();
            let connector = id.strip_prefix("display-").unwrap_or(&id).to_owned();
            let Ok(profile) = OutputProfile::new(
                &objects,
                &OutputMatch {
                    connector: Some(connector),
                    ..OutputMatch::default()
                },
            ) else {
                return;
            };
            let _ = profiles.add(&profile);
            say(format!("profiles hold {}", profiles.length().unwrap_or(0)));
            let profile_id = profile.id().unwrap_or_default();

            let over = |name: &str| -> gnoblin_std::Result<Display> {
                let selector = DisplaySelector {
                    profile: profiles.by_id(&profile_id)?,
                    ..DisplaySelector::default()
                };
                let display = Display::new(&objects, &[selector])?;
                display.set_id(name)?;
                Ok(display)
            };
            match over("other").and_then(|other| Ok(displays.replace(library, &other)?)) {
                Ok(()) => say("refused library display: no".to_owned()),
                Err(error) => say(format!("refused library display: {error}")),
            }
            let Ok(mine) = over("mine") else { return };
            let _ = displays.add(&mine);
            say(format!("displays hold {}", ids()));
            if let Ok(next) = over("next") {
                let _ = displays.replace(&mine, &next);
            }
            say(format!("replaced mine with next: {}", ids()));
            let removed = displays
                .by_id("next")
                .ok()
                .flatten()
                .and_then(|next| displays.remove(&next).ok())
                .unwrap_or(false);
            say(format!("removed next: {removed}"));
            say(format!(
                "after removing, displays hold {}",
                displays.length().unwrap_or(0)
            ));
            say(format!(
                "removed profile: {}",
                profiles.remove(&profile).unwrap_or(false)
            ));
            say(format!("profiles hold {}", profiles.length().unwrap_or(0)));
            say("done".to_owned());
        }
    };
    displays.on_changed(on_changed.clone())?.keep();

    g.log("stdroot: running");
    on_changed();
    Ok(())
}
