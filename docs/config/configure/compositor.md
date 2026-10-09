# gnoblin.configure.compositor

Configure this part of `gnoblin.configure` with the `compositor` key.

Put these fields inside `gnoblin.configure {compositor = {...}}`. Changes apply
on configuration reload. Omitted values keep the defaults shown below.

Animations run only when selected through animation declarations or window
rules. There is no global animation switch. See the
[animation guide](/guides/animations).

| Key                  | Values                                  | Default              | Effect                                                   |
| -------------------- | --------------------------------------- | -------------------- | -------------------------------------------------------- |
| `locate_pointer`     | Boolean                                 | `false`              | Sends `gnoblin.pointer.locate-requested` when the key is pressed. |
| `locate_pointer_key` | Mutter key name, `"disabled"`, or `""`  | `"Control_L"`        | Key that triggers the pointer locator.                   |
| `visual_bell`        | Boolean                                 | `false`              | Flashes the screen when an app requests the visual bell. |
| `audible_bell`       | Boolean                                 | `true`               | Plays a sound when an app requests the audible bell.     |
| `visual_bell_type`   | `"fullscreen-flash"` or `"frame-flash"` | `"fullscreen-flash"` | Flash the full screen or the focused window.             |

Gnoblin draws nothing for the pointer locator. With `locate_pointer = true`,
pressing `locate_pointer_key` sends the `gnoblin.pointer.locate-requested`
event with the pointer position (`x`, `y`) and its monitor. A shell or a Lua
callback decides how to show it. See [Lua events](/config/lua-events).

For example, enable the pointer locator and use a focused-window flash:

```lua
gnoblin.configure {
    compositor = {
        locate_pointer = true,
        locate_pointer_key = "F12",
        visual_bell = true,
        visual_bell_type = "frame-flash",
    },
}
```

`locate_pointer_key` accepts one XKB keysym name that Mutter recognizes.
Examples are `"Control_L"`, `"Super_R"`, and `"F12"`. A modifier name without a
side, such as `"Control"`, matches both left and right keys.

Use `"disabled"` or an empty string to remove the trigger. The key only has an
effect while `locate_pointer = true`.

Guide: [session settings](/guides/session_settings).

## Type definition

The block below is schema pseudocode in Lua table form. `?` marks an optional
field, and `|` separates accepted alternatives.

```lua
gnoblin.configure {
    compositor = {
        locate_pointer = boolean?,
        locate_pointer_key = string?, -- XKB keysym name, "disabled", or ""
        visual_bell = boolean?,
        audible_bell = boolean?,
        visual_bell_type = "fullscreen-flash" | "frame-flash"?,
    },
}
```
