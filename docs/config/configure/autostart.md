# gnoblin.configure.autostart

<<<<<<< origin/main
Define commands to run once per login. Entries merge with the same name; omitted fields keep their earlier values. `when` currently accepts only `"on_login"` and defaults to that value. Set `enable = false` to disable an imported entry. Disabling an entry does not stop a process that is already running.
=======
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
>>>>>>> HEAD

```lua
gnoblin.configure {
    autostart = {
<<<<<<< origin/main
        panel = {command = {"waybar"}, when = "on_login"},
=======
        panel = {
            command = {"waybar", "--config", "/home/you/.config/waybar/config"},
            restart = "on_failure",
        },
>>>>>>> HEAD
    },
}
```

Replace the example path with your own. Use one array item per command
argument. For shell syntax such as pipes, explicitly run a shell; see
[command syntax](/guides/shortcuts#commands-and-shell-syntax).

Use `restart = "always"` for a long-running client such as the bundled
wallpaper process. Gnoblin waits two seconds before restarting an exited
process. `"on_failure"` restarts only after a nonzero exit or signal; `"never"`
starts the command once and does not retry a failed launch. When restarting is
enabled, Gnoblin also retries a command that fails to launch, using the same
two-second delay.

See the [autostart guide](/guides/autostart) for launch timing, overrides,
and process behavior.
