# gnoblin.configure.autostart

`autostart` maps a name to a command that Gnoblin starts in the user session.
Put this in `~/.config/gnoblin/init.lua`. A new entry starts after reload if
you are already logged in; otherwise it starts at the next login.

| Field     | Accepted values                              | Default and effect                                                                         |
| --------- | -------------------------------------------- | ------------------------------------------------------------------------------------------ |
| Name      | Nonempty, unique name in the `autostart` map | Identifies the entry when another config merges or disables it.                            |
| `command` | Nonempty array of strings                    | Required. Runs directly, without shell expansion.                                          |
| `when`    | `"on_login"`                                 | `"on_login"`; starts once per session login. This is currently the only supported trigger. |
| `enable`  | Boolean                                      | `true`; set to `false` to disable an imported entry. This does not stop a running process. |
| `restart` | `"never"`, `"on_failure"`, or `"always"`     | `"never"`; controls whether Gnoblin starts the command again after it exits.               |

Reuse a name to change an imported command. Omitted fields keep their earlier
values.

```lua
gnoblin.configure {
    autostart = {
        panel = {
            command = {"waybar", "--config", "/home/you/.config/waybar/config"},
            restart = "on_failure",
        },
    },
}
```

Replace the example path with your own. Use one array item per command
argument. For shell syntax such as pipes, explicitly run a shell; see
[command syntax](/guides/shortcuts#commands-and-shell-syntax).

Use `restart = "always"` for a long-running client that should stay available.
Gnoblin waits two seconds before restarting an exited process.

`"on_failure"` restarts only after a nonzero exit or signal; `"never"` starts
the command once and does not retry a failed launch. When restarting is
enabled, Gnoblin also retries a command that fails to launch, using the same
two-second delay. Gnoblin's built-in wallpaper renderer does not use autostart.

See the [autostart guide](/guides/autostart) for launch timing, overrides,
and process behavior.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`["entry-name"]` stands for any valid entry name.

```lua
gnoblin.configure {
    autostart = {
        ["entry-name"] = {
            command = {string, ...},
            when = "on_login"?,
            enable = boolean?,
            restart = "never" | "on_failure" | "always"?,
        },
    },
}
```
