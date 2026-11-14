# Organize a growing config

Keep the entry point short and group settings by the kind of change you make. For example, let `init.ts` import your
rules and your shortcuts, and combine them in one `defineConfig`.

In `rules.ts`:

```ts
// rules.ts
export const rules = [{ appId: "org.gnome.TextEditor", maximised: true }];
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
import { keys } from "./bindings.ts";
import { rules } from "./rules.ts";

export default defineConfig({ keys, rules });
```

Imported files share the same Gnoblin library and need no registration. Keep the combining in one place, `init.ts`, so
you can see which file supplies each setting. Lists such as `rules`, `autostart` and `xkbOptions` replace the stock
list, so list every value you intend to keep. To add to a list from another file, spread it:
`rules: [...baseRules, ...localRules]`.

Check the files with `gnoblinctl check init.ts`, and read [file loading and order](/guides/files_and_load_order) before
adding overlapping files.
