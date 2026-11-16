# Configuration

Gnoblin is configured with one TypeScript file, `~/.config/gnoblin/init.ts`. The file is a program: it runs from top to
bottom against live objects such as `gnoblin.workspaces`, `gnoblin.input` and `gnoblin.clients`, and what it sets is
what the desktop does. There is no config object and nothing is layered under the file.

A file replaces the stock setup, and a statement you delete is gone after the next reload.

```ts
import { FloatingLayout, Workspace } from "gnoblin-std";

gnoblin.workspaces.add(new Workspace({ id: "main", layout: new FloatingLayout() }));
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

## Where to go

- [TypeScript shell](/typescript-shell): write your first config, where it lives and how it reloads.
- [Settings in init.ts](/js-settings): what you can set on each object, with an example, and the settings that have no
  TypeScript home yet.
- [Settings reference](/config/configure): the pages for devices, monitors, dialogs and the other settings.
- [Shortcuts](/guides/shortcuts): bind keys and gestures.
- [Files and load order](/guides/files_and_load_order): split a config over several files.
- [`gnoblinctl`](/gnoblinctl): check, reload and inspect the running configuration.
- [Migrating from Lua](/migrating-from-lua): convert an `init.lua` with `gnoblinctl config migrate`, and see what each construct became.

## First config

Write a commented starter `init.ts` and the editor files that give you completion, then reload the session:

```sh
gnoblinctl config init
gnoblinctl config reload
```

`config init` never replaces a config. A running shell also reloads the file when you save it. See
[configuration recipes](/recipes/) for complete examples and the [guides](/guides/shortcuts) for task-based
instructions.
