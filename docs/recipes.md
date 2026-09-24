# recipes

[Configuration reference](/config/reference)

Add these snippets to `~/.config/gnoblin/init.lua`, after any `gnoblin.load(...)`
lines. Those lines load settings from other files; placing your changes last
lets them override the loaded values.

## Complete starter config

Save this as `~/.config/gnoblin/init.lua`.
Install the commands you use; replace `ptyxis` with your terminal.

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

gnoblin.shortcut {
    name = "terminal",
    binding = "<Super>Return",
    command = {"ptyxis", "--new-window"},
}

gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

This adds a terminal shortcut and dims unfocused windows. It does not install or
start a desktop shell; choose one in [shell setup](/bring-your-own-shell).
If an imported shortcut already uses Super+Enter under another name, override
that name instead.

## Add a shortcut without losing the others

```lua
gnoblin.shortcut {
    name = "my-terminal",
    binding = "<Super>Return",
    command = {"ptyxis", "--new-window"},
}
```

Use an installed terminal. To change an imported shortcut, use its existing name.
Only the fields you supply change; its binding stays the same:

```lua
gnoblin.shortcut {
    name = "terminal",
    command = {"ptyxis", "--new-window"},
}
```

To remove an imported shortcut:

```lua
gnoblin.remove_shortcut("terminal")
```

## Make unfocused windows slightly translucent

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

Opacity affects text and controls too. Use client background transparency for
a translucent panel with opaque text. Remove this rule to inherit the previous
matching opacity again.

## Round application windows

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 14, smoothing = 0.6, mode = "auto"},
}
```

Radius uses logical pixels; smoothing is a 0–1 shape parameter. Automatic mode
preserves existing client corners. See [corners](/config/window_effects#rounded-window-corners)
for state exceptions and how to force a mask deliberately.

## Turn off compositor layer animations

```lua
gnoblin.configure {shell = {layer_animation = "none"}}
gnoblin.window_rule {
    match = {type = "layer"}, animation = "none",
}
```

The last rule overrides animation choices in earlier component rules. This
controls bars and popups appearing or disappearing; a shell may animate its own contents
separately. Configure those animations in that shell.

## Let apps request a Gnoblin titlebar {#enable-negotiated-server-decorations}

```lua
gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "native", extents = {36, 0, 0, 0}},
}
```

This draws a 36-pixel titlebar when an app asks Gnoblin to provide its frame.
Apps that draw their own titlebars keep them. `extents` gives the top, right,
bottom and left sizes in logical pixels. See [titlebar modes](/config/window_frames).

## Combine rules

The first rule applies to all application windows. The second changes only
opacity when a window loses focus; the corner settings remain in effect.

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 12, smoothing = 0.5},
    opacity = 1,
}

gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

## Split a config into files

In `init.lua`, load files in the order you want them applied:

```lua
gnoblin.load("conf.d/**/*.lua")
gnoblin.load("appearance.lua")
gnoblin.load("bindings.lua")
```

In `appearance.lua`:

```lua
gnoblin.configure {
    shell = {minimize_duration = 120, layer_duration = 180},
}
```

In `bindings.lua`:

```lua
gnoblin.shortcut {
    name = "launcher",
    binding = "<Super>d",
    command = {"fuzzel"},
}
```

Files share the same API. No `require("gnoblin")` or return statement is needed.

## Apply a recipe

```sh
gnoblinctl config reload
```

Check the affected window or shortcut. See [troubleshooting](/troubleshooting)
if nothing changes.
