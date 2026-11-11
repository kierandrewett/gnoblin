# Complete starter config

Save this as `~/.config/gnoblin/init.lua`. Install the commands you use;
replace `ptyxis` with your terminal.

```lua
-- Load personal fragments. System drop-ins from packages load before this file.
gnoblin.load("conf.d/**/*.lua")

gnoblin.configure {
    window_management = {
        focus_mode = "click",
    },
}

gnoblin.configure {shortcuts = {terminal = {binding = "<Super>Return", command = {"ptyxis", "--new-window"}}}}
```

| Setting      | Accepted values                           | Default   | Effect                                                                              |
| ------------ | ----------------------------------------- | --------- | ----------------------------------------------------------------------------------- |
| `focus_mode` | `"click"`, `"hover"`, or `"hover-strict"` | `"click"` | Selects when pointer or click input changes focus.                                  |
| `binding`    | One chord                                 | Not set   | Runs the shortcut when pressed. See [chord syntax](/guides/shortcuts#chord-syntax). |
| `command`    | Nonempty array of strings                 | Not set   | Executable followed by its arguments; Gnoblin does not expand shell syntax.         |

Save the file and run `gnoblinctl config reload` to apply it. See the
[shortcut reference](/config/configure/shortcuts) and
[window rule reference](/config/window_rule) for field constraints and
precedence.

This sets click-to-focus and adds a terminal shortcut. To dim unfocused windows, see
[dim unfocused windows](/recipes/dim-unfocused-windows).

It does not install or start a desktop shell; choose one in
[shell setup](/bring-your-own-shell).
If an imported shortcut already uses Super+Enter under another name, override
that name instead. See the [configuration reference](/config/configure),
[shortcut settings](/config/configure/shortcuts), and
[window rule reference](/config/window_rule) for accepted values.
