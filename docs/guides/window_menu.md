# Window menu requests

Looking for the menu the stock shell draws? See [the window menu and alerts](/guides/stock-window-menu).

The compositor reports a window's menu request to the shell that holds the tree. The shell draws the menu as its own
interface and chooses which actions to show.

## Answer a menu request

A menu request is one kind of request a client makes of its window. A `RequestListener` added to `WindowState` hears it.

- The handler is `on_request` in Python and Rust, and `onRequest` in TypeScript.
- The anchor the menu opens at is `x` and `y` on the request, in global logical coordinates.

::: tabs

```typescript
import { RequestListener, WindowState } from "gnoblin-std";
import type { ClientRequest } from "gnoblin-std";
import type {} from "gnoblin-std";

class OwnMenu extends RequestListener {
    onRequest(request: ClientRequest) {
        if (request.kind === "menu") {
            request.handle();
            gnoblin.spawn(["notify-send", `Menu at ${request.x}, ${request.y}`]);
        }
    }
}

const state = WindowState.get();
state.addListener(new OwnMenu());
state.start();
```

```python
from gnoblin import gnoblin as session
from gnoblin.objects.std import ClientRequestKind, RequestListener, WindowState


class OwnMenu(RequestListener):
    def on_request(self, request):
        if request.kind == ClientRequestKind.MENU:
            request.handle()
            session.spawn(["notify-send", f"Menu at {request.x}, {request.y}"])


state = WindowState.get()
state.add_listener(OwnMenu())
state.start()
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{ClientRequestKind, RequestListenerHandlers, WindowState};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let model = Gnoblin::get(&g.objects()?)?;
    let listener = RequestListenerHandlers {
        on_request: Box::new(move |request| {
            if request.kind()? == ClientRequestKind::Menu {
                request.take()?;
                let text = format!("Menu at {}, {}", request.x()?, request.y()?);
                model.spawn(&["notify-send".to_string(), text], None)?;
            }
            Ok(())
        }),
    }
    .implement(&objects)?;
    let state = WindowState::get(&objects)?;
    state.add_listener(&listener, None)?;
    // The service holds the listener from here on.
    std::mem::forget(listener);
    state.start()?;
    Ok(())
}
```

:::

Put the code in the shell's entry file: `~/.config/gnoblin/shell.ts`, `shell.py`, or the `main` of a Rust shell.
Right-click a titlebar: the stock menu does not open, and a notification says where the menu would have been.

Answering the request stops the stock menu from running. Call `handle()` in Python and TypeScript, or `take()` in Rust. To
change what the stock menu offers, see the [stock window menu](/guides/stock-window-menu).

A request comes from one of these:

- A client calls `xdg_toplevel.show_window_menu` with a serial the compositor gave it on an input event.
- The user presses the window-menu key.
- A program calls `window.request_menu`.
- The shell's own titlebar asks, for a right click.

## Hear a request on the signal

A program that does not run in the shell hears the request as the `gnoblin.window.menu_requested` signal. It carries:

- `x`, `y`, `width` and `height`: the anchor rectangle.
- `source`: what asked, `client`, `keyboard` or `extension`.
- `seat`: the seat that asked.

The compositor does not send the signal while a shell holds the tree, so two programs do not open two menus. See the
[compositor bridge](/bridge/) for the socket.

Use the window's own identity for other window operations; do not target the focused window after the menu opens, because
focus may have moved. Read the live window to check which actions are available. The compositor rechecks the window and the
session state when it applies an operation.
