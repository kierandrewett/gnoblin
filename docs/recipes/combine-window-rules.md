# Combine window rules

Set shared defaults first, then override them for selected windows. Later statements win, so put the general one first.
Put this in `~/.config/gnoblin/shell.ts`. It imports `gnoblin-std`, which provides `gnoblin.clients.style`:

```ts
import { CollectionListener } from "gnoblin-core";
import "gnoblin-std";
import type { Client } from "gnoblin-core";

gnoblin.clients.style.opacity = 1;

gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client: Client) {
            client.maximised = false;
            if (client.appId === "org.gnome.TextEditor") {
                client.maximised = true;
            }
        }
    })(),
);
```

The first statement and the first line of the listener apply to every window. The `if` changes only the editor: the
default is set first and the exception after it. See [window rules](/guides/window_rules#order).
