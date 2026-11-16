# Split a config into files

Import functions from `shell.ts` and call them in the order you want them applied. In `appearance.ts`:

```ts
// appearance.ts
export function styleWindows(): void {
    gnoblin.clients.style.cornerRadius = 12;
    gnoblin.clients.style.opacity = 1;
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

In `shell.ts`:

```ts
import { styleWindows } from "./appearance.ts";
import { bindKeys } from "./bindings.ts";

styleWindows();
bindKeys();
```

Modules share the same `gnoblin` object, and a module that exports nothing still runs when it is imported. A path
starts with `./`, names the file with its extension and resolves under the config directory. There is no glob.
See [files and load order](/guides/files_and_load_order) for how a reload runs several files.
