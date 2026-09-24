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

This adds a terminal shortcut and dims unfocused windows. It does not install
or start a desktop shell; choose one in [shell setup](/bring-your-own-shell).
If an imported shortcut already uses Super+Enter under another name, override
that name instead.
