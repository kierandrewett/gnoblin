# Split a config into files

Import fragments from `init.ts` and put them in `defineConfig`, in the order you want them applied. In `appearance.ts`:

```ts
// appearance.ts
export const windowManagement = { focusMode: "click" } as const;
export const cursor = { size: 28 } as const;
```

In `bindings.ts`:

```ts
// bindings.ts
import { spawn } from "gnoblin";

export const keys = { "Super+d": spawn("fuzzel") };
```

In `init.ts`:

```ts
import { defineConfig } from "gnoblin";
import { cursor, windowManagement } from "./appearance.ts";
import { keys } from "./bindings.ts";

export default defineConfig({ windowManagement, cursor, keys });
```

Modules share the same `gnoblin` library, and a module that exports nothing still runs when it is imported. A path
starts with `./`, names the file with its extension and resolves under the config directory. There is no glob.
See [files and load order](/guides/files_and_load_order) for how settings from several files combine.
