# Config API

Use [`gnoblin.configure`](/config/configure) for settings, named shortcuts and autostart commands. The reference pages list the supported fields and defaults.

## Functions

- [`gnoblin.configure`](/config/configure) — set compositor, shell, input and window-management options.
- [`gnoblin.window_rule`](/config/window_rule) — add a window or layer-surface rule.
- [`gnoblin.permission_rule`](/config/permission_rule) — add a portal permission rule.
- [`gnoblin.load`](/config/load) — load another Lua config file.
- [`gnoblin.snapshot`](/config/snapshot) — inspect a copy of the current config.
- [`gnoblin.array`](/config/array) — mark a Lua table as a list.

The named views [`gnoblin.configure.shortcuts`](/config/configure/shortcuts) and [`gnoblin.configure.autostart`](/config/configure/autostart) read or update entries by name. See the [shortcuts](/guides/shortcuts) and [autostart](/guides/autostart) guides. Lua's `require` loader is covered under [`gnoblin.load`](/config/load).

## First config

On first login, `gnoblin-session` copies the packaged reference config to `~/.config/gnoblin/init.lua` when no Lua or legacy config exists. It does not replace an existing config. Edit that file to configure Gnoblin.

```sh
gnoblinctl config path
gnoblinctl config reload
```

See [configuration recipes](/recipes) for complete examples and the [guides](/guides/window_rules) for task-based instructions.

## Deprecated compatibility functions

`gnoblin.shortcut`, `gnoblin.autostart`, `gnoblin.remove_shortcut`,
`gnoblin.remove_autostart`, and `gnoblin.set` are deprecated. Older configs may
continue to use them for now, but they may be removed at any time. Each config
refresh that executes one prints a warning with its migration path. Migrate to
`gnoblin.configure` now.

- Replace `gnoblin.shortcut {name = "terminal", binding = ..., command = ...}`
  with `gnoblin.configure {shortcuts = {terminal = {binding = ..., command = ...}}}`.
- Replace `gnoblin.autostart {name = "panel", command = ...}` with
  `gnoblin.configure {autostart = {panel = {command = ...}}}`.
- Replace `gnoblin.remove_shortcut("terminal")` by removing `terminal` from
  `gnoblin.configure.shortcuts`; to disable an imported shortcut, set
  `gnoblin.configure.shortcuts.terminal.enable = false`.
- Replace `gnoblin.remove_autostart("panel")` by removing `panel` from
  `gnoblin.configure.autostart`; to disable an imported command, set
  `gnoblin.configure.autostart.panel.enable = false`.
- Replace `gnoblin.set { ... }` with `gnoblin.configure { ... }`, using public
  `snake_case` setting names.
