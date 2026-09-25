# Make pointer feedback easier to see

Use a larger cursor, enable the compositor's locate-pointer action, and show a
visual flash when a client requests the audible bell.

```lua
gnoblin.configure {
    cursor = {
        theme = "Adwaita-Hyprcursor",
        size = 32,
    },
    compositor = {
        locate_pointer = true,
        audible_bell = false,
        visual_bell = true,
        visual_bell_type = "frame-flash",
    },
}
```

Gnoblin bundles Adwaita-Hyprcursor. Other themes must be installed first.
The options in this example have these defaults and effects:

| Setting                       | Accepted values and default                                           | Effect                                                 |
| ----------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------ |
| `cursor.size`                 | Integer `1`–`256`; default `24`                                       | Cursor size in logical pixels.                         |
| `compositor.locate_pointer`   | Boolean; default `false`                                              | Enables the compositor's pointer-location gesture.     |
| `compositor.audible_bell`     | Boolean; default `true`                                               | Plays a sound when a client requests the audible bell. |
| `compositor.visual_bell`      | Boolean; default `false`                                              | Shows a visual response to a client bell request.      |
| `compositor.visual_bell_type` | `"fullscreen-flash"` or `"frame-flash"`; default `"fullscreen-flash"` | Flashes the whole display or the focused frame.        |

The pointer-location gesture does not enlarge or constantly animate the
pointer. See the [cursor reference](/config/configure/cursor) and
[compositor reference](/config/configure/compositor) for details.

The visual bell only responds to client bell requests. It does not replace
application notifications or guarantee that every application sends a bell.
Reload the config, then test the pointer-location gesture configured by your
session and a client that supports the bell. See [cursor setup](/guides/cursors)
and the [compositor reference](/config/configure/compositor).
