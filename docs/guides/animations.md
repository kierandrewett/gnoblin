# Animations

No shell language declares named animations, keyframes or per-window animation rules. This page says what moves today and how to move a window yourself.

## What animates

The shell and compositor animate these on their own. The compositor's part (the fades below) is the desktop's look, which the std
library sets: a session that runs a shell with no std library (a core-only shell) has none of it, and the shell animates its own windows.

- **Workspace switch.** The windows of the workspace being left slide off one side and the new workspace's windows slide
  in from the same side. Each window moves by one output width over 200 ms with an ease-out cubic curve, and the side
  follows the workspace order. A second switch during a slide starts from where the windows are drawn. A three-finger
  swipe on the touchpad drags the slide with the fingers.
- **Hide and close.** Hiding a window, as minimising does, fades the window's last picture out over 200 ms. Restoring it
  during that fade eases it back to opaque. A window that closes fades out the same way, from the last picture its
  client committed.
- **Titlebar.** The close fade of the titlebar is skipped with reduced motion.

A client that reuses a GPU buffer for its next frame while a fade runs can show through the fade.

## Turn animations off

Reduced motion makes every one of these instant: a workspace switch lands at once and a window is hidden at once. A
config can read the setting, as `gnoblin.input.primary.settings.reducedMotion`, but no shell can set it. See [dropped settings](/js-settings#dropped-settings).

## Animate a window yourself

`gnoblin.requestTransition(work, options)` runs `work` once and moves what it changed to the new values over
`options.duration` milliseconds along a named curve. See [animation in TypeScript](/js-transitions) for what the callback
may do.

- The properties of a window that move over time are `x`, `y`, `z` and `opacity`. The others land at once.
- The curves are the CSS names `linear`, `ease`, `ease-in`, `ease-out` (the default) and `ease-in-out`.
- A duration of `0` writes at once.
- `motion(duration)` (`gnoblin.motion(duration)` in TypeScript, `gnoblin.motion(duration)` in Python, `motion(duration)?` in Rust) returns `0` while reduced motion is on, so a script that passes its duration through it respects the setting.

This fades each Text Editor window in when it opens:

::: tabs

```typescript
import { ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.gnome.TextEditor") {
                client.opacity = 0;
                gnoblin.requestTransition(
                    () => {
                        client.opacity = 1;
                    },
                    { duration: gnoblin.motion(300), easing: "ease-out" },
                );
            }
        }
    })(),
);
```

```python
import gnoblin.objects.std
from gnoblin.objects.model import Client, Gnoblin
from gnoblin.objects.std import TransitionOptions

gnoblin = Gnoblin.get()


@gnoblin.clients.on("added")
def fade_in(client: Client) -> None:
    if client.app_id == "org.gnome.TextEditor":
        client.opacity = 0

        def reveal() -> None:
            client.opacity = 1

        gnoblin.request_transition(
            reveal,
            TransitionOptions(duration=gnoblin.motion(300), easing="ease-out"),
        )
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{Client, ModelRoot};
use gnoblin_std::objects::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    clients.on_added(move |client: Client, _| {
        if client.app_id().unwrap_or_default() == "org.gnome.TextEditor" {
            let _ = client.set_opacity(0.0);
            let target = client.clone();
            let _ = model.request_transition(
                move || target.set_opacity(1.0),
                Some(&TransitionOptions {
                    duration: model.motion(300.0)?,
                    easing: Easing::EaseOut,
                    delay: 0.0,
                    autoplay: true,
                }),
                None::<fn() -> gnoblin::Result<()>>,
            );
        }
    })?;
    Ok(())
}
```

:::

Replace the app id with the value `gnoblinctl window list` prints.
