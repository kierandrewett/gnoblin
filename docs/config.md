# Config API

This section documents the Lua API exposed by Gnoblin. Each reference page is named for its public `gnoblin.*` function. Configuration settings and their defaults are documented under [`gnoblin.configure`](/config/configure).

## Functions

- [`gnoblin.configure`](/config/configure) — set compositor, shell, input and window-management options.
- [`gnoblin.window_rule`](/config/window_rule) — add a window or layer-surface rule.
- [`gnoblin.permission_rule`](/config/permission_rule) — add a portal permission rule.
- [`gnoblin.shortcut`](/config/shortcut) — add or update a named keyboard shortcut.
- [`gnoblin.autostart`](/config/autostart) — add or update a command run at login.
- [`gnoblin.remove_shortcut`](/config/remove_shortcut) — remove a named shortcut.
- [`gnoblin.remove_autostart`](/config/remove_autostart) — remove a named autostart entry.
- [`gnoblin.load`](/config/load) — load another Lua config file.
- [`gnoblin.snapshot`](/config/snapshot) — inspect a copy of the current config.
- [`gnoblin.set`](/config/set) — merge internal setting names into the config.
- [`gnoblin.array`](/config/array) — mark a Lua table as a list.

The named views `gnoblin.configure.shortcuts` and `gnoblin.configure.autostart` read or update entries by name. See the [shortcuts](/guides/shortcuts) and [autostart](/guides/autostart) guides. `require` is Lua's global module loader, not a `gnoblin.*` function; see [`gnoblin.load`](/config/load) for its supported module behavior.

## First config

On first login, `gnoblin-session` copies the packaged reference config to `~/.config/gnoblin/init.lua` when no Lua or legacy config exists. It does not replace an existing config. Edit that file to configure Gnoblin.

```sh
gnoblinctl config path
gnoblinctl config reload
```

See [configuration recipes](/recipes) for complete examples and the [guides](/guides/window_rules) for task-based instructions.
