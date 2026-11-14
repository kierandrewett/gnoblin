# TypeScript shell

Configure the shell with one TypeScript file, `init.ts`. You install nothing and run no build step: the shell reads the
file as it is, and reloads it when you save.

Select the TypeScript shell with an environment variable. Unset, the session runs the Lua shell.

```sh
GNOBLIN_SHELL_VM=js gnoblin
```

## Write your first config

Run this once. It writes a short, commented `init.ts` to `~/.config/gnoblin` (`$XDG_CONFIG_HOME/gnoblin` when that is set)
and the files your editor needs for completion:

```sh
gnoblinctl config init
```

```text
ok  init.ts written: /home/me/.config/gnoblin/init.ts
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json

Next:
  1. Open /home/me/.config/gnoblin/init.ts in your editor.
  2. Check it with `gnoblinctl check --types /home/me/.config/gnoblin/init.ts`.
  3. Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request.
```

`config init` never replaces a config. If `init.ts` or `init.js` is already there it stops and says so. Add `--force`
to write the starter over it.

Open the file. As written it is the config the shell runs when there is no `init.ts` at all, so nothing changes until you
edit it. The lines that start with `// ` are examples. Remove the `// ` to switch one on:

```ts
workspaces: [
    { id: "desktop", layout: "floating" },
    { id: "work", layout: "master-stack" },
],
keys: {
    "Super+Return": spawn("foot"),
},
```

Save the file. The shell reloads it after a short pause, and the windows you have open stay where they are. Press
`Super+2` and you are on the new workspace.

## Check a file

```sh
gnoblinctl check --types ~/.config/gnoblin/init.ts
```

`check` loads the file the way the shell does. It reports a syntax error, an import that does not resolve or a throw at
the line you wrote.

With `--types` it also runs a TypeScript compiler when one is installed. That catches a key that does not exist or a value
of the wrong type. Without a compiler it says so and exits 0. See [Check and types](gnoblinctl.md#check-and-types).

## Editor support

`gnoblinctl config init` runs `gnoblinctl types init`, which writes `gnoblin.d.ts` and `tsconfig.json` beside `init.ts`.
An editor that understands TypeScript then completes every key and underlines a wrong one. Run `gnoblinctl types init`
again after an upgrade to refresh `gnoblin.d.ts`. It never replaces a `tsconfig.json` you have.

## What the keys are

Every key of `defineConfig`, with its accepted values, its default and an example, is in the
[defineConfig reference](typescript-config.md). The ones the starter shows are these:

- `gaps`, `decorations` and `workspaces` shape the desktop.
- `keys` binds keys, and `rules` places windows.
- `autostart` starts programs once per session.
- `setup` is a function for anything else, with the whole object API.

A mistake in a key name fails the reload, and the message suggests the key you meant:

```text
config has no "autostrt" (did you mean "autostart"?); expected one of gaps, style, outputs, ...
```

## With no `init.ts`

A session with no `init.ts` is not a bare shell. It runs the stock config:

- one floating workspace, `desktop`;
- `Super+1` to `Super+9` show the workspace at that position, sliding over a quarter of a second, and do nothing when
  there is none;
- `Super+Left` and `Super+Right` move focus between the columns of a `strip` workspace;
- a three-finger swipe slides between workspaces, and the windows follow the fingers;
- the volume, microphone and media keys run `wpctl` and `playerctl`;
- `Super` with a left drag moves a window, and `Super` with a right drag resizes it;
- a titlebar on every window, with a window menu on a right click;
- an alert with Wait and Force quit for a window that stops answering;
- `Super+space` opens a launcher entry that starts what you type.

Creating `init.ts` later loads it at once. An `init.ts` that fails to load when the shell starts is reported in the log,
and the stock config runs until the file is fixed.

## Reloading

Saving `init.ts`, or any module it imports, reloads the shell after a short pause. `gnoblinctl config reload` reloads it on
request: see [`gnoblinctl config reload`](/gnoblinctl#config). Windows keep their identity through a reload, and the new
config is told about every window that exists.

A file that fails to load (a syntax error, a throw, a script that runs away) leaves the running config in place. The failure
is logged once as `file:line:column: message`, in your `.ts` coordinates, and `gnoblinctl config reload` exits 1 with the
same text.

Set `GNOBLIN_JS_WATCH=0` to turn the watcher off, for a config directory on a file system that cannot be watched. The shell
then reloads only when asked.

## Console and inspector

`gnoblinctl console` opens a live prompt into the running shell, and `inspect.clients()`, `inspect.workspaces()`,
`inspect.bindings()` and `inspect.config()` return what the shell holds as plain data. See
[Console and inspector](/js-console).

## XR

`gnoblin.extensions.xr` is `null` until the XR program has a session, so a config guards it and works without a headset.
The stock shell builds the room and places the panels when the session starts. See
[XR mode](/xr#scripting-xr-in-typescript) for the room, the viewport and the bindings that move and resize a panel.

## When the shell stops

The shell is a client of the compositor. If it exits or is killed, the compositor keeps running and answers every client.
With no shell holding the tree:

- a window that maps is placed by the compositor's own policy, a cascade from 32, 32;
- windows the shell had moved go back to the places the compositor gave them.

The session restarts the shell. It attaches again, learns of every window and applies the config again. No window changes
its identity, and programs listed in `autostart` are started again.
