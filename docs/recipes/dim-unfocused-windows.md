# Dim unfocused windows

Set the opacity of application windows that are not focused:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

Opacity affects text and controls too. For a translucent panel with opaque text,
use transparency in the client's background instead. Remove this rule to
inherit the previous matching opacity again.
