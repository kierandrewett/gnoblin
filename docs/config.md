# Config API

Use [`gnoblin.configure`](/config/configure) for settings, named shortcuts and autostart commands. The reference pages list the supported fields and defaults.

## Functions {#api-reference}

- [`gnoblin.configure`](/config/configure) — set compositor, shell, input and window-management options.
- [`gnoblin.window_rule`](/config/window_rule) — add a window or layer-surface rule.
- [`gnoblin.permission_rule`](/config/permission_rule) — add a portal permission rule.
- [`gnoblin.animation`](/config/animation) — register a named animation.
- [`gnoblin.config`](/config/config) — inspect or edit the config assembled during loading.
- [`gnoblin.load`](/config/load) — load another Lua config file.
- [`gnoblin.snapshot`](/config/snapshot) — inspect a copy of the current config.
- [`gnoblin.array`](/config/array) — mark a Lua table as a list.
- [`gnoblin.on`](/config/lua-events) — handle compositor and input events.

The named views [`gnoblin.configure.shortcuts`](/config/configure/shortcuts) and [`gnoblin.configure.autostart`](/config/configure/autostart) read or update entries by name. See the [shortcuts](/guides/shortcuts) and [autostart](/guides/autostart) guides. Lua's `require` loader is covered under [`gnoblin.load`](/config/load).

## First config

On first login, `gnoblin-session` copies the packaged reference config to
`~/.config/gnoblin/init.lua` when that file does not exist. It does not replace
an existing config. Edit that file to configure Gnoblin.

```sh
gnoblinctl config path
gnoblinctl config reload
```

See [configuration recipes](/recipes/) for complete examples and the [guides](/guides/window_rules) for task-based instructions.
