# Route new app windows to named workspaces

Give workspaces stable IDs in `init.ts`, then place each matching new window on its workspace when it opens. Replace
the example app IDs with the values reported by `gnoblinctl window list --json`.

```ts
import { CollectionListener } from "gnoblin-core";
import { FloatingLayout, Workspace } from "gnoblin-std";
import type { Client } from "gnoblin-core";

for (const id of ["code", "web", "chat"]) {
    gnoblin.workspaces.add(new Workspace({ id, layout: new FloatingLayout() }));
}

const homes: Record<string, string> = {
    "org.gnome.TextEditor": "code",
    "org.mozilla.firefox": "web",
    "org.gnome.Fractal": "chat",
};

gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client: Client) {
            const id = homes[client.appId];
            const target = id === undefined ? undefined : gnoblin.workspaces.get(id);
            if (target !== undefined) {
                client.moveToWorkspace(target);
            }
        }
    })(),
);
```

Each `Workspace` has a unique non-empty `id`, and `gnoblin.workspaces.get(id)` finds it. The first one added is shown when
the shell starts.

| Part                        | Values and default                    | Effect                                                         |
| --------------------------- | ------------------------------------- | -------------------------------------------------------------- |
| `client.appId`              | The application ID of the window      | Selects windows by application ID, here by exact match.        |
| `client.moveToWorkspace(w)` | A `Workspace` in `gnoblin.workspaces` | Moves the window; false when it cannot (a dialog, fullscreen). |

An ID matches whole, so `"org.mozilla.firefox"` matches only that app. For every app of one prefix, test
`client.appId.startsWith("org.mozilla.")`, or use a `RegExp`.

Save the file and the running shell reloads it. A listener also runs for the windows that are already open, so they move
too when the file loads.

Placement happens once; it does not lock a window to its workspace. You can move it afterward, and the shell leaves it
there.

- If an app reuses an existing window, the listener does not run again.
- Dialogs stay with their parent window.
- A workspace id you did not declare makes `gnoblin.workspaces.get` return `undefined`, so the window stays where it is.

See [window rules](/guides/window_rules#workspaces) and the [workspaces](/shell-api/workspaces) reference.
