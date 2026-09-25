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
Cursor size uses logical pixels and accepts integers from 1 to 256. These
settings apply on config reload; omitted cursor fields keep the current
GNOME/Mutter preference.

| Setting                       | Values                                  | Default              | Effect                                                 |
| ----------------------------- | --------------------------------------- | -------------------- | ------------------------------------------------------ |
| `cursor.theme`                | Installed Hyprcursor theme name         | Current preference   | Selects the cursor theme.                              |
| `cursor.size`                 | Integer from 1 to 256 logical pixels    | Current preference   | Sets the cursor size.                                  |
| `compositor.locate_pointer`   | Boolean                                 | `false`              | Enables the pointer-location effect.                   |
| `compositor.audible_bell`     | Boolean                                 | `true`               | Plays a sound when a client requests the audible bell. |
| `compositor.visual_bell`      | Boolean                                 | `false`              | Flashes when a client requests the visual bell.        |
| `compositor.visual_bell_type` | `"fullscreen-flash"` or `"frame-flash"` | `"fullscreen-flash"` | Flashes the display or focused frame.                  |

`locate_pointer` enables the compositor's pointer-location gesture; it does
not enlarge or constantly animate the pointer.

The visual bell only responds to client bell requests. It does not replace
application notifications or guarantee that every application sends a bell.
Reload the config, then test the pointer-location gesture configured by your
session and a client that supports the bell. See [cursor setup](/guides/cursors)
and the [compositor reference](/config/configure/compositor).
