# Autostart

[Configuration reference](/config/configure/autostart)

Use `autostart` for commands that should start in your session. Add this to
`~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {
    autostart = {
        waybar = {command = {"waybar"}},
    },
}
```

Install the program first. Gnoblin runs the command directly, without shell
expansion; use one string for each argument. If you need pipes or redirection,
explicitly run a shell.

`when = "on_login"` is the only supported trigger and is the default. Gnoblin
starts each named entry once per login and does not restart it after exit.

A new name starts after a successful config reload if you are already logged
in; otherwise it starts at the next login.

If a command changes or an entry is removed or disabled, Gnoblin sends
`SIGTERM` to the process group it started, then sends `SIGKILL` if it has not
exited after 200 ms. Descendants that remain in the group are included. It
starts the updated command if enabled. See the
[autostart reference](/config/configure/autostart) for launch failures.

Gnoblin starts configured commands after Mutter publishes the Wayland display.
It also launches XDG autostart entries at this point, so graphical login apps
do not start before the compositor is ready. When using Bingux, load its
Gnoblin module to start its shell and layer clients through this same
configuration path.

## Override an imported command

Use the same name to change its arguments:

```lua
gnoblin.configure {
    autostart = {
        waybar = {
            command = {"waybar", "--config", "/home/you/.config/waybar/work.jsonc"},
        },
    },
}
```

Replace the path with your own. Omitted fields keep their earlier values.

## Disable an entry

Set `enable = false` on the named entry to prevent it starting in future
sessions:

```lua
gnoblin.configure {
    autostart = {
        waybar = {enable = false},
    },
}
```

An unknown name adds a disabled entry. Disabling or removing an entry stops its Gnoblin-owned process group on reload.

## When does it run?

- A new name starts after Gnoblin accepts a config reload.
- Each name gets one launch per login. Saving again or unlocking does not
  start a second copy.
- Changing the command stops the owned process group and starts the updated
  command. Gnoblin sends `SIGKILL` after 200 ms if the group ignores `SIGTERM`.
- Disabling or removing an entry stops the owned process group.

See [command syntax](/guides/shortcuts#commands-and-shell-syntax) for shell
commands and [configuration loading](/guides/files_and_load_order) for how
named entries combine across files.
