# Window state shortcuts

Bind Super+Down to restore a maximised window first, then minimise it on a second press.

## Configure the binding

A key is a function, and the function can read and write the focused window:

::: tabs

```typescript
gnoblin.input.primary.bind("Super+Down", () => {
    const client = gnoblin.input.primary.focusedClient;
    if (!client) {
        return;
    }
    if (client.maximised) {
        client.maximised = false;
    } else {
        client.minimised = true;
    }
});
```

```python
from gnoblin import gnoblin

seat = gnoblin.input.primary


def toggle_window(event):
    client = seat.focused_client
    if client is None:
        return
    if client.maximised:
        client.maximised = False
    else:
        client.minimised = True


seat.bind("Super+Down", toggle_window)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    g.bind("Super+Down", move || -> gnoblin::Res<()> {
        let input = model.input()?;
        let primary = input.primary()?;
        let Some(client) = primary.focused_client()? else { return Ok(()) };
        if client.maximised()? {
            client.set_maximised(false)?;
        } else {
            client.set_minimised(true)?;
        }
        Ok(())
    })?;
    Ok(())
}
```

:::

Save the file and the running shell reloads it. Press Super+Down on a maximised window to return it to its previous
size. Press it again to minimise it. On an ordinary window, the first press minimises it at once.

`client.maximised`, `client.minimised` and `client.fullscreen` are writable. Writing one asks the compositor for the
change, and the shell answers it as it does a request from the window itself. A trigger is bound once, so if the stock
file or an earlier line already binds `Super+Down`, delete that line first: see [keyboard shortcuts](/guides/shortcuts)
for conflicts.

A minimised window is hidden, and the shell hands the keyboard focus to the topmost window left. Bring it back from
the taskbar or panel you use, or with a key that sets `client.minimised = false` on a window you pick.
