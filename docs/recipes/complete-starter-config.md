# Complete starter config

Save this as `~/.config/gnoblin/init.lua`. Install the commands you use;
replace `ptyxis` with your terminal.

```lua
-- Keep your shell installer's includes if it provided different paths.
gnoblin.load("/usr/share/gnoblin/conf.d/*.lua")
gnoblin.load("conf.d/**/*.lua")

gnoblin.configure {
    shell = {
        minimize_animation = "zoom",
        minimize_duration = 150,
    },
}

gnoblin.configure {shortcuts = {terminal = {binding = "<Super>Return", command = {"ptyxis", "--new-window"}}}}

gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

| Setting              | Accepted values                                               | Default  | Effect                                                                                 |
| -------------------- | ------------------------------------------------------------- | -------- | -------------------------------------------------------------------------------------- |
| `minimize_animation` | `"zoom"`, `"fade"`, `"none"`, `"gnome"`, or a registered name | `"zoom"` | Selects the minimize motion. See [animation settings](/config/configure/shell).        |
| `minimize_duration`  | Integer milliseconds from 0 to 5000                           | `200`    | Sets the motion duration; this example uses 150 ms.                                    |
| `binding`            | One GTK accelerator string                                    | Not set  | Runs the shortcut when pressed. See [accelerator syntax](/guides/shortcuts#key-names). |
| `command`            | Nonempty array of strings                                     | Not set  | Executable followed by its arguments; Gnoblin does not expand shell syntax.            |
| `focused`            | Boolean                                                       | Not set  | `false` matches application windows without focus.                                     |
| `opacity`            | Number from 0 to 1                                            | `1`      | Sets matching windows' opacity; 1 is fully opaque.                                     |

Save the file and run `gnoblinctl config reload` to apply it. See the
[shortcut reference](/config/configure/shortcuts) and
[window rule reference](/config/window_rule) for field constraints and
precedence.

This adds a terminal shortcut and dims unfocused windows. It does not install
or start a desktop shell; choose one in [shell setup](/bring-your-own-shell).
If an imported shortcut already uses Super+Enter under another name, override
that name instead. See the [shell settings](/config/configure/shell),
[shortcut settings](/config/configure/shortcuts), and
[window rule reference](/config/window_rule) for accepted values.
