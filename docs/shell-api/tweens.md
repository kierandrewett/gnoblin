# Tweens and easing

The transition call (`requestTransition` in TypeScript, `Transition.request` in Python, `request_transition` in Rust) moves a window's properties to new values over time. The callback `work` writes
the new values, and the compositor plays the animation in its render loop, so each frame follows the display and the
script does not run per frame.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import "gnoblin-std";

const gnoblin = Gnoblin.get();

gnoblin.input?.primary?.bind("Super+o", () => {
    const client = gnoblin.input?.primary?.focusedClient;
    if (client !== undefined) {
        gnoblin.requestTransition(
            () => {
                client.opacity = client.opacity < 1 ? 1 : 0.5;
            },
            { duration: 300 },
        );
    }
});
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Transition, TransitionOptions

gnoblin = Gnoblin.get()


def toggle_opacity(event):
    client = gnoblin.input.primary.focused_client
    if client is None:
        return False

    def write():
        client.opacity = 1.0 if client.opacity < 1 else 0.5

    Transition.request(write, TransitionOptions(duration=300.0))
    return True


gnoblin.input.primary.bind("Super+o", toggle_opacity)
```

```rust
use std::rc::Rc;

use gnoblin_core::Gnoblin;
use gnoblin_std::TransitionOptions;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Rc::new(Gnoblin::get(&g.objects()?)?);
    let input = model.input()?;
    let context = input.primary()?;
    let fader = Rc::clone(&model);
    context.bind(
        "Super+o",
        Some(move |_event| {
            let Some(client) = fader.input()?.focused_client()? else {
                return Ok(false);
            };
            let options = TransitionOptions {
                duration: 300.0,
                ..TransitionOptions::default()
            };
            fader.request_transition(
                move || {
                    let opacity = if client.opacity()? < 1.0 { 1.0 } else { 0.5 };
                    client.set_opacity(opacity)?;
                    Ok(())
                },
                Some(&options),
                Some(|| Ok(())),
            )?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts` and the Python in `~/.config/gnoblin/shell.py`. The Rust is a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+o` fades the focused window to half opacity
over 300 ms, and back on the next press.

## Animate a window

The transition call runs `work` once, now. Every write it makes to a window is staged, and the options say how the window moves to the staged values. It returns a `Transition`. See [Animation](/js-transitions).

| Option     | Values                                                 | Default    | Meaning                                                   |
| ---------- | ------------------------------------------------------ | ---------- | --------------------------------------------------------- |
| `duration` | milliseconds, 0 or more                                | 250        | How long the movement takes. 0 writes at once.            |
| `easing`   | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` | `ease-out` | The curve between the start and the end, as CSS names it. |
| `delay`    | milliseconds, 0 or more                                | 0          | How long the compositor holds the start before it begins. |

Without `options` the movement takes 250 ms along `ease-out`.

The properties the compositor moves over time:

| Property                    | Value               | Meaning                                    |
| --------------------------- | ------------------- | ------------------------------------------ |
| `x`, `y`, `width`, `height` | number              | The window geometry, no shadow, in pixels. |
| `opacity`                   | number, 0 to 1      | How opaque the window is.                  |
| `z`                         | number              | The window's place in depth.               |
| `rotation`                  | `[x, y, z]` numbers | Rotation about each axis, in radians.      |
| `scale`                     | `[x, y, z]` numbers | Scale along each axis.                     |

Any other writable property, such as `visible`, is written at once with the rest. Reading a property back right after the
call gives the end value; the compositor draws the way there. A write the tree refuses or a `duration` below 0 throws, and
nothing is written.

What the callback may write:

- A property of a `Client` (`opacity`, `x`, `rotation`: `client.opacity = 0.5` in TypeScript and Python, a setter method in Rust) is staged with the other writes, and reading it inside the callback gives the staged value.
  A timed write of a window (`setOpacityTimed` in TypeScript, `set_opacity_timed` in Python and Rust) is not staged: it plays by the timing it is given.
- A property of a workspace, a layout or the styles is staged by the library and tweens through the windows it moves.
- Staging a window's property needs a core that has the write stage (`set_write_stage`, see [ABI compatibility](/abi-compatibility)).
  On a core built before it the writes of a callback to a window's properties stay plain: they are immediate, untimed and not rolled back.

To move one window or workspace, call its `set(values, options)`. It is the transition call for that one object:

- Every key of `values` must be a writable property, or the call is an error and writes nothing.
- It takes the same options and returns the same `Transition`.
- Without options it writes the values at once.

::: tabs

```typescript
import "gnoblin-std";

gnoblin.input.primary.focusedClient?.set({ x: 0, y: 0 }, { duration: 400, easing: "ease-out" });
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Easing, Transition, TransitionOptions

client = Gnoblin.get().input.primary.focused_client
if client is not None:
    window = client

    def move_to_origin():
        window.x = 0.0
        window.y = 0.0

    Transition.request(move_to_origin, TransitionOptions(duration=400.0, easing=Easing.EASE_OUT))
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let Some(client) = model.input()?.focused_client()? else {
        return Ok(());
    };
    let options = TransitionOptions {
        duration: 400.0,
        easing: Easing::EaseOut,
        ..TransitionOptions::default()
    };
    model.request_transition(
        move || {
            client.set_x(0.0)?;
            client.set_y(0.0)?;
            Ok(())
        },
        Some(&options),
        Some(|| Ok(())),
    )?;
    Ok(())
}
```

:::

The curves are the CSS timing functions of the same names:

- `linear` moves at a constant speed.
- `ease` starts quickly, then slows down. It is the CSS default.
- `ease-in` starts slowly and speeds up.
- `ease-out` starts fast and slows down towards the end. It is the default here.
- `ease-in-out` speeds up in the first half and slows down in the second.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import "gnoblin-std";

const gnoblin = Gnoblin.get();
const client = gnoblin.input?.primary?.focusedClient;
if (client !== undefined) {
    gnoblin.requestTransition(
        () => {
            client.x = 0;
            client.y = 0;
            client.rotation = [0, 0, 0.1];
        },
        { duration: 400, easing: "ease-out" },
    );
}
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Easing, Transition, TransitionOptions

client = Gnoblin.get().input.primary.focused_client
if client is not None:
    window = client

    def move_and_tilt():
        window.x = 0.0
        window.y = 0.0
        window.rotation = [0.0, 0.0, 0.1]

    Transition.request(move_and_tilt, TransitionOptions(duration=400.0, easing=Easing.EASE_OUT))
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let Some(client) = model.input()?.focused_client()? else {
        return Ok(());
    };
    let options = TransitionOptions {
        duration: 400.0,
        easing: Easing::EaseOut,
        ..TransitionOptions::default()
    };
    model.request_transition(
        move || {
            client.set_x(0.0)?;
            client.set_y(0.0)?;
            client.set_rotation(&[0.0, 0.0, 0.1])?;
            Ok(())
        },
        Some(&options),
        Some(|| Ok(())),
    )?;
    Ok(())
}
```

:::

The compositor draws a window's own open, close and minimise animations itself, and a value you set while one plays can be
overwritten when it ends.

A newer write of a property takes it from the animation that was moving it, and that animation ends `cancelled`.

To animate with a transition object that you can pause, reverse or cancel, see [Animation](/js-transitions).

## Workspace slides

Showing a workspace slides the windows of the two workspaces over `duration` milliseconds. The stock keys `Super+1` to `Super+9` slide over 250 ms. See [Workspaces](workspaces#show-a-workspace).

## Reduced motion

`motion(duration)` returns `duration` milliseconds, or 0 while the user asks for reduced motion. The stock animations call
it, so a workspace slide and the titlebar's close fade take no time for a user who asks for reduced motion. Pass your own
durations through it:

::: tabs

```typescript
const duration = gnoblin.motion(300);
```

```python
from gnoblin import Gnoblin

duration = Gnoblin.get().motion(300.0)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let duration = model.motion(300.0)?;
    Ok(())
}
```

:::

## Run something after an animation

The transition call returns a `Transition`. Its `finished` event runs when the animation ends, and tells whether it was cancelled:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import "gnoblin-std";

const gnoblin = Gnoblin.get();
const client = gnoblin.input?.primary?.focusedClient;
if (client !== undefined) {
    const fade = gnoblin.requestTransition(
        () => {
            client.opacity = 0;
        },
        { duration: 200 },
    );
    fade.on("finished", (cancelled) => {
        if (!cancelled) {
            client.close();
        }
    });
}
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Transition, TransitionOptions

client = Gnoblin.get().input.primary.focused_client
if client is not None:
    window = client

    def fade_out():
        window.opacity = 0.0

    fade = Transition.request(fade_out, TransitionOptions(duration=200.0))

    def close_unless_cancelled(cancelled):
        if not cancelled:
            window.close()

    fade.on("finished", close_unless_cancelled)
```

```rust
use std::rc::Rc;

use gnoblin_core::Gnoblin;
use gnoblin_std::TransitionOptions;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let Some(client) = model.input()?.focused_client()? else {
        return Ok(());
    };
    let client = Rc::new(client);
    let writer = Rc::clone(&client);
    let options = TransitionOptions {
        duration: 200.0,
        ..TransitionOptions::default()
    };
    let fade = model.request_transition(
        move || {
            writer.set_opacity(0.0)?;
            Ok(())
        },
        Some(&options),
        Some(|| Ok(())),
    )?;
    fade.on_finished(move |cancelled| {
        if !cancelled {
            let _ = client.close();
        }
    })?
    .keep();
    Ok(())
}
```

:::

`timers.after(ms, fn)` runs at the first frame after the time. See [Operations, errors and timers](errors#timers).

## Turn animations off

Reduced motion makes every one of these instant. A shell reads the setting through `motion`, and cannot change it. See [dropped settings](/js-settings#dropped-settings).

## Animate a window yourself

[Animate a window yourself](/guides/animations#animate-a-window-yourself) has a complete example that fades each window
in when it opens.
