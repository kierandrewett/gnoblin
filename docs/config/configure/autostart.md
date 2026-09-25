# gnoblin.configure.autostart

`autostart` maps each entry name to a command Gnoblin starts in the user
session. Put these settings in `~/.config/gnoblin/init.lua`.

| Field     | Accepted values                       | Default and effect                                   |
| --------- | ------------------------------------- | ---------------------------------------------------- |
| Name      | 1–80 letters, numbers, `_` or `-`     | Identifies the entry when another config merges it.  |
| `command` | Nonempty array of strings             | Required. Runs directly, without shell expansion.    |
| `when`    | `"on_login"`                          | `"on_login"`; currently the only supported trigger.  |
| `restart` | `"never"`, `"on_failure"`, `"always"` | `"never"`; retry after 2 seconds when selected.      |
| `enable`  | Boolean                               | `true`; set to `false` to disable an imported entry. |

Use the same name to override an imported command. Omitted fields keep their
earlier values. Disabling an entry prevents future launches but does not stop
a process that is already running.

```lua
gnoblin.configure {
    autostart = {
        panel = {command = {"waybar", "--config", "/home/you/.config/waybar/config"}},
    },
}
```

If an earlier config file defines `waybar`, you can change it directly by
name. `enable` defaults to `true`:

```lua
gnoblin.configure.autostart.waybar.enable = false
```

![Firefox below Waybar in a Gnoblin session](../../images/gnoblin-waybar-firefox.png)

_Waybar starts as an independent session client; Firefox is a regular application window._

Loop over autostart entries loaded earlier in the config:

```lua
for name, entry in pairs(gnoblin.configure.autostart) do
    if entry.enable then
        print(name, table.concat(entry.command, " "))
    end
end
```

Use one array item per command argument. Commands run directly, so pipes,
redirection and other shell syntax require explicitly launching a shell. See
[command syntax](/guides/shortcuts#commands-and-shell-syntax) and the
[autostart guide](/guides/autostart) for launch timing and overrides.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`["entry-name"]` stands for any valid entry name.

```lua
gnoblin.configure {
    autostart = {
        ["entry-name"] = {
            command = {string, ...},
            when = "on_login"?,
            restart = "never" | "on_failure" | "always"?,
            enable = boolean?,
        },
    },
}
```
