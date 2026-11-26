# Focus and activation

Focus policy decides which window gets the keyboard, and when it rises above the others. The shell keeps click to
focus, focuses a window that opens, and raises a window when you click it.

The library starts that policy itself when a file uses it to make a workspace, a display or a window frame, so a file
needs no line for it.

A file cannot change the policy's settings. Focus mode, auto raise and the rest of the older `focus` settings have no object in the
API: see [Settings with no object](/js-settings). Read the order windows were focused with `gnoblin.focus.history()` (`focus.history()` in Python and Rust), and the seat's current values with
`gnoblin.input.primary.settings` (`input.primary.settings` in Python and Rust).

A file that decides focus itself calls `FocusPolicy.get().stop()` before it does, so that the two never answer one click.

`FocusPolicy.get().start()` starts the stock policy again, and does nothing while it runs. In Rust the call is
`FocusPolicy::get(&objects)?.stop()?`, with the `objects` of `g.std()?.objects()?`. A shell that is not a desktop of the
library, one written on the core alone, has no focus policy: nothing moves its keyboard until it calls `client.focus()`.

## Give a window the focus

`client.focus()` asks for the keyboard on a window. The compositor applies its own policy: it may leave the focus where it
was. To raise the window as well, call `client.raise()`.

::: tabs

```typescript
const client = gnoblin.clients.toList().find((c) => c.appId === "foot");
client?.focus();
client?.raise();
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
client = next((c for c in gnoblin.clients.to_list() if c.app_id == "foot"), None)
if client is not None:
    client.focus()
    client.raise_()
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    for client in clients.to_list()? {
        if client.app_id()? == "foot" {
            client.focus()?;
            client.raise()?;
            break;
        }
    }
    Ok(())
}
```

:::

A window that closes while focused hands the keyboard to its neighbour on the workspace: the next window, else the
previous one. A dialog hands it to its parent.

## Focus history

`gnoblin.focus.history(filter?)` returns the live windows, last focused first. Windows never seen focused follow in list
order, and minimised windows are included.

::: tabs

```typescript
const [current, previous] = gnoblin.focus.history({ limit: 2 });
previous?.focus();
```

```python
from gnoblin.objects.model import FocusFilter, Gnoblin

history = Gnoblin.get().focus.history(FocusFilter(limit=2))
if len(history) > 1:
    history[1].focus()
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{FocusFilter, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(focus) = model.focus()? else {
        return Ok(());
    };
    let filter = FocusFilter {
        limit: Some(2),
        ..FocusFilter::default()
    };
    if let Some(previous) = focus.history(Some(&filter))?.get(1) {
        previous.focus()?;
    }
    Ok(())
}
```

:::

| Filter key    | Values                | Default | Effect                              |
| ------------- | --------------------- | ------- | ----------------------------------- |
| `workspaceId` | a workspace id        | none    | Only the windows of that workspace. |
| `monitorId`   | an output name        | none    | Only the windows of that output.    |
| `limit`       | integer from 1 to 256 | `50`    | How many windows to return.         |

An unknown filter key throws a `TypeError` that names the keys that work.

In TypeScript the keys are `workspaceId`, `monitorId` and `limit`. Python's `FocusFilter` and the Rust `FocusFilter` struct spell them
`workspace_id`, `monitor_id` and `limit`. Rust takes the filter by reference, as `Some(&filter)`, and `limit` is an `i64`.

## Launching from a layer surface

A shell that runs as a separate Wayland client can start an application from a panel or popup with the standard
[XDG activation protocol](https://wayland.app/protocols/xdg-activation-v1).

Request the token from the connection that received the input, with that surface, seat and initiating press serial. Commit
the request before you dismiss or destroy the source surface, then pass the returned token to the application.

With the compositor's default activation policy, it keeps a verified launch gesture after the source closes. Releasing its key or
mouse button does not revoke it. A later press, touch start, scroll or session lock does, and unlocking does not restore it.
The token expires 15 seconds after the initiating event and can authorize focus only once.
