# Files and load order

[Configuration reference](/config/configure)

Start with one `shell.ts`. Split it into modules when that makes it easier to read. A module is an ordinary TypeScript
file that `shell.ts` imports.

## Which file is loaded?

The shell reads `shell.ts` from the config directory, `~/.config/gnoblin` or `$XDG_CONFIG_HOME/gnoblin` when that
variable is set. A `shell.js` beside it is an error, because two shell files leave nothing to choose by.

- The `GNOBLIN_SHELL` environment variable names a file or an installed shell instead.
- `[shell]` in `config.toml` does the same from a file.

An already-running compositor does not see a variable you set in a terminal.

When no user config exists, the session runs the stock config described in the
[TypeScript shell](/typescript-shell#with-no-shell-ts). `gnoblinctl config init` writes a starter `shell.ts` you can edit.

There are no system drop-in directories: `/etc/gnoblin/conf.d` and `gnoblin/conf.d` under `XDG_DATA_DIRS` are not
read by the shell. Put a setting you want from a package in your `shell.ts`.

## Split a config into files

An import runs the module. Put each concern in its own file as a function that takes what it needs and does its calls,
and call the functions from `shell.ts`:

```ts
// keys.ts
export function bindKeys(input: typeof gnoblin.input.primary): void {
    input.bind("Super+Return", () => {
        gnoblin.spawn(["foot"]);
    });
    input.bind("Super+d", () => {
        gnoblin.spawn(["fuzzel"]);
    });
}
```

```ts
// appearance.ts
import "gnoblin-std";

export function styleWindows(): void {
    gnoblin.clients.style.cornerRadius = 12;
}
```

`gnoblin.clients.style` is part of `gnoblin-std`, which a file loads with `import "gnoblin-std";`, so `appearance.ts`
imports it before it uses the style.

```ts
import { styleWindows } from "./appearance.ts";
import { bindKeys } from "./keys.ts";

styleWindows();
bindKeys(gnoblin.input.primary);
```

- A path starts with `./` or `../` and names the file with its extension. It resolves under the config directory, and a
  path outside it is refused.
- There is no glob. List each file you import. The order of the calls in `shell.ts` is the order they run.
- A module that is imported twice runs once.
- A module is a program, so it can compute a value or read the `gnoblin` object like `shell.ts` can. Keep it quick: the
  whole load runs inside one time budget.
- An import that does not resolve, a syntax error or a throw in any file fails the load at the line you wrote, in that
  file's own coordinates. The running config stays.

## Replace or add?

Your `shell.ts` replaces the stock default as a whole: nothing is layered under it, so a stock key or workspace you want
is a line you keep. Bind a key once: a second `bind` of a trigger that is already bound is refused, so bind it from one
file only.

```ts
const common = ["Super+Return", "Super+d"];
const laptop = ["XF86MonBrightnessUp"];

for (const trigger of [...common, ...laptop]) {
    gnoblin.input.primary.bind(trigger, () => {
        gnoblin.spawn(["notify-send", trigger]);
    });
}
```

- Later statements run later. A style set later on the same scope overwrites an earlier one.
- Several modules can add to the same collection: `gnoblin.workspaces.add` and `gnoblin.clients.addListener` accumulate.

## What a config file can do

`shell.ts` and its modules are TypeScript run by the shell. They can use the language's own features and the
`gnoblin` object. Three limits apply:

- They cannot read files, and they cannot run programs while loading. Start a program with a key or
  `gnoblin.spawn` from a function that runs after the load.
- An endless loop is cut off by the time budget.
- Each reload evaluates the files again, so a variable does not survive it.

## Reload

A save reloads a running session, and so does `gnoblinctl config reload`. The shell watches the config directory, and
any module file in it that changes triggers one reload. `GNOBLIN_JS_WATCH=0` turns that off, for a file system that
cannot be watched.

A reload commits the whole document your files describe, so a setting you delete is back to its default. A failed reload keeps the running config, and `gnoblinctl config reload` prints the error with its file, line and
column.

Check a set of files without a session:

```sh
gnoblinctl check shell.ts
```
