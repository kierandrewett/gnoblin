# Make Caps Lock an Escape key

Add the `caps:escape` XKB option to the active keyboard configuration:

```lua
gnoblin.configure {
    input = {
        keyboard = {
            xkb_options = {"caps:escape"},
        },
    },
}
```

This replaces the active XKB option list. If you already use other options,
include them too, for example `{ "caps:escape", "compose:ralt" }`. See the
[keyboard reference](/config/configure/input/keyboard) for more options.
