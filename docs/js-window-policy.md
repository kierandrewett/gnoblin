# Window policy

The window policy of the shell: moving and resizing with the keyboard, touchpad gesture actions, reduced motion,
binding-only devices, focus hand-off, workspace removal and gesture bindings.

It is the same in a TypeScript, a Python and a Rust shell: the policy is in the std library, and each language reaches it
through the same classes. Each section names the test that proves it (`crates/gnoblin-js/tests/{reposition,window_policy,input}.rs`).

Members are spelled the way the language spells them. TypeScript writes `moveWithKeys()`, Python `move_with_keys()` and
Rust `move_with_keys()?`.

## Move and resize with the keyboard

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import "gnoblin-std";

const client = gnoblin.clients.at(0);
client?.moveWithKeys();
client?.resizeWithKeys();
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import ClientAugment

gnoblin = Gnoblin.get()

client = gnoblin.clients.at(0)
if client is not None:
    ClientAugment.get().move_with_keys(client)
    ClientAugment.get().resize_with_keys(client)
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let Some(client) = model.clients()?.at(0)? else {
        return Ok(());
    };
    client.move_with_keys()?;
    client.resize_with_keys()?;
    Ok(())
}
```

:::

The arrow keys move the window by 16 pixels inside the work area. For a resize, Right and Down grow it and Left and Up
shrink it, never below 120 by 80.

- Return, Enter and Space keep the result; Escape puts the window back.
- The mode holds the keyboard while it runs and gives it back to the window.
- It ends when the window closes and when the session locks.
- Both methods return `false` for a window that is fullscreen, maximised, tiled or not showing.
- A snapped window is unsnapped first, and Escape snaps it again.

The window menu's Move and Resize start these.

## Gesture actions

::: tabs

```typescript
import { PinchActionBinding, SwipeActionBinding } from "gnoblin-std";

const swipe = new SwipeActionBinding({ trigger: "Swipe4" });
swipe.setAction("up", "window.toggle-maximise");
swipe.setAction("left", "workspace.next");
swipe.setAction("right", "workspace.previous");
swipe.install();

const pinch = new PinchActionBinding({ trigger: "Pinch3" });
pinch.setAction("in", "window.minimise");
pinch.setFunction("out", (event) => console.log(event.scale));
pinch.install();
```

```python
from gnoblin.objects.std import GestureEvent, PinchActionBinding, PinchWay, SwipeActionBinding, SwipeWay

swipe = SwipeActionBinding(trigger="Swipe4")
swipe.set_action(SwipeWay.UP, "window.toggle-maximise")
swipe.set_action(SwipeWay.LEFT, "workspace.next")
swipe.set_action(SwipeWay.RIGHT, "workspace.previous")
swipe.install()


def zoomed(event: GestureEvent) -> None:
    print(event.scale)


pinch = PinchActionBinding(trigger="Pinch3")
pinch.set_action(PinchWay.IN, "window.minimise")
pinch.set_function(PinchWay.OUT, zoomed)
pinch.install()
```

```rust
use gnoblin_std::{
    PinchActionBinding, PinchActionBindingOptions, PinchWay, SwipeActionBinding, SwipeActionBindingOptions, SwipeWay,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    let swipe = SwipeActionBinding::new_with(
        &objects,
        SwipeActionBindingOptions {
            trigger: Some("Swipe4".to_owned()),
            ..SwipeActionBindingOptions::default()
        },
    )?;
    swipe.set_action(SwipeWay::Up, "window.toggle-maximise")?;
    swipe.set_action(SwipeWay::Left, "workspace.next")?;
    swipe.set_action(SwipeWay::Right, "workspace.previous")?;
    swipe.install(None)?;

    let pinch = PinchActionBinding::new_with(
        &objects,
        PinchActionBindingOptions {
            trigger: Some("Pinch3".to_owned()),
            ..PinchActionBindingOptions::default()
        },
    )?;
    pinch.set_action(PinchWay::In, "window.minimise")?;
    pinch.set_function(PinchWay::Out, |event| {
        println!("{}", event.scale);
        Ok(())
    })?;
    pinch.install(None)?;
    Ok(())
}
```

:::

A gesture is `Swipe` or `Pinch` and a finger count. An action runs once, when the fingers lift, for the direction the
gesture went; a cancelled gesture runs nothing.

- The named actions are `workspace.next`, `workspace.previous`, `window.close`, `window.minimise` and
  `window.toggle-maximise`. `Gestures.get().actionNames` lists them (`action_names` in Python and Rust), and
  `Gestures.get().run(name)` runs one now.
- `setAction(way, name)` gives a way a named action and `setFunction(way, fn)` gives it a function. `clear(way)` takes
  it away. A swipe has the ways `up`, `down`, `left` and `right`, and a pinch has `in` and `out`. Python and Rust take the
  enums `SwipeWay` and `PinchWay`, and Python also takes the name as text.
- `threshold` is how far the gesture must go before it counts. `Gestures.direction(dx, dy, threshold?)` gives the way a
  gesture has gone.
- A gesture bound on `Swipe3` is refused while the stock three-finger slide holds that trigger, so delete the starter's
  `Swipe3` line first.
- A gesture event carries the raw `dx`, `dy`, `scale` and `rotation` beside `progress`.

## Reduced motion

While the seat's `reduced_motion` is true, `gnoblin.motion(duration)` is 0. Pass every duration you animate over through it.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import "gnoblin-std";

const duration = gnoblin.motion(250); // 0 while the seat asks for reduced motion
gnoblin.requestTransition(() => {}, { duration });
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import GnoblinAugment, TransitionOptions

gnoblin = Gnoblin.get()

duration = gnoblin.motion(250.0)  # 0.0 while the seat asks for reduced motion
GnoblinAugment.get().request_transition(lambda: None, TransitionOptions(duration=duration))
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::TransitionOptions;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let duration = model.motion(250.0)?; // 0 while the seat asks for reduced motion
    model.request_transition(
        || Ok(()),
        Some(&TransitionOptions {
            duration,
            ..TransitionOptions::default()
        }),
        Some(|| Ok(())),
    )?;
    Ok(())
}
```

:::

- Workspace slides and a `requestTransition` with a duration passed through `gnoblin.motion` take no time.
- The titlebar's close fade is skipped.
- The slide back of a short swipe is instant.

## Binding-only devices and the `false` action

`device.bind(trigger, action, { only: true })` makes the device binding-only: its keys reach no client and fire only the
bindings made for it or for every device. `device.bindingOnly = false` gives the claim up. An action of `false` claims
the trigger and does nothing with it; a function that returns `false` declines and lets the next binding have it.

::: tabs

```typescript
import { Keyboard } from "gnoblin-core";

for (const device of gnoblin.input.devices) {
    if (device instanceof Keyboard && device.name.includes("Macro")) {
        device.bind(
            "F13",
            () => {
                gnoblin.spawn(["foot"]);
            },
            { only: true },
        );
    }
}
```

```python
from gnoblin.objects.model import BindingEvent, BindingOptions, Gnoblin, Keyboard

gnoblin = Gnoblin.get()


def terminal(event: BindingEvent) -> bool:
    gnoblin.spawn(["foot"])
    return True


for device in gnoblin.input.devices:
    if isinstance(device, Keyboard) and "Macro" in device.name:
        device.bind("F13", terminal, BindingOptions(only=True))
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::{BindingOptions, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let input = model.input()?;
    for device in input.devices()? {
        if !device.name()?.contains("Macro") {
            continue;
        }
        let shell = model.clone();
        device.bind(
            "F13",
            Some(move |_| {
                shell.spawn(&["foot".to_owned()], None)?;
                Ok(true)
            }),
            Some(&BindingOptions {
                only: Some(true),
                ..BindingOptions::default()
            }),
        )?;
    }
    Ok(())
}
```

:::

## Focus and workspaces

- A focused window that closes hands the keyboard to its neighbour in the workspace (the next, else the previous; a
  dialog hands it to its parent) in every layout that has no `onClientRemoved` of its own.
- `Workspace.walk(step)` moves the focus along the workspace's windows.
- `gnoblin.workspaces.remove(ws)` moves the windows to the neighbour before the workspace (the one after, for the
  first), shows it where the removed one was shown, and refuses the last workspace (returns `false`).
- A left press on a titlebar focuses and raises its window as a press on the window does.
- Maximised, snapped and floating windows are refitted when a bar changes the work area.
- A reload keeps the rectangles a maximised, fullscreen or snapped window returns to.

::: tabs

```typescript
import { Workspaces } from "gnoblin-std";

const workspaces = Workspaces.get();
const dev = workspaces.byId("dev");
if (dev && !workspaces.remove(dev)) {
    console.log("the last workspace stays");
}
```

```python
from gnoblin.objects.std import Workspaces

workspaces = Workspaces.get()
dev = workspaces.by_id("dev")
if dev is not None and not workspaces.remove(dev):
    print("the last workspace stays")
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let workspaces = model.workspaces()?;
    if let Some(dev) = workspaces.by_id("dev")? {
        if !workspaces.remove(&dev)? {
            println!("the last workspace stays");
        }
    }
    Ok(())
}
```

:::

## Workspace sets

Workspace sets are not part of the shell's API: there is no way to declare a named group of workspaces or to activate
one. Each display chooses its own workspace: set `display.viewport.content = workspace` for each display you want to
place. See [Settings with no object](js-settings.md).
