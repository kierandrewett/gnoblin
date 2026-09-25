# gnoblin.configure.compositor

Configure this part of `gnoblin.configure` with the `compositor` key.

Put these fields inside `gnoblin.configure {compositor = {...}}`. Changes apply
on configuration reload. Omitted values keep the defaults shown below.

| Key                 | Values                                  | Default              | Effect                                                   |
| ------------------- | --------------------------------------- | -------------------- | -------------------------------------------------------- |
| `enable_animations` | Boolean                                 | `true`               | Enables compositor animations.                           |
| `locate_pointer`    | Boolean                                 | `false`              | Enables the pointer-location effect.                     |
| `visual_bell`       | Boolean                                 | `false`              | Flashes the screen when an app requests the visual bell. |
| `audible_bell`      | Boolean                                 | `true`               | Plays a sound when an app requests the audible bell.     |
| `visual_bell_type`  | `"fullscreen-flash"` or `"frame-flash"` | `"fullscreen-flash"` | Flash the full screen or the focused window.             |

For example, enable the pointer locator and use a focused-window flash:

```lua
gnoblin.configure {
    compositor = {
        locate_pointer = true,
        visual_bell = true,
        visual_bell_type = "frame-flash",
    },
}
```

Guide: [session settings](/guides/session_settings).
