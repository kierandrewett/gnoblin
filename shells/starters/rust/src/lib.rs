// The shell is this library: build it, and `gnoblin` loads the `.so` and reloads it when you rebuild.
// It runs once against live objects such as `Workspaces`, `Gnoblin::input()` and `Gnoblin::clients()`.
// Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
// This file does what a shell with no shell file does, so a copy you leave unchanged behaves like the default.
// Build it with `cargo build`; see the shell API pages for each object's details.

use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;
use gnoblin_std::{
    Decorations, FloatingLayout, Launcher, MoveBinding, MoveBindingOptions, NotRespondingAlerts, ResizeBinding,
    ResizeBindingOptions, ShowOptions, StripLayout, WindowMenus, WindowState, Workspace, WorkspaceOptions,
    WorkspaceSlideBinding, WorkspaceSlideBindingOptions, Workspaces,
};
// To use the examples below, add what they name to the import above.
// use gnoblin_std::MasterStackLayout;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let model = Rc::new(Gnoblin::get(&g.objects()?)?);

    // Four floating workspaces. The first is shown until another is selected.
    // Choose a layout: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout or StripLayout.
    // A handle releases its object when it is dropped, so what the shell leaves in place (a layout, a workspace, a binding)
    // is forgotten: the object then lasts until the shell reloads.
    let workspaces = Workspaces::get(&objects)?;
    for id in ["1", "2", "3", "4"] {
        let floating = FloatingLayout::new(&objects)?;
        let name = format!("Workspace {id}");
        let options = WorkspaceOptions {
            name: Some(name),
            gaps: None,
            layout: Some(floating.as_layout()?),
        };
        let workspace = Workspace::new_with(&objects, id, options)?;
        workspaces.add(&workspace)?;
        std::mem::forget(floating);
        std::mem::forget(workspace);
    }

    // Add a titlebar to each window, the window menu and the not-responding alert, and answer a window that asks to be
    // maximised, minimised or made fullscreen. Remove a call to disable that part.
    // The library moves the keyboard focus on its own, as the seat's focus settings say: a click or a new window takes it.
    Decorations::get(&objects)?.start()?;
    WindowMenus::get(&objects)?.start()?;
    NotRespondingAlerts::get(&objects)?.start()?;
    WindowState::get(&objects)?.start()?;

    let seat = model.input()?.primary()?;

    // Super+1 .. Super+9 show the workspace at that position on the display under the pointer. A key past the last
    // workspace does nothing.
    for position in 1..=9_usize {
        let library = objects.clone();
        seat.bind(
            &format!("Super+{position}"),
            Some(move |_event| {
                if let Some(target) = Workspaces::get(&library)?.list()?.get(position - 1) {
                    target.show(Some(&ShowOptions {
                        duration: 250.0,
                        progress: 0.0,
                        output: None,
                    }))?;
                }
                Ok(true)
            }),
            None,
        )?;
    }

    // Super+Left and Super+Right move focus between StripLayout columns on the display under the pointer. They do nothing
    // on other layouts.
    for (key, right) in [("Super+Left", false), ("Super+Right", true)] {
        let model = Rc::clone(&model);
        let library = objects.clone();
        seat.bind(
            key,
            Some(move |_event| {
                let Some(display) = model.input()?.primary()?.active_display()? else {
                    return Ok(true);
                };
                let Some(viewport) = display.viewport()? else {
                    return Ok(true);
                };
                let Some(content) = viewport.content()? else {
                    return Ok(true);
                };
                let Some(layout) = content.layout()? else {
                    return Ok(true);
                };
                if let Some(strip) = StripLayout::downcast(&library, &layout)? {
                    if right {
                        strip.focus_right()?;
                    } else {
                        strip.focus_left()?;
                    }
                }
                Ok(true)
            }),
            None,
        )?;
    }

    // Three-finger sideways swipes switch workspaces.
    let slide = WorkspaceSlideBindingOptions {
        priority: None,
        trigger: Some("Swipe3".to_owned()),
        travel: None,
        commit: None,
        duration: None,
    };
    let slide = WorkspaceSlideBinding::new_with(&objects, slide)?;
    slide.install(None)?;
    std::mem::forget(slide);

    // Super with the left button drags a window, and Super with the right button resizes it from the nearest corner.
    let moving = MoveBinding::new_with(
        &objects,
        MoveBindingOptions {
            priority: None,
            trigger: None,
            snap: None,
            snap_margin: None,
        },
    )?;
    moving.install(None)?;
    std::mem::forget(moving);
    let resizing = ResizeBinding::new_with(
        &objects,
        ResizeBindingOptions {
            priority: None,
            trigger: None,
            grip: None,
        },
    )?;
    resizing.install(None)?;
    std::mem::forget(resizing);

    // Volume, microphone and media player keys: each key runs one command.
    let media_keys: [(&str, &[&str]); 7] = [
        (
            "XF86AudioRaiseVolume",
            &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
        ),
        (
            "XF86AudioLowerVolume",
            &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
        ),
        (
            "XF86AudioMute",
            &["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
        ),
        (
            "XF86AudioMicMute",
            &["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
        ),
        ("XF86AudioPlay", &["playerctl", "play-pause"]),
        ("XF86AudioNext", &["playerctl", "next"]),
        ("XF86AudioPrev", &["playerctl", "previous"]),
    ];
    for (key, command) in media_keys {
        let model = Rc::clone(&model);
        let command: Vec<String> = command.iter().map(|word| (*word).to_owned()).collect();
        seat.bind(
            key,
            Some(move |_event| {
                model.spawn(&command, None)?;
                Ok(true)
            }),
            None,
        )?;
    }

    // Super+space opens the launcher, with a command entry in the middle of the screen.
    let library = objects.clone();
    seat.bind(
        "Super+space",
        Some(move |_event| {
            // Return starts what you typed.
            let shell = g.clone();
            Launcher::open(
                &library,
                Some(move |command: String| {
                    let _ = shell.spawn(&[command], None);
                    Ok(())
                }),
            )?;
            Ok(true)
        }),
        None,
    )?;

    // Add custom key bindings here. A key already bound above is refused; remove that binding first.
    // seat.bind("Super+Return", Some(|_event| { let _ = g.spawn(&["foot".to_owned()], None); Ok(true) }), None)?;

    Ok(())
}
