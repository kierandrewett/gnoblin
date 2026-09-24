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

Install the selected cursor theme first. Cursor size uses logical pixels and
accepts integers from 1 to 256. `locate_pointer` enables the compositor's
pointer-location gesture; it does not enlarge or constantly animate the
pointer. `frame-flash` flashes the focused frame; use `fullscreen-flash` to
flash the whole display instead.

The visual bell only responds to client bell requests. It does not replace
application notifications or guarantee that every application sends a bell.
Reload the config, then test the pointer-location gesture configured by your
session and a client that supports the bell. See [cursor setup](/guides/cursors)
and the [compositor reference](/config/configure/compositor).
