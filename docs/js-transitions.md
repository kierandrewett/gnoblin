# Animation

`gnoblin.requestTransition(callback, options)` draws a change of state as one animation. The callback runs once, now, and
stages the changes; the compositor's tween player moves what is drawn from where it is to where the callback put it.

The call is the same in the three languages a shell can be written in:

- TypeScript has `gnoblin.requestTransition`.
- Python has `gnoblin.request_transition`.
- Rust has `model.request_transition(..)` on the `ModelRoot`.

Each is also the static `Transition.request` of the std library.

::: tabs

```typescript
import { MasterStackLayout } from "gnoblin-std";

const workspace = gnoblin.workspaces.byId("dev");
const client = gnoblin.input.primary.focusedClient;

if (workspace && client) {
    gnoblin.requestTransition(
        () => {
            client.opacity = 0.85;
            client.x = 120;
            workspace.layout = new MasterStackLayout({ ratio: 0.7 });
        },
        { duration: 250, easing: "ease-out" },
    );
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Easing, MasterStackLayout, TransitionOptions

gnoblin = Gnoblin.get()

workspace = gnoblin.workspaces.by_id("dev")
client = gnoblin.input.primary.focused_client

if workspace is not None and client is not None:

    def stage() -> None:
        client.opacity = 0.85
        client.x = 120.0
        workspace.layout = MasterStackLayout(ratio=0.7)

    gnoblin.request_transition(stage, TransitionOptions(duration=250.0, easing=Easing.EASE_OUT))
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Easing, MasterStackLayout, MasterStackLayoutOptions, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let workspace = model.workspaces()?.by_id("dev")?;
    let client = model
        .input()?
        .and_then(|input| input.primary().ok().flatten())
        .and_then(|context| context.focused_client().ok().flatten());

    if let (Some(workspace), Some(client)) = (workspace, client) {
        let layout = MasterStackLayout::new_with(
            &objects,
            MasterStackLayoutOptions {
                ratio: Some(0.7),
                ..MasterStackLayoutOptions::default()
            },
        )?
        .as_layout()?;
        model.request_transition(
            move || {
                client.set_opacity(0.85)?;
                client.set_x(120.0)?;
                workspace.set_layout(&layout)?;
                Ok(())
            },
            Some(&TransitionOptions {
                duration: 250.0,
                easing: Easing::EaseOut,
                ..TransitionOptions::default()
            }),
            Some(|| Ok(())),
        )?;
    }
    Ok(())
}
```

:::

An ordinary assignment, `client.opacity = 0.85`, stays immediate. A transition is for changes that belong together.

## What the callback does

- It runs once and synchronously, in the call. It is not called again on a frame.
- Every write to a window's properties is staged. Reading a property inside the callback gives the staged value.
- A layout change inside it (`workspace.layout = other`, `layout.ratio = 0.7`, `workspace.gaps = { inner: 8 }`) is
  computed once, against the staged state, and each window that moves tweens from where it is drawn to its new place. A
  window is paired by its id. A window with no place to start from (no size yet, or hidden) is placed at once.
- A style change (`gnoblin.clients.style.opacity = 0.6`) becomes a write of the window's `opacity`, so it tweens too.
- All of it travels in the shell's one transaction of the frame, as timed writes: a duration and an easing curve of 33
  samples for each property.
- The state is committed when the transition begins. Reading `client.x` after the call gives the end value; the
  compositor draws the way there.

The callback has three limits, and each is an error that changes nothing:

- It may not be asynchronous (an `async` function in TypeScript or Python, a future in Rust) and may not return a value
  to wait for.
- It may not call `requestTransition` again.
- It may not call anything with a side effect. `client.raise()`, `lower()`, `focus()` and `close()`, `gnoblin.spawn()`,
  `Process.kill()` and `gnoblin.session.*()` fail inside a callback, because a failed transition is rolled back and a
  spawned process cannot be.

## Options

| Option     | Meaning                                                                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `duration` | Milliseconds, 0 or more. Default 250. 0 commits the change at once, still as one transaction.                                              |
| `easing`   | `linear`, `ease`, `ease-in`, `ease-out` and `ease-in-out`: the CSS names, each the CSS timing function of that name. Default `ease-out`.   |
| `delay`    | Milliseconds, 0 or more. The compositor holds the start state this long before the animation begins. Default 0; none under reduced motion. |
| `autoplay` | Default true. False holds the tracks at the start until `play()` or `reverse()`; `progress` scrubs them.                                   |

TypeScript takes the easing as its name (`"ease-out"`), Python as `Easing.EASE_OUT` or its name, and Rust as
`Easing::EaseOut`. Any other easing name or option is refused with an error that names the known ones.

## The `Transition`

`requestTransition` returns a `Transition`.

| Member               | Meaning                                                                                                                                                          |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `running`            | True until it ends or is cancelled.                                                                                                                              |
| `state`              | `running`, `finished` or `cancelled`. `cancelled` after `cancel()`, after a newer transition took every property it animated, or after it ran back to its start. |
| `duration`           | The milliseconds it takes: the duration asked for, or 0 under reduced motion.                                                                                    |
| `progress`           | 0 to 1, settable (a value outside it is refused). Moves every track to that point of its duration. Reads the last value written.                                 |
| `paused`             | Whether the tracks are held: `autoplay: false` until `play()` or `reverse()`, and after `pause()`.                                                               |
| `reversed`           | Whether the tracks run back to their start.                                                                                                                      |
| `play()`             | Lets the tracks run on towards the target from where they are.                                                                                                   |
| `pause()`            | Holds the tracks where they are.                                                                                                                                 |
| `reverse()`          | Runs the tracks back towards their start from where they are.                                                                                                    |
| `on("finished", fn)` | Calls `fn(cancelled)` once as a queued call, like any listener. Returns a `Listener` to stop it. Rust has `on_finished(fn)`.                                     |
| `cancel()`           | Writes the end values of what it still animates at once, which ends the tracks. `finished` runs with `cancelled` true.                                           |

::: tabs

```typescript
import "gnoblin-std";

const client = gnoblin.input.primary.focusedClient;

if (client) {
    const transition = gnoblin.requestTransition(
        () => {
            client.opacity = 0.5;
        },
        { duration: 400, autoplay: false },
    );
    transition.on("finished", (cancelled) => console.log(`finished, cancelled: ${cancelled}`));
    transition.progress = 0.25; // scrub to a quarter of the way
    transition.play(); // then let it run on
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import TransitionOptions

gnoblin = Gnoblin.get()

client = gnoblin.input.primary.focused_client

if client is not None:

    def stage() -> None:
        client.opacity = 0.5

    transition = gnoblin.request_transition(stage, TransitionOptions(duration=400.0, autoplay=False))
    transition.on("finished", lambda cancelled: print(f"finished, cancelled: {cancelled}"))
    transition.progress = 0.25  # scrub to a quarter of the way
    transition.play()  # then let it run on
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::TransitionOptions;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let client = model
        .input()?
        .and_then(|input| input.primary().ok().flatten())
        .and_then(|context| context.focused_client().ok().flatten());

    if let Some(client) = client {
        let transition = model.request_transition(
            move || client.set_opacity(0.5),
            Some(&TransitionOptions {
                duration: 400.0,
                autoplay: false,
                ..TransitionOptions::default()
            }),
            Some(|| Ok(())),
        )?;
        let _finished = transition.on_finished(|cancelled| println!("finished, cancelled: {cancelled}"))?;
        transition.set_progress(0.25)?; // scrub to a quarter of the way
        transition.play()?; // then let it run on
    }
    Ok(())
}
```

:::

Cancelling puts what is drawn on the end state, which the transition committed when it began. It does not go back. The end
is the compositor's: each track ends in a `track_ended` signal, so a transition settles when its tracks do, by the
compositor's clock and not the shell's.

With `autoplay: false` the tracks start held at the start value.

- The target state is committed when the transition is requested: the layout, the styles and the tree hold it. What is
  drawn is the compositor's interpolation.
- Writing `progress` moves every track to that point of its own duration at once. Hit testing follows the pose that is
  drawn.
- A transition that runs back to its start puts the start values back and ends `cancelled`.
- The tracks join a `tween` node of the compositor. A transition makes it when it has tracks and destroys it when it
  ends.

## One object

To animate one window or one workspace, write only its properties in the callback:

::: tabs

```typescript
import { MasterStackLayout } from "gnoblin-std";

const client = gnoblin.input.primary.focusedClient;
const workspace = gnoblin.workspaces.byId("dev");

gnoblin.requestTransition(
    () => {
        if (client) {
            client.opacity = 0.92;
            client.rotation = [0, 0.12, 0];
        }
        if (workspace) {
            workspace.layout = new MasterStackLayout({ ratio: 0.7 });
        }
    },
    { duration: 250, easing: "ease-out" },
);
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Easing, MasterStackLayout, TransitionOptions

gnoblin = Gnoblin.get()

client = gnoblin.input.primary.focused_client
workspace = gnoblin.workspaces.by_id("dev")


def stage() -> None:
    if client is not None:
        client.opacity = 0.92
        client.rotation = [0.0, 0.12, 0.0]
    if workspace is not None:
        workspace.layout = MasterStackLayout(ratio=0.7)


gnoblin.request_transition(stage, TransitionOptions(duration=250.0, easing=Easing.EASE_OUT))
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Easing, MasterStackLayout, MasterStackLayoutOptions, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let client = model
        .input()?
        .and_then(|input| input.primary().ok().flatten())
        .and_then(|context| context.focused_client().ok().flatten());
    let workspace = model.workspaces()?.by_id("dev")?;
    let layout = MasterStackLayout::new_with(
        &objects,
        MasterStackLayoutOptions {
            ratio: Some(0.7),
            ..MasterStackLayoutOptions::default()
        },
    )?
    .as_layout()?;

    model.request_transition(
        move || {
            if let Some(client) = &client {
                client.set_opacity(0.92)?;
                client.set_rotation(&[0.0, 0.12, 0.0])?;
            }
            if let Some(workspace) = &workspace {
                workspace.set_layout(&layout)?;
            }
            Ok(())
        },
        Some(&TransitionOptions {
            duration: 250.0,
            easing: Easing::EaseOut,
            ..TransitionOptions::default()
        }),
        Some(|| Ok(())),
    )?;
    Ok(())
}
```

:::

A write the tree would refuse fails before anything is written. Without `requestTransition` a write is atomic and
immediate.

## When transitions overlap

Two transitions can write one property of one node.

- The one that began second owns the property, and the compositor ends the other track on it.
- The earlier transition goes on with the properties it still owns, and ends at its own time.
- One that has lost every property ends at once, `cancelled`.
- A plain assignment to a property takes it from a transition the same way.

## When it fails

A write the tree would refuse, a destroyed client and a layout that throws are errors in the call. The error names the
property, as in `Client#3.opacity: must be a finite number` and `Client#4.opacity: the client is gone`. Nothing is sent.
The call throws in TypeScript, raises `GnoblinError` in Python and returns `Err` in Rust.

What is put back:

- each workspace's layout, gaps and name
- the own fields of the layout the workspace had when the transition began
- every style

What is not: a layout object the callback assigned and changed, because it was not the workspace's layout when the
transition began, and a list edited in place.

## Reduced motion

While the seat asks for reduced motion (`gnoblin.motion(1) === 0`, or `gnoblin.motion(1) == 0` in Python and
`model.motion(1.0)? == 0.0` in Rust) no write is timed: the change lands as one transaction of plain writes and
`finished` still runs, with `cancelled` false.

## What tweens

The tween player animates the properties its scene can interpolate. The shell sends a timed write for each property the
tree takes a curve for; what the compositor can draw is smaller.

| Property                                             | On the compositor                 |
| ---------------------------------------------------- | --------------------------------- |
| `x`, `y`, `z`, `opacity`                             | Tweens from the drawn value.      |
| `width`, `height`, `rotation`, `scale`               | Lands at once (no track to play). |
| `visible`, `tiled`, effects (shadows, backdrop blur) | Written at once, as any write is. |

A style `cornerRadius`, `borders` and `background` are not written to the tree for a window at all
(`client.styleWarnings`, `client.style_warnings` in Python and Rust).

## Not built

These are not built yet, in the compositor or in the shell. The shell does not fake them.

- **Effects and corner radius.** A shadow, a blur and a corner radius are not tween tracks. A style change of them lands at
  once.
- **A camera.** A `Camera2D` or `Camera3D` on a viewport is kept and not applied, so a projection blend has nothing to interpolate.
- **The place of a running transition.** The compositor reports the end of a track and nothing before it, so `progress`
  reads what the script last wrote and not where an autoplaying track is. `finish()` (settle forward or back at once) is
  the player's (`TweenPlayer::finish`) and has no tree surface yet: a script cancels, or runs the tracks with `play()` and
  `reverse()`.
- **Rotation as one orientation.** A rotation tweens per axis, as the node stores it (three angles), so a path through
  several axes is not the shortest spherical path that a quaternion would take.

## Proof

`crates/gnoblin-js/tests/transition.rs` runs the staging, the transaction and the failures over a fake compositor, and
`crates/gnoblin/tests/js_shell.rs` runs a layout change inside a transition against the real compositor with two mapped
windows and an injected clock. The compositor's tracks are tested in `crates/gnoblin-wayland/tests/tree_live.rs`.
