# Shell runtime API

The runtime API is what a shell sees. Use it to read compositor state, place and style windows, handle input and events,
and ask the session for things. Each language imports it its own way:

- TypeScript: the `gnoblin` global and classes from `"gnoblin-core"` and `"gnoblin-std"`.
- Python: `gnoblin.objects.model` and `gnoblin.objects.std`.
- Rust: the `gnoblin_core` and `gnoblin_std` crates.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

for (const client of gnoblin.clients.toList()) {
    gnoblin.spawn(["notify-send", `${client.appId} is open`]);
}
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()

for client in gnoblin.clients.to_list():
    gnoblin.spawn(["notify-send", f"{client.app_id} is open"])
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let clients = model.clients()?;
    for client in clients.to_list()? {
        model.spawn(&["notify-send".to_owned(), format!("{} is open", client.app_id()?)], None)?;
    }
    Ok(())
}
```

:::

Where each example goes:

- TypeScript: `~/.config/gnoblin/shell.ts`. The `gnoblin` object is a global, so a listener or a key handler may use it.
- Python: `~/.config/gnoblin/shell.py`.
- Rust: a shell library, which [Write a shell in Rust](/rust-shell) builds and runs.

A shell file runs top to bottom when the shell loads it. To write your first shell, start with the
[TypeScript](/typescript-shell), [Python](/python-shell) or [Rust](/rust-shell) guide. Settings the API has no object
for are listed in [Settings with no object](/js-settings).

## The `gnoblin` object

| Member                            | What it is                                                                  | Page                                     |
| --------------------------------- | --------------------------------------------------------------------------- | ---------------------------------------- |
| `clients`                         | The live collection of windows.                                             | [Windows](windows)                       |
| `workspaces`                      | The workspaces.                                                             | [Workspaces](workspaces)                 |
| `outputs`                         | The connected displays.                                                     | [Outputs](outputs)                       |
| `find(kind, filter?)`             | Selects windows, workspaces or outputs.                                     | [Find objects](find)                     |
| `focus`                           | The windows in the order they were focused.                                 | [Focus and activation](focus-activation) |
| `input`                           | The seats and their devices, and bindings.                                  | [Input and seats](input)                 |
| `idle`                            | Watches for the session going idle, and holds that keep it awake.           | [Idle watches](idle)                     |
| `launches`                        | Launch feedback.                                                            | [Launch feedback](launch-feedback)       |
| `process`, `spawn`                | Programs the shell started.                                                 | [Processes](processes)                   |
| `session`, `runtime`, `version()` | The session, reloads and the build.                                         | [Session and runtime](session-runtime)   |
| `dialogs`                         | The compositor's consent and passphrase dialogs.                            | [Requests and dialogs](requests)         |
| `authentication`                  | Polkit authentication requests.                                             | [Authentication prompts](authentication) |
| `timers`                          | Calls that run later, and the monotonic clock.                              | [Operations, errors and timers](errors)  |
| `motion(duration)`, `gestures`    | Animation durations that obey reduced motion, and touchpad gesture helpers. | [Tweens and easing](tweens)              |
| `on(event, handler)`              | Compositor signals.                                                         | [Events](/events/)                       |
| `requestTransition(work, ...)`    | Draws a change of state as one animation.                                   | [Tweens and easing](tweens)              |
| `scene`                           | The scene: the nodes and views a script makes, and property bindings.       | [Scene and views](scene)                 |

## More pages

- [Canvas painting](canvas) and the [UI toolkit](/js-ui): draw the shell's own interface.
- [Keyboard shortcuts](keyboard-shortcuts) and [Input handlers](input-handlers): triggers, drags and gestures.
- [Window snapping](window-snapping): tile a window to an edge.
- [Shared types](types): records and small types that several pages use.
- [Window style](/js-style), [Window policy](/js-window-policy) and the [console](/js-console).

## Conventions

- Names are camelCase in TypeScript: `appId`, `exitCode`, `reloadConfig`. Python and Rust spell the same members in `snake_case`:
  `app_id`, `exit_code`, `reload_config`. The tables on these pages use the TypeScript spellings.
- A read returns a live object or a plain value. A write to a live object asks the compositor, and the next read shows the
  result.
- A call that finishes later returns an `Operation`. A call with a bad argument throws at once. See
  [Operations, errors and timers](errors).
- Every time is in milliseconds: durations, delays, timeouts and the clocks. The one exception is a drag event's
  `deltaTime`, which is in seconds. Easings are the CSS names.
- Your code runs on the shell's single thread, between frames, under a time budget per call. Keep a handler short.
- Listeners and handlers that a configuration attached end with it, so a reload starts from your file again.

`gnoblin.version().api` is the API version of the build. The socket protocol for programs that are not scripts is in the
[compositor bridge](/bridge/). For command-line use, see [gnoblinctl](/cli/).

## Not in the TypeScript shell

These areas have no TypeScript API, because the compositor does not offer what they would need:

| Area                  | Why                                                                                                        |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| Privacy state         | The compositor publishes none, and a shell does not reach the session bus that serves it.                  |
| Keyboard grab         | The tree carries no key events. A binding takes a chord, never the keyboard, so there is nothing to lease. |
| Text insertion        | There is no text-input target or grant to insert into.                                                     |
| Tablet-pad help       | The compositor sends no help request. A device lists a pad's name and capabilities and nothing more.       |
| Changing an output    | The compositor has no operation to apply a mode, scale or position.                                        |
| Node effects          | A window's blur and shadow are its [style](/js-style); there is no `effects` list on a node.               |
| Overrides of settings | A script lays no value over a committed setting. Change the file and reload.                               |
