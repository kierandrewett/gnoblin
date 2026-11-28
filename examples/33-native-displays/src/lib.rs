//! Example 33: output profiles, displays and viewports of `libgnoblin-std`.
//!
//! The library keeps one `single` display for every output, so the list of displays says what each monitor shows. This shell:
//!
//! - makes two workspaces and shows the second one on the display of the output, through the display's viewport;
//! - adds an output profile for that output (a scale), and hears the compositor take it (`applied`);
//! - adds a display of its own over the profile, which takes the output from the display of the library, and removes it again, which
//!   gives the output a display of the library's again;
//! - tries what the checks refuse: a scale out of range, a display with nothing to present, a second display over an output that has one.
//!
//! Every line it logs starts with `displays:`. Build it with `cargo build -p gnoblin-example-native-displays` and run it with
//! `gnoblin ./target/debug/libnative_displays.so`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_std::prelude::*;
use gnoblin_std::{
    Display, DisplayManager, DisplaySelector, OutputMatch, OutputProfile, ProfileSettings, Viewport, Workspace,
    Workspaces,
};

/// The displays as `id:mode:outputs:workspace` words, sorted.
fn listed(manager: &DisplayManager) -> String {
    let mut words: Vec<String> = manager
        .displays()
        .unwrap_or_default()
        .iter()
        .map(|display| {
            let shown = display
                .viewport()
                .ok()
                .flatten()
                .and_then(|viewport| viewport.content().ok().flatten())
                .and_then(|workspace| workspace.id().ok())
                .unwrap_or_else(|| "none".to_owned());
            format!(
                "{}:{}:{}:{shown}",
                display.id().unwrap_or_default(),
                display.mode().map_or("single", |mode| mode.as_str()),
                display.outputs().map_or(0, |outputs| outputs.len()),
            )
        })
        .collect();
    words.sort();
    words.join(" ")
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let workspaces = Workspaces::get(&objects)?;
    let one = Rc::new(Workspace::new(&objects, "one")?);
    let two = Rc::new(Workspace::new(&objects, "two")?);
    workspaces.add(&one)?;
    workspaces.add(&two)?;
    let manager = Rc::new(DisplayManager::get(&objects)?);

    // 0: waiting for the library's first display; 1: the profile is added; 2: the profile was answered.
    let phase = Rc::new(Cell::new(0_u8));
    let profile_id: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let on_changed = {
        let (manager, phase, profile_id, objects) = (
            Rc::clone(&manager),
            Rc::clone(&phase),
            Rc::clone(&profile_id),
            objects.clone(),
        );
        move || {
            if phase.get() != 0 {
                return;
            }
            let Some(display) = manager.displays().unwrap_or_default().into_iter().next() else {
                return;
            };
            phase.set(1);
            g.log(&format!("displays: listed {}", listed(&manager)));
            let Ok(Some(viewport)) = display.viewport() else {
                return;
            };
            if viewport.set_content(&two).is_ok() {
                let shown = viewport
                    .content()
                    .ok()
                    .flatten()
                    .and_then(|workspace| workspace.id().ok())
                    .unwrap_or_default();
                g.log(&format!(
                    "displays: the display {} shows {shown}",
                    display.id().unwrap_or_default()
                ));
            }
            let id = display.id().unwrap_or_default();
            let connector = id.strip_prefix("display-").unwrap_or(&id).to_owned();
            let made = OutputProfile::new(
                &objects,
                &OutputMatch {
                    connector: Some(connector),
                    ..OutputMatch::default()
                },
            );
            let Ok(profile) = made else { return };
            let settings = ProfileSettings {
                mode: Some("preferred".to_owned()),
                scale: Some(1.25),
                ..ProfileSettings::default()
            };
            if let Err(error) = profile
                .set_settings(&settings)
                .and_then(|()| manager.add_profile(&profile))
            {
                g.log(&format!("displays: the profile is refused: {error}"));
                return;
            }
            *profile_id.borrow_mut() = profile.id().unwrap_or_default();
            g.log(&format!(
                "displays: the profile {} is in the manager: {}",
                profile.id().unwrap_or_default(),
                profile.listed().unwrap_or(false)
            ));
        }
    };
    manager.on_changed(on_changed.clone())?.keep();

    let on_applied = {
        let (manager, phase, profile_id, objects, one) = (
            Rc::clone(&manager),
            Rc::clone(&phase),
            Rc::clone(&profile_id),
            objects.clone(),
            Rc::clone(&one),
        );
        move |_output: u64, error: String| {
            if phase.get() != 1 {
                return;
            }
            phase.set(2);
            g.log(&format!(
                "displays: the compositor answered the request: {}",
                if error.is_empty() { "ok" } else { &error }
            ));
            let id = profile_id.borrow().clone();
            let Ok(Some(profile)) = manager.profile_named(&id) else {
                return;
            };
            if let Ok(Some(settings)) = profile.settings() {
                g.log(&format!(
                    "displays: the profile reads back scale {} mode {}",
                    settings.scale.unwrap_or_default(),
                    settings.mode.unwrap_or_default()
                ));
            }
            let over = |name: &str| -> gnoblin_std::Result<Display> {
                let selector = DisplaySelector {
                    profile: manager.profile_named(&id)?,
                    ..DisplaySelector::default()
                };
                let display = Display::new(&objects, &[selector])?;
                display.set_id(name)?;
                Ok(display)
            };
            let mine = over("main");
            if let Ok(mine) = &mine {
                let given = Viewport::new(&objects).and_then(|viewport| {
                    viewport.set_content(&one)?;
                    mine.set_viewport(&viewport)
                });
                if let Err(error) = given.and_then(|()| manager.add_display(mine)) {
                    g.log(&format!("displays: the display is refused: {error}"));
                }
            }
            g.log(&format!("displays: listed {}", listed(&manager)));

            match over("rival").and_then(|rival| Ok(manager.add_display(&rival)?)) {
                Ok(()) => g.log("displays: refused overlap: no"),
                Err(error) => g.log(&format!("displays: refused overlap: {error}")),
            }
            if let Ok(mine) = &mine {
                g.log(&format!(
                    "displays: removed main: {}",
                    manager.remove_display(mine).unwrap_or(false)
                ));
            }
            g.log(&format!("displays: listed {}", listed(&manager)));

            let refused_scale = OutputProfile::new(
                &objects,
                &OutputMatch {
                    connector: Some("HEADLESS-1".to_owned()),
                    ..OutputMatch::default()
                },
            )
            .and_then(|profile| {
                profile.set_settings(&ProfileSettings {
                    scale: Some(9.0),
                    ..ProfileSettings::default()
                })
            });
            match refused_scale {
                Ok(()) => g.log("displays: refused scale: no"),
                Err(error) => g.log(&format!("displays: refused scale: {error}")),
            }
            match Display::new(&objects, &[]) {
                Ok(_) => g.log("displays: refused selectors: no"),
                Err(error) => g.log(&format!("displays: refused selectors: {error}")),
            }
            g.log("displays: done");
        }
    };
    manager.on_applied(on_applied)?.keep();

    g.log("displays: running");
    on_changed();
    Ok(())
}
