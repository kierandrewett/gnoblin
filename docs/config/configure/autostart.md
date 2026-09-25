# gnoblin.configure.autostart

`autostart` maps each entry name to a command Gnoblin starts in the user
session. Put these settings in `~/.config/gnoblin/init.lua`.

| Field     | Accepted values           | Default and effect                                   |
| --------- | ------------------------- | ---------------------------------------------------- |
| Name      | Nonempty name             | Identifies the entry when another config merges it.  |
| `command` | Nonempty array of strings | Required. Runs directly, without shell expansion.    |
| `when`    | `"on_login"`              | `"on_login"`; currently the only supported trigger.  |
| `enable`  | Boolean                   | `true`; set to `false` to disable an imported entry. |

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

Use one array item per command argument. Commands run directly, so pipes,
redirection and other shell syntax require explicitly launching a shell. See
[command syntax](/guides/shortcuts#commands-and-shell-syntax) and the
[autostart guide](/guides/autostart) for launch timing and overrides.
