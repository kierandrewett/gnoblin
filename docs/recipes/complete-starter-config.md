# Complete starter config

Run `gnoblinctl config init` to write the stock default in the language you choose: `shell.ts` (the default), `shell.py` with
`--language python`, or a Rust library crate with `--language rust`. This page shows the same file in all three languages.

The file is the whole configuration: the shell runs it from top to bottom, and with no shell file it runs this same file.
Nothing is layered under your file, so keep the lines you want to keep.

The file declares four floating workspaces, starts the titlebar, window menu and not-responding alert, and binds the workspace
keys, the strip focus keys, the three-finger swipe, the launcher key and the media keys, each as a call on a live object. The
Rust file leaves out the two strip focus keys. It is this, in full:

::: tabs

```typescript
// The shell reloads `~/.config/gnoblin/shell.ts` when you save it.
// It runs top to bottom against live objects such as `gnoblin.workspaces`, `gnoblin.input` and `gnoblin.clients`.
// Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
// A shell with no `shell.ts` runs this same file, so a copy you leave unchanged behaves like the default.
// Check changes with `gnoblinctl check --types shell.ts`; see the shell API pages for each object's details.

import { gnoblin } from "gnoblin-core";
import {
    Decorations,
    FloatingLayout,
    Launcher,
    MoveBinding,
    NotRespondingAlerts,
    ResizeBinding,
    StripLayout,
    WindowMenus,
    WindowState,
    Workspace,
    WorkspaceSlideBinding,
} from "gnoblin-std";
// To use the examples below, add what they name to the import above.
// import { MasterStackLayout } from "gnoblin-std";

// Four floating workspaces. The first is shown until another is selected.
// Choose a layout: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout or StripLayout.
for (const id of ["1", "2", "3", "4"]) {
    gnoblin.workspaces.add(new Workspace(id, { name: `Workspace ${id}`, layout: new FloatingLayout() }));
}

// gnoblin.workspaces.add(new Workspace("work", { name: "Work", layout: new MasterStackLayout() }));

// Add a titlebar to each window, the window menu and the not-responding alert, and answer a window that asks to be
// maximised, minimised or made fullscreen. Remove a call to disable that part.
// The library moves the keyboard focus on its own, as the seat's focus settings say: a click or a new window takes it.
// To write a focus policy of your own, import FocusPolicy and call `FocusPolicy.get().stop()` first.
Decorations.get().start();
WindowMenus.get().start();
NotRespondingAlerts.get().start();
WindowState.get().start();

const input = gnoblin.input.primary;

// Super+1 .. Super+9 show the workspace at that position on the display under the pointer. A key past the last
// workspace does nothing.
for (let position = 1; position <= 9; position++) {
    input.bind(`Super+${position}`, () => {
        const target = gnoblin.workspaces.list[position - 1];
        if (target !== undefined) {
            gnoblin.workspaces.show(target, { duration: 250 });
        }
    });
}

// Super+Left and Super+Right move focus between StripLayout columns on the display under the pointer. They do nothing
// on other layouts.
const moveColumn = (side: "focusLeft" | "focusRight") => {
    const layout = input.activeDisplay?.viewport?.content?.layout;
    if (layout instanceof StripLayout) {
        layout[side]();
    }
};
input.bind("Super+Left", () => moveColumn("focusLeft"));
input.bind("Super+Right", () => moveColumn("focusRight"));

// Three-finger sideways swipes switch workspaces.
new WorkspaceSlideBinding({ trigger: "Swipe3" }).install();

// Super with the left button drags a window, and Super with the right button resizes it from the nearest corner.
new MoveBinding().install();
new ResizeBinding().install();

// Volume, microphone and media player keys: each key runs one command.
const mediaKeys: Record<string, string[]> = {
    XF86AudioRaiseVolume: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
    XF86AudioLowerVolume: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
    XF86AudioMute: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
    XF86AudioMicMute: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
    XF86AudioPlay: ["playerctl", "play-pause"],
    XF86AudioNext: ["playerctl", "next"],
    XF86AudioPrev: ["playerctl", "previous"],
};
for (const [key, command] of Object.entries(mediaKeys)) {
    input.bind(key, () => {
        gnoblin.spawn(command);
    });
}

// Super+space opens the launcher, with a command entry in the middle of the screen.
input.bind("Super+space", () => {
    Launcher.open();
});

// Add custom key bindings here. A key already bound above is refused; remove that binding first.
// input.bind("Super+Return", () => { gnoblin.spawn(["foot"]); });
// input.bind("Super+q", () => { input.focusedClient?.close(); });

// Set a style property to change how each window looks.
// gnoblin.clients.style.cornerRadius = 12;
```

```python
# The shell reloads `~/.config/gnoblin/shell.py` when you save it.
# It runs top to bottom against live objects such as `Workspaces`, `Gnoblin.get().input` and `Gnoblin.get().clients`.
# Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
# This file does what a shell with no shell file does, so a copy you leave unchanged behaves like the default.
# Check changes with `gnoblinctl check shell.py`; see the shell API pages for each object's details.

from gnoblin import gnoblin
from gnoblin import Gnoblin
from gnoblin.std import (
    Decorations,
    FloatingLayout,
    InputContextAugment,
    Launcher,
    MoveBinding,
    NotRespondingAlerts,
    ResizeBinding,
    ShowOptions,
    StripLayout,
    WindowMenus,
    WindowState,
    Workspace,
    Workspaces,
    WorkspaceSlideBinding,
)

# To use the examples below, add what they name to the import above.
# from gnoblin.std import MasterStackLayout

# Four floating workspaces. The first is shown until another is selected.
# Choose a layout: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout or StripLayout.
for workspace_id in ["1", "2", "3", "4"]:
    Workspaces.get().add(Workspace(workspace_id, name=f"Workspace {workspace_id}", layout=FloatingLayout()))

# Workspaces.get().add(Workspace("work", name="Work", layout=MasterStackLayout()))

# Add a titlebar to each window, the window menu and the not-responding alert, and answer a window that asks to be
# maximised, minimised or made fullscreen. Remove a call to disable that part.
# The library moves the keyboard focus on its own, as the seat's focus settings say: a click or a new window takes it.
Decorations.get().start()
WindowMenus.get().start()
NotRespondingAlerts.get().start()
WindowState.get().start()

input = Gnoblin.get().input.primary


# Super+1 .. Super+9 show the workspace at that position on the display under the pointer. A key past the last
# workspace does nothing.
def show_workspace(position):
    def action(_event):
        items = Workspaces.get().list
        if position <= len(items):
            items[position - 1].show(ShowOptions(duration=250.0))

    return action


for position in range(1, 10):
    input.bind(f"Super+{position}", show_workspace(position))


# Super+Left and Super+Right move focus between StripLayout columns on the display under the pointer. They do nothing
# on other layouts.
def move_column(side):
    def action(_event):
        display = InputContextAugment.get().active_display(input)
        viewport = display.viewport if display is not None else None
        layout = viewport.content.layout if viewport is not None and viewport.content is not None else None
        if isinstance(layout, StripLayout):
            getattr(layout, side)()

    return action


input.bind("Super+Left", move_column("focus_left"))
input.bind("Super+Right", move_column("focus_right"))

# Three-finger sideways swipes switch workspaces.
WorkspaceSlideBinding(trigger="Swipe3").install()

# Super with the left button drags a window, and Super with the right button resizes it from the nearest corner.
MoveBinding().install()
ResizeBinding().install()

# Volume, microphone and media player keys: each key runs one command.
media_keys = {
    "XF86AudioRaiseVolume": ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
    "XF86AudioLowerVolume": ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
    "XF86AudioMute": ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
    "XF86AudioMicMute": ["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
    "XF86AudioPlay": ["playerctl", "play-pause"],
    "XF86AudioNext": ["playerctl", "next"],
    "XF86AudioPrev": ["playerctl", "previous"],
}


def run_command(command):
    def action(_event):
        gnoblin.spawn(command)

    return action


for key, command in media_keys.items():
    input.bind(key, run_command(command))


# Super+space opens the launcher, with a command entry in the middle of the screen.
def open_launcher(_event):
    Launcher.open()


input.bind("Super+space", open_launcher)

# Add custom key bindings here. A key already bound above is refused; remove that binding first.
# input.bind("Super+Return", run_command(["foot"]))
# input.bind("Super+q", lambda _event: input.focused_client().close())

# Set a style property to change how each window looks.
# Gnoblin.get().clients.style.corner_radius = 12
```

```rust
// The shell is this library: build it, and `gnoblin` loads the `.so` and reloads it when you rebuild.
// It runs once against live objects such as `Workspaces`, `Gnoblin::input()` and `Gnoblin::clients()`.
// Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
// This file does what a shell with no shell file does, so a copy you leave unchanged behaves like the default.
// Build it with `cargo build`; see the shell API pages for each object's details.

use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::{
    Decorations, FloatingLayout, Launcher, MoveBinding, MoveBindingOptions, NotRespondingAlerts, ResizeBinding,
    ResizeBindingOptions, ShowOptions, WindowMenus, WindowState, Workspace, WorkspaceOptions, WorkspaceSlideBinding,
    WorkspaceSlideBindingOptions, Workspaces,
};
use gnoblin_std::prelude::*;
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
                    let _ = shell.spawn(&command);
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
```

:::

To add a terminal shortcut, add one line after the stock keys; replace `foot` with your terminal.

::: tabs

```typescript
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin import gnoblin
from gnoblin import Gnoblin

def open_terminal(_event):
    gnoblin.spawn(["foot"])


Gnoblin.get().input.primary.bind("Super+Return", open_terminal)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let input = model.input()?;
    let seat = input.primary()?;
    let shell = g.clone();
    seat.bind("Super+Return", Some(move |_event| {
        let _ = shell.spawn(&["foot".to_owned()], None);
        Ok(true)
    }), None)?;
    Ok(())
}
```

:::

Save the file and the running shell reloads it. A key is bound once at one priority, so a second `bind` of a trigger the
file already binds is refused: delete the stock line first.

To dim unfocused windows, see [dim unfocused windows](/recipes/dim-unfocused-windows). The file does not install or start a
desktop shell; choose one in [shell setup](/bring-your-own-shell).
