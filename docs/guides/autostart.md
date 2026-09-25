# autostart

[Configuration reference](/config/configure)

Use `autostart` for commands that should start with the session. Set its
`restart` policy when the process should be relaunched after it exits.

## Add a program

Add this to `~/.config/gnoblin/init.lua`, after any `gnoblin.load(...)` lines:

```lua
gnoblin.configure {
    autostart = {
        waybar = {command = {"waybar"}},
    },
}
```

`when` is optional; it defaults to `"on_login"`. Gnoblin currently supports
that trigger, which starts the command once in the session. A new entry added
after login starts when the config reloads. Reusing a name updates the entry.
Install the program first.
Do not add a program already started by a service, such as Bingux.

## Override an imported command

Use the same name to change its arguments:

```lua
gnoblin.configure {
    autostart = {
        waybar = {
            command = {"waybar", "--config", "/home/you/.config/waybar/work.jsonc"},
            restart = "on_failure",
        },
    },
}
```

Replace the path with your own. If Waybar has already started, the changed
command takes effect at your next login. `restart` accepts `"never"` (the
default), `"on_failure"`, or `"always"`. Gnoblin waits two seconds between
restarts. Use `"always"` for a long-running client that should recover after
any exit.

## Remove an entry

To prevent an imported program from starting next time, disable its named entry
after the file that adds it:

```lua
gnoblin.configure {
    autostart = {
        waybar = {enable = false},
    },
}
```

An unknown name adds a disabled entry. Disabling an entry does not stop an
already running process.

## When does it run?

- A new name starts on the next config reload.
- Each name gets one initial launch per login. A configured restart policy may
  launch it again after it exits.
- Saving again or unlocking does not start a second copy.
- An exited process follows its `restart` policy. The default, `"never"`,
  does not restart it; `"on_failure"` restarts unsuccessful exits, and
  `"always"` restarts every exit.
- Changing the command for a program already launched takes effect at the next login.
- Removing the entry does not stop the running process.

Gnoblin waits two seconds before a restart. Failed launches are logged; when a
restart policy is enabled, Gnoblin retries them using the same delay.

![A Mako notification displayed in a Gnoblin session](../images/gnoblin-mako-notification.png)

_A notification daemon such as Mako runs as a separate autostarted client._

## Command arguments

Use one string per argument. Commands run without shell expansion, so use
absolute paths or programs on PATH. For pipes or redirection, explicitly
invoke a shell; see [command syntax](/guides/shortcuts#commands-and-shell-syntax).
