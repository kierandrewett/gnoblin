# Files and load order

[Configuration reference](/config/configure)

Start with one `init.ts`. Split it into modules when that makes it easier to read. A module is an ordinary TypeScript
file that `init.ts` imports.

## Which file is loaded?

The shell reads `init.ts` from the config directory, `~/.config/gnoblin` or `$XDG_CONFIG_HOME/gnoblin` when that
variable is set. When `init.ts` does not exist it takes `init.js`. The `GNOBLIN_CONFIG` environment variable names one
file instead, and its extension says which shell reads it. An already-running compositor does not see a variable you
set in a terminal.

When no user config exists, the session runs the stock config described in the
[TypeScript shell](/typescript-shell#with-no-init-ts). `gnoblinctl config init` writes a starter `init.ts` you can edit.

There are no system drop-in directories: `/etc/gnoblin/conf.d` and `gnoblin/conf.d` under `XDG_DATA_DIRS` are not
read by the shell. Put a setting you want from a package in your `init.ts`.

## Split a config into files

An import runs the module, and the module's exports are values you put into `defineConfig`. Put each concern in its
own file and combine them in `init.ts`:

```ts
// appearance.ts
export const appearance = { colorScheme: "prefer-dark", accent: "teal" } as const;
export const cursor = { size: 28 } as const;
```

```ts
// keys.ts
import { spawn } from "gnoblin";

export const keys = {
    "Super+Return": spawn("foot"),
    "Super+d": spawn("fuzzel"),
};
```

```ts
import { defineConfig } from "gnoblin";
import { appearance, cursor } from "./appearance.ts";
import { keys } from "./keys.ts";

export default defineConfig({ appearance, cursor, keys });
```

- A path starts with `./` or `../` and names the file with its extension. It resolves under the config directory, and a
  path outside it is refused.
- There is no glob. List each file you import. To control the order of two lists, spread them in the order you want:
  `rules: [...baseRules, ...localRules]`.
- A module that is imported twice runs once.
- A module is a program, so it can compute a value or read the `gnoblin` object like `init.ts` can. Keep it quick: the
  whole load runs inside one time budget.
- An import that does not resolve, a syntax error or a throw in any file fails the load at the line you wrote, in that
  file's own coordinates. The running config stays.

## Override or append?

Settings that are tables merge by key, and lists replace. Build the table or list you want, with a spread when you
want to append:

```ts
import { defineConfig, spawn } from "gnoblin";

const common = { "Super+Return": spawn("foot"), "Super+d": spawn("fuzzel") };
const laptop = { XF86MonBrightnessUp: spawn("brightnessctl", "set", "5%+") };

export default defineConfig({
    keys: { ...common, ...laptop },
    autostart: ["waybar", "nm-applet --indicator"],
});
```

- `keys` merge by trigger, and a key of yours replaces the stock key with the same trigger.
- `rules`, `autostart` and `workspaces` are lists. Yours replaces the stock list as a whole.
- Later properties of an object spread win, so put the file that should have the last word last.

The [defineConfig reference](/typescript-config#how-your-config-combines-with-the-stock-one) lists how each key
combines with the stock config.

## What a config file can do

`init.ts` and its modules are TypeScript run by the shell. They can use the language's own features and the
`gnoblin` object. Three limits apply:

- They cannot read files, and they cannot run programs while loading. Start a program with a key, an `autostart`
  entry or `gnoblin.spawn` from a function that runs after the load.
- An endless loop is cut off by the time budget.
- Each reload evaluates the files again, so a variable does not survive it.

## Reload

A save reloads a running session, and so does `gnoblinctl config reload`. The shell watches the config directory, and
any module file in it that changes triggers one reload. `GNOBLIN_JS_WATCH=0` turns that off, for a file system that
cannot be watched.

A reload commits the whole document your files describe, so a setting you delete is back to its default. The
[TypeScript shell](/typescript-shell#where-the-config-lives) says what a reload does to running `autostart` commands.
A failed reload keeps the running config, and `gnoblinctl config reload` prints the error with its file, line and
column.

Check a set of files without a session:

```sh
gnoblinctl check init.ts
```
