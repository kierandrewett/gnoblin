# Window menu requests

Looking for the menu the stock shell draws? See [the window menu and alerts](/guides/stock-window-menu).

The compositor reports a window's menu request to the shell that holds the tree. The shell draws the menu as its own
interface and chooses which actions to show.

## Hear a request in TypeScript

A menu request is one kind of request a client makes of its window. A `ClientListener` hears it in `onRequest`, with the
anchor the menu opens at in `request.x` and `request.y`, in global logical coordinates:

```ts
import { ClientListener, gnoblin } from "gnoblin";
import type { Client, ClientRequest } from "gnoblin";

class OwnMenu extends ClientListener {
    onRequest(client: Client, request: ClientRequest<Client>) {
        if (request.kind === "menu") {
            request.handle();
            gnoblin.spawn(["notify-send", `Menu for ${client.title} at ${request.x}, ${request.y}`]);
        }
    }
}

gnoblin.clients.addListener(new OwnMenu());
```

Put the code in `~/.config/gnoblin/init.ts`. Right-click a titlebar: the stock menu does not open, and a notification says where
the menu would have been. `request.handle()` takes the request, so the stock menu does not run. To change what the stock
menu offers or draw another one, assign `windowMenu.items` or `windowMenu.show` instead.

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
