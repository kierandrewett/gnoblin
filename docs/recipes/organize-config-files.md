# Organize a growing config

Keep the entry point short and group statements by the kind of change you make. For example, let `init.ts` import a module
that styles windows and one that binds your shortcuts, and call them in the order you want.

In `windows.ts`:

```ts
// windows.ts
import { CollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

export function placeWindows(): void {
    gnoblin.clients.addListener(
        new (class extends CollectionListener {
            onAdded(client: Client) {
                if (client.appId === "org.gnome.TextEditor") {
                    client.maximised = true;
                }
            }
        })(),
    );
}
```

In `bindings.ts`:

```ts
// bindings.ts
export function bindKeys(): void {
    gnoblin.input.primary.bind("Super+d", () => {
        gnoblin.spawn(["fuzzel"]);
    });
}
```

In `init.ts`:

```ts
import { bindKeys } from "./bindings.ts";
import { placeWindows } from "./windows.ts";

placeWindows();
bindKeys();
```

Imported files share the same `gnoblin` object and need no registration. Keep the calls in one place, `init.ts`, so you
can see which file supplies each part, and in the order they run. A trigger is bound once, so bind each key from one file
only. `gnoblin.workspaces.add` and `gnoblin.clients.addListener` accumulate across files.

Check the files with `gnoblinctl check init.ts`, and read [file loading and order](/guides/files_and_load_order) before
adding overlapping files.
