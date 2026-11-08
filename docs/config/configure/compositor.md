# gnoblin.configure.compositor

Configure this part of `gnoblin.configure` with the `compositor` key.

Put these fields inside `gnoblin.configure {compositor = {...}}`. Only
`visual_bell` and `visual_bell_type` apply on configuration reload. The other
fields are read when the session starts, so restart the session after changing
them. Omitted values keep the defaults shown below.

Animations run only when selected through animation declarations or window
rules. See the [animation guide](/guides/animations).

| Key                  | Values                                          | Default              | Effect                                                            |
| -------------------- | ----------------------------------------------- | -------------------- | ----------------------------------------------------------------- |
| `locate_pointer`     | Boolean                                         | `false`              | Sends `gnoblin.pointer.locate-requested` when the key is pressed. |
| `locate_pointer_key` | Mutter key name, `"disabled"`, or `""`          | `"Control_L"`        | Key that triggers the pointer locator.                            |
| `visual_bell`        | Boolean                                         | `false`              | Flashes the screen when an app requests the visual bell.          |
| `audible_bell`       | Boolean                                         | `true`               | Plays a sound when an app requests the audible bell.              |
| `blur`               | `"auto"`, `"on"`, or `"off"`                    | `"auto"`             | Controls compositor background blur.                              |
| `renderer`           | `"auto"`, `"vulkan"`, `"gles"`, or `"software"` | `"auto"`             | Chooses the renderer that draws frames.                           |
| `debug`              | Boolean                                         | `false`              | Shows a mostly opaque FPS, frame-time, CPU and memory overlay.    |
| `visual_bell_type`   | `"fullscreen-flash"` or `"frame-flash"`         | `"fullscreen-flash"` | Flash the full screen or the focused window.                      |

`blur = "auto"` enables blur when Mutter reports hardware accelerated rendering and disables it otherwise. `"on"` always enables blur; `"off"` disables blur for panels, window rules, and shell-requested background effects. The setting is parsed, but the session does not apply it, so changing it has no effect.

Gnoblin draws nothing for the pointer locator. With `locate_pointer = true`,
pressing `locate_pointer_key` sends the `gnoblin.pointer.locate-requested`
event with the pointer position (`x`, `y`) and its monitor. A shell or a Lua
callback decides how to show it. See [Lua events](/events/).

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

## Choosing the renderer

`renderer = "auto"` keeps the renderer the selected backend draws with today: software
for the headless and nested backends, and GLES for the DRM backend. The other values
choose one explicitly:

- `"vulkan"` draws with Vulkan. It needs a Vulkan loader and a device, and a software
  driver such as lavapipe is accepted.
- `"gles"` draws with OpenGL ES on a surfaceless context.
- `"software"` draws on the CPU and needs no GPU.

A named renderer is never replaced by another. A session that cannot create it is
refused with a message that names the setting, for example
`compositor.renderer = vulkan: no Vulkan loader`. The nested backend draws in software
only, so it refuses `"vulkan"` and `"gles"` the same way. The renderer is chosen when
the session starts, so a change needs a restart and is not applied on reload.

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
        debug = boolean?,
        blur = "auto" | "on" | "off"?,
        renderer = "auto" | "vulkan" | "gles" | "software"?,
        visual_bell_type = "fullscreen-flash" | "frame-flash"?,
    },
}
```
