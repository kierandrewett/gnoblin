# Dim unfocused windows

Set the opacity of application windows that are not focused:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

`focused = false` matches windows that do not currently have focus. `opacity`
is a number from `0` (transparent) to `1` (opaque); the example uses `0.95`.
The rule applies to windows, not layer surfaces.

Opacity affects text and controls too. For a translucent panel with opaque text,
use transparency in the client's background instead. Remove this rule to
inherit the previous matching opacity again.

See the [window rule reference](/config/window_rule) for all match fields and
effect ranges.
