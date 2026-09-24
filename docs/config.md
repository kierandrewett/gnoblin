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

## Migrate older config calls

You can keep existing configs and migrate them a piece at a time. Replace the older named-entry calls with the matching `gnoblin.configure` map:

| Older call                                  | Current form                                          |
| ------------------------------------------- | ----------------------------------------------------- |
| `gnoblin.shortcut {name = "terminal", ...}` | `gnoblin.configure.shortcuts.terminal = {...}`        |
| `gnoblin.autostart {name = "panel", ...}`   | `gnoblin.configure.autostart.panel = {...}`           |
| `gnoblin.remove_shortcut("terminal")`       | `gnoblin.configure.shortcuts.terminal.enable = false` |
| `gnoblin.remove_autostart("panel")`         | `gnoblin.configure.autostart.panel.enable = false`    |

For `gnoblin.set`, move the values into `gnoblin.configure` and use the public keys from the reference. For example, change the internal `minimize-duration` key to `minimize_duration`:

```lua
-- Older form
gnoblin.set {shell = {["minimize-duration"] = 150}}

-- Current form
gnoblin.configure {shell = {minimize_duration = 150}}
```

Disabling an autostart entry affects future launches; it does not stop a process that is already running.

## First config

On first login, `gnoblin-session` copies the packaged reference config to `~/.config/gnoblin/init.lua` when no Lua or legacy config exists. It does not replace an existing config. Edit that file to configure Gnoblin.

```sh
gnoblinctl config path
gnoblinctl config reload
```

See [configuration recipes](/recipes) for complete examples and the [guides](/guides/window_rules) for task-based instructions.
