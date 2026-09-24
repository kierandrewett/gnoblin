# Start programs at login

[Configuration reference](configuration-reference.md)

Use the named `autostart` table for simple commands. Use a systemd user service when a program
needs automatic restarts.

## Add a program

Add this to `~/.config/gnoblin/init.lua`, after any `gnoblin.load(...)` lines:

```lua
gnoblin.configure {
    autostart = {
        waybar = {command = {"waybar"}},
    },
}
```

Install the program first. Reusing a name updates that entry.
Do not add a program already started by a service, such as Bingux.

## Function form

Existing configs can use `gnoblin.autostart {name = "waybar", command = {"waybar"}}`.
The same name and `enable = false` disable an imported entry.

## Override an imported command

Use the same name to change its arguments:

```lua
gnoblin.configure {
    autostart = {
        waybar = {command = {"waybar", "--config", "/home/you/.config/waybar/work.jsonc"}},
    },
}
```

Replace the path with your own. If Waybar has already started, this command
change takes effect at your next login.

## Remove an entry

After loading the file that defines `waybar`, disable it directly:

```lua
gnoblin.configure.autostart.waybar.enable = false
```

The name must already exist. If it does not, Lua reports an error instead of
silently changing nothing. Disabling an entry does not stop an already running
process. The named map form, `gnoblin.configure {autostart = {waybar =
{enable = false}}}`, also works; `gnoblin.remove_autostart(name)` remains for
older configs.

## When does it run?

- A new name starts on the next config reload.
- Each name starts once per login.
- Saving again or unlocking does not start a second copy.
- An exited process is not automatically restarted.
- Changing the command for a program already launched takes effect at the next login.
- Removing the entry does not stop the running process.

Failed launches are logged and can be retried on a later reload.

## Command arguments

Use one string per argument. Commands run without shell expansion, so use
absolute paths or programs on PATH. For pipes or redirection, explicitly
invoke a shell; see [command syntax](shortcuts.md#commands-and-shell-syntax).
