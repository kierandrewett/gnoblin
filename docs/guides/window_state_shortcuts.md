# Window state shortcuts

[Configuration reference](/config/configure)

Bind Super+Down to restore a maximised window first, then minimise it on a second press.

## Configure the binding

A key can run a function, and the function can read and write the focused window:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    keys: {
        "Super+Down": ({ gnoblin }) => {
            const client = gnoblin.clients.focused;
            if (client === null) {
                return;
            }
            if (client.maximised) {
                client.maximised = false;
            } else {
                client.minimised = true;
            }
        },
    },
});
```

Save the file and the running shell reloads it. Press Super+Down on a maximised window to return it to its previous
size. Press it again to minimise it. On an ordinary window, the first press minimises it at once.

`client.maximised`, `client.minimised` and `client.fullscreen` are writable. Writing one asks the compositor for the
change, and the shell answers it as it does a request from the window itself. A key that matches a stock key replaces
it: see [keyboard shortcuts](/guides/shortcuts) for conflicts.

A minimised window is hidden, and the shell hands the keyboard focus to the topmost window left. Bring it back from
the taskbar or panel you use, or with a key that sets `client.minimised = false` on a window you pick.
