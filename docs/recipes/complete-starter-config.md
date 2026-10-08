# Complete starter config

Save this as `~/.config/gnoblin/init.lua`. Install the commands you use;
replace `ptyxis` with your terminal.

```lua
-- Load optional system defaults first, then personal fragments.
gnoblin.load("/usr/share/gnoblin/conf.d/*.lua")
gnoblin.load("conf.d/**/*.lua")

gnoblin.configure {
    window_management = {
        focus_mode = "click",
    },
}

gnoblin.configure {shortcuts = {terminal = {binding = "<Super>Return", command = {"ptyxis", "--new-window"}}}}

gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

| Setting             | Accepted values                     | Default   | Effect                                                                                 |
| ------------------- | ----------------------------------- | --------- | -------------------------------------------------------------------------------------- |
| `focus_mode`        | `"click"`, `"hover"`, or `"hover-strict"` | `"click"` | Selects when pointer or click input changes focus.                                     |
| `binding`           | One GTK accelerator string          | Not set   | Runs the shortcut when pressed. See [accelerator syntax](/guides/shortcuts#key-names). |
| `command`           | Nonempty array of strings           | Not set   | Executable followed by its arguments; Gnoblin does not expand shell syntax.            |
| `focused`           | Boolean                             | Not set   | `false` matches application windows without focus.                                     |
| `opacity`           | Number from 0 to 1                  | `1`       | Sets matching windows' opacity; 1 is fully opaque.                                     |

Save the file and run `gnoblinctl config reload` to apply it. See the
[shortcut reference](/config/configure/shortcuts) and
[window rule reference](/config/window_rule) for field constraints and
precedence.

This sets click-to-focus, adds a terminal shortcut, and dims unfocused windows.

It does not install or start a desktop shell; choose one in
[shell setup](/bring-your-own-shell).
If an imported shortcut already uses Super+Enter under another name, override
that name instead. See the [configuration reference](/config/configure),
[shortcut settings](/config/configure/shortcuts), and
[window rule reference](/config/window_rule) for accepted values.
