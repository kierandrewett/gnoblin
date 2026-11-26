# Settings in your shell

A shell has no config object. It is a program that runs against live objects, and a setting is a property you set on one
of them. This page lists what you can set, one example for each kind of object, and the settings that have no object yet.

The examples are the same in the three languages a shell can be written in: TypeScript in `shell.ts`, Python in
`shell.py` and Rust in a shell library.

There is no config object to fill in. Delete a statement and that part of the setup is gone after the next reload. See
[the TypeScript shell](/typescript-shell), [the Python shell](/python-shell) and [the Rust shell](/rust-shell) for where
the file lives and how it reloads.

Each language spells the same members its own way:

- TypeScript uses `camelCase` properties, such as `repeatRate`.
- Python uses `snake_case` attributes, such as `repeat_rate`.
- Rust has a getter and a `set_` method for each property, such as `repeat_rate()` and `set_repeat_rate(..)`. Each returns a
  `Result`.

The tables on this page use the TypeScript spelling. The [API reference](/reference) lists every member in each language.

## Device settings

Devices are in `gnoblin.input.devices`. A `Keyboard`, `Mouse` or `Touchpad` has the settings below, and `reset(name)`
puts one back to the system's value. The compositor keeps one setting for each kind of device, so a write on one mouse
is the setting of every mouse.

::: tabs

```typescript
import { gnoblin, Keyboard, Mouse, Touchpad } from "gnoblin-core/generated/objects";

for (const device of gnoblin.input.devices) {
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
    if (device instanceof Mouse) {
        device.sensitivity = 0.5;
        device.acceleration = "flat";
    }
    if (device instanceof Touchpad) {
        device.naturalScroll = true;
        device.tapToClick = true;
    }
}
```

```python
from gnoblin.objects.model import Gnoblin, Keyboard, Mouse, Touchpad

gnoblin = Gnoblin.get()

for device in gnoblin.input.devices:
    if isinstance(device, Keyboard):
        device.repeat_rate = 40
        device.repeat_delay = 250
    if isinstance(device, Mouse):
        device.sensitivity = 0.5
        device.acceleration = "flat"
    if isinstance(device, Touchpad):
        device.natural_scroll = True
        device.tap_to_click = True
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{
    AccelerationProfile, Keyboard, KeyboardOptions, Mouse, MouseOptions, Touchpad, TouchpadOptions,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    // The compositor keeps one setting for each kind of device, so one handle of a kind sets every device of it.
    let _keyboard = Keyboard::new_with(
        &objects,
        KeyboardOptions {
            repeat_rate: Some(40.0),
            repeat_delay: Some(250),
            ..KeyboardOptions::default()
        },
    )?;
    let _mouse = Mouse::new_with(
        &objects,
        MouseOptions {
            sensitivity: Some(0.5),
            acceleration: Some(AccelerationProfile::Flat),
            ..MouseOptions::default()
        },
    )?;
    let _touchpad = Touchpad::new_with(
        &objects,
        TouchpadOptions {
            natural_scroll: Some(true),
            tap_to_click: Some(true),
            ..TouchpadOptions::default()
        },
    )?;
    Ok(())
}
```

:::

Each device page lists the properties and their ranges: [keyboard](/config/configure/input/keyboard),
[mouse](/config/configure/input/mouse) and [touchpad](/config/configure/input/touchpad).

## Outputs

An `OutputProfile` is made from what it matches, and its `settings` describe the mode, scale, transform and position of
the output that matches it. It survives hotplug. Add it to `OutputProfiles.get()`, the list `gnoblin.outputs.profiles` holds. See
[monitors](/config/configure/monitors).

::: tabs

```typescript
import { OutputProfile, OutputProfiles } from "gnoblin-std/generated/objects";

const profiles = OutputProfiles.get();
const main = new OutputProfile(
    { connector: "DP-1" },
    { settings: { mode: "3440x1440@144", scale: 1.25, position: { kind: "absolute", x: 0, y: 0 } } },
);
profiles.add(main);
profiles.add(
    new OutputProfile(
        { connector: "HDMI-A-1" },
        { settings: { position: { kind: "right_of", profile: main, align: "top" } } },
    ),
);
```

```python
from gnoblin.objects.std import OutputMatch, OutputPosition, OutputProfile, OutputProfiles, PositionKind, ProfileSettings

profiles = OutputProfiles.get()

main = OutputProfile(
    OutputMatch(connector="DP-1"),
    settings=ProfileSettings(
        mode="3440x1440@144",
        scale=1.25,
        position=OutputPosition(PositionKind.ABSOLUTE, x=0, y=0),
    ),
)
profiles.add(main)
profiles.add(
    OutputProfile(
        OutputMatch(connector="HDMI-A-1"),
        settings=ProfileSettings(position=OutputPosition(PositionKind.RIGHT_OF, profile=main, align="top")),
    )
)
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{
    OutputMatch, OutputPosition, OutputProfile, OutputProfileOptions, PositionKind, ProfileSettings,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let outputs = model.outputs()?;
    let profiles = outputs.profiles()?;

    let main = OutputProfile::new_with(
        &objects,
        &OutputMatch {
            connector: Some("DP-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            settings: Some(ProfileSettings {
                mode: Some("3440x1440@144".to_owned()),
                scale: Some(1.25),
                position: Some(OutputPosition {
                    x: Some(0),
                    y: Some(0),
                    ..OutputPosition::new(PositionKind::Absolute)
                }),
                ..ProfileSettings::default()
            }),
            ..OutputProfileOptions::default()
        },
    )?;
    profiles.add(&main)?;
    let side = OutputProfile::new_with(
        &objects,
        &OutputMatch {
            connector: Some("HDMI-A-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            settings: Some(ProfileSettings {
                position: Some(OutputPosition {
                    profile: Some(main),
                    align: Some("top".to_owned()),
                    ..OutputPosition::new(PositionKind::RightOf)
                }),
                ..ProfileSettings::default()
            }),
            ..OutputProfileOptions::default()
        },
    )?;
    profiles.add(&side)?;
    Ok(())
}
```

:::

## Window style

`gnoblin.clients.style` is the look of every window. A workspace has its own `workspace.clients.style`, and
`gnoblin.clients.where(query)` selects some windows. See [window style](/js-style).

::: tabs

```typescript
import { ClientStyle } from "gnoblin-std/generated/objects";

ClientStyle.ofClients().cornerRadius = { all: 12 };
ClientStyle.ofClients().where({ appId: "foot" }).opacity = 0.9;
```

```python
from gnoblin.objects.std import ClientStyle, CornerRadius, StyleQuery

ClientStyle.of_clients().corner_radius = CornerRadius(all=12)
ClientStyle.of_clients().where(StyleQuery(app_id="foot")).opacity = 0.9
```

```rust
use gnoblin_core::objects::{ClientQuery, ModelRoot};
use gnoblin_std::objects::CornerRadius;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let clients = model.clients()?;
    clients.style()?.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    let foot = ClientQuery {
        app_id: Some("foot".to_owned()),
        ..ClientQuery::default()
    };
    clients.where_(&foot, None)?.style()?.set_opacity(0.9)?;
    Ok(())
}
```

:::

## Workspaces and gaps

A workspace is an object with a layout. The `gaps` of a workspace set its outer and inner gap, and a layout's own `gap`
overrides the inner one. See [workspaces](/shell-api/workspaces).

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import { MasterStackLayout, Workspace } from "gnoblin-std/generated/objects";

gnoblin.workspaces.add(
    new Workspace("dev", {
        name: "Development",
        layout: new MasterStackLayout({ gap: 12 }),
        gaps: { outer: 8 },
    }),
);
```

```python
from gnoblin.objects.std import Gaps, MasterStackLayout, Workspace, Workspaces

Workspaces.get().add(
    Workspace(
        "dev",
        name="Development",
        layout=MasterStackLayout(gap=12.0),
        gaps=Gaps(inner=0.0, outer=8.0),
    )
)
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Gaps, MasterStackLayout, MasterStackLayoutOptions, Workspace, WorkspaceOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let layout = MasterStackLayout::new_with(
        &objects,
        MasterStackLayoutOptions {
            gap: Some(12.0),
            ..MasterStackLayoutOptions::default()
        },
    )?;
    let dev = Workspace::new_with(
        &objects,
        "dev",
        WorkspaceOptions {
            name: Some("Development".to_owned()),
            layout: Some(layout.as_layout()?),
            gaps: Some(Gaps {
                inner: 0.0,
                outer: 8.0,
            }),
        },
    )?;
    model.workspaces()?.add(&dev)?;
    Ok(())
}
```

:::

## Keys and gestures

`input.bind` binds a key, a pointer button or a touchpad gesture to a function. A trigger that is already bound at the
same priority is refused, so delete the stock line first. See [shortcuts](/guides/shortcuts).

::: tabs

```typescript
const input = gnoblin.input.primary;
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
input.bind("Swipe4Up", () => {
    input.focusedClient?.close();
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
seat = gnoblin.input.primary


def terminal(event):
    gnoblin.spawn(["foot"])


def close_focused(event):
    client = seat.focused_client
    if client is not None:
        client.close()


seat.bind("Super+Return", terminal)
seat.bind("Swipe4Up", close_focused)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let context = model.input()?.primary()?;
    let launcher = Rc::clone(&model);
    context.bind(
        "Super+Return",
        Some(move |_| {
            launcher.spawn(&["foot".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    let closer = Rc::clone(&model);
    context.bind(
        "Swipe4Up",
        Some(move |_| {
            if let Some(client) = closer.input()?.focused_client()? {
                client.close()?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

## Dialogs

`gnoblin.dialogs` shows you each consent or passphrase request before its dialog opens, and a listener may refuse it.
See [dialogs](/config/configure/dialogs).

::: tabs

```typescript
import { DialogListener, DialogRequest, gnoblin } from "gnoblin-core/generated/objects";

gnoblin.dialogs.addListener(
    new (class extends DialogListener {
        onConsentRequest(request: DialogRequest) {
            if (request.subject === "Screenshot") {
                request.decline("screenshots are off");
            }
        }
    })(),
);
```

```python
from gnoblin.objects.model import DialogListener, DialogRequest, Gnoblin

gnoblin = Gnoblin.get()


class NoScreenshots(DialogListener):
    def on_consent_request(self, request: DialogRequest) -> None:
        if request.subject == "Screenshot":
            request.decline("screenshots are off")


gnoblin.dialogs.add_listener(NoScreenshots())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{DialogListenerHandlers, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let dialogs = model.dialogs()?;
    let listener = DialogListenerHandlers {
        on_consent_request: Some(Box::new(|request| {
            if request.subject()? == "Screenshot" {
                request.decline(Some("screenshots are off"))?;
            }
            Ok(())
        })),
        on_secret_request: None,
    }
    .implement(&objects)?;
    dialogs.add_listener(&listener)?;
    Ok(())
}
```

:::

## Compositor settings

`gnoblin.compositor` holds the settings of the compositor itself that a shell may write. The compositor reads them when it
starts, so a change is accepted at once and applies from the next login. `gnoblin.compositor.reset("user_shaders")` takes it back
to the default.

| Property      | Accepted values   | Default | Effect                                                                                   |
| ------------- | ----------------- | ------- | ---------------------------------------------------------------------------------------- |
| `userShaders` | `true` or `false` | `true`  | Whether a shell may give a window its own shader. See [custom shaders](/custom-shaders). |

::: tabs

```typescript
gnoblin.compositor.userShaders = false;
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()

gnoblin.compositor.user_shaders = False
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    model.compositor()?.set_user_shaders(false)?;
    Ok(())
}
```

:::

## Settings with no object yet {#dropped-settings}

No object of the shell carries these settings, so a shell cannot set them in any language. The shell keeps its default for
each. The table says what stays in each case.

| Setting                                                                                                                                                                          | What stays                                       |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| Window rules (`rules`)                                                                                                                                                           | Listeners and `where(...).style`: see above      |
| Focus mode, new-window focus, raise, auto raise, `edgeTiling`, `dragConstraint`                                                                                                  | Click focus, raise on click, edge snapping on    |
| `autostart`                                                                                                                                                                      | A `spawn` call at the top of the shell file      |
| `protocols`, `keyring`, `compositor` (all but `userShaders`), `appearance`, `accessibility`, `layerShell`, `auth`, `location`, `xwayland`, `cursor`, `monitors`, `dialogs.style` | The default of each                              |
| `touchpadGestures` (drawn paths), `workspaceSets`, `launcher`, `inputSources`, tablets, styluses, trackballs, pointing sticks, `orientationLock`, keyboard options               | A swipe or pinch is `input.bind`; the rest: none |

Window rules, titlebar clicks and the other seat policy have no replacement yet.

The click focus, the focus of a new window and the raise on click stay with no line in the shell file: the library starts its focus
policy when a file makes a workspace, a display or a window frame. A file with a focus policy of its own calls
`FocusPolicy.get().stop()` first (`FocusPolicy::get(&objects)?.stop()?` in Rust); see
[Focus and activation](/shell-api/focus-activation).
