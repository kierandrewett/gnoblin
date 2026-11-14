# Configuration

Gnoblin is configured with one TypeScript file, `~/.config/gnoblin/init.ts`, that makes one `defineConfig` call. The
reference pages list the supported keys, their accepted values and defaults.

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    gaps: { inner: 8, outer: 8 },
    keys: { "Super+Return": spawn("foot") },
});
```

## Where to go

- [TypeScript shell](/typescript-shell): write your first config, where it lives and how it reloads.
- [`defineConfig` reference](/typescript-config): every key, its values and its defaults.
- [Settings reference](/config/configure): one page for each section of the compositor's settings.
- [Window rules](/config/window_rule): place and style windows by app or title.
- [Shortcuts](/guides/shortcuts) and [autostart](/guides/autostart): bind keys and start programs.
- [Files and load order](/guides/files_and_load_order): split a config over several files.
- [`gnoblinctl`](/gnoblinctl): check, reload and inspect the running configuration.
- [Migrating from Lua](/migrating-from-lua): what each Lua construct became.

## First config

Write a commented starter `init.ts` and the editor files that give you completion, then reload the session:

```sh
gnoblinctl config init
gnoblinctl config reload
```

`config init` never replaces a config. A running shell also reloads the file when you save it. See
[configuration recipes](/recipes/) for complete examples and the [guides](/guides/window_rules) for task-based
instructions.
