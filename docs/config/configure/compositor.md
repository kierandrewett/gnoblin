# gnoblin.configure.compositor

Configure this part of `gnoblin.configure` with the `compositor` key.

Put these fields inside `gnoblin.configure {compositor = {...}}`. Changes apply
on configuration reload. Omitted values keep the defaults shown below.

Animations run only when selected through animation declarations or window
rules. There is no global animation switch. See the
[animation guide](/guides/animations).

| Key                  | Values                                  | Default              | Effect                                                            |
| -------------------- | --------------------------------------- | -------------------- | ----------------------------------------------------------------- |
| `locate_pointer`     | Boolean                                 | `false`              | Sends `gnoblin.pointer.locate-requested` when the key is pressed. |
| `locate_pointer_key` | Mutter key name, `"disabled"`, or `""`  | `"Control_L"`        | Key that triggers the pointer locator.                            |
| `visual_bell`        | Boolean                                 | `false`              | Flashes the screen when an app requests the visual bell.          |
| `audible_bell`       | Boolean                                 | `true`               | Plays a sound when an app requests the audible bell.              |
| `blur`               | `"auto"`, `"on"`, or `"off"`            | `"auto"`             | Controls compositor background blur.                              |
| `debug`              | Boolean                                 | `false`              | Shows a mostly opaque FPS, frame-time, CPU and memory overlay.    |
| `visual_bell_type`   | `"fullscreen-flash"` or `"frame-flash"` | `"fullscreen-flash"` | Flash the full screen or the focused window.                      |
| `background`         | Table: `color`, `image`                 | black                | What the compositor paints under every surface.                   |

### `background`

The compositor paints `background` on every monitor beneath all windows and
layer surfaces. It is the first thing on screen after a modeset, before any
wallpaper client has mapped, and it stays visible wherever nothing covers it.

Set it to the same picture your shell shows. The start of a session, the return
from the login screen and a wallpaper client restart then no longer flash black.

| Field   | Values                       | Effect                                                        |
| ------- | ---------------------------- | ------------------------------------------------------------- |
| `color` | `"#rrggbb"` or `"#rrggbbaa"` | Fill colour. Also shows through a missing or failed `image`.  |
| `image` | Absolute path                | Picture scaled to fill each monitor, keeping its aspect ratio |

```lua
gnoblin.configure {
    compositor = {
        background = {
            color = "#1b2a4a",
            image = "/usr/share/backgrounds/valley.jpg",
        },
    },
}
```

The image is decoded once per path. A file that does not exist or cannot be
decoded logs a warning and leaves the colour. Changing either field on reload
repaints at once. A shell that manages wallpapers can regenerate a drop-in
file with the current picture; see the [wallpaper guide](/guides/wallpapers).

The same picture covers the gap between two compositors. When a compositor
exits, the kernel puts its own console framebuffer back on the display. It
stays there until the next compositor shows its first frame, so a handover such
as login screen to user session shows black or console text.

Gnoblin paints `background` into that framebuffer once its first frame is up
and again when `background` changes. The picture then stays on screen across
the handover.

Gnoblin writes only a linear XRGB8888 or ARGB8888 framebuffer that the driver
can map. It leaves any other framebuffer alone, and it does nothing in a nested
session.

The kernel's text console draws over that framebuffer when a session ends. A
display that shows the text console still flashes black between two
compositors, whatever Gnoblin painted.

To keep the picture, unbind the text console from the display at boot, before
the first session starts. The display then has no text console, which suits a
login screen or a kiosk. Run this as root:

```sh
for console in /sys/class/vtconsole/vtcon*; do
    grep -q 'frame buffer' "$console/name" && echo 0 > "$console/bind"
done
```

`blur = "auto"` enables blur when Mutter reports hardware accelerated rendering and disables it otherwise. `"on"` always enables blur; `"off"` disables blur for panels, window rules, and shell-requested background effects. The setting applies on configuration reload.

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
        visual_bell_type = "fullscreen-flash" | "frame-flash"?,
        background = {
            color = string?, -- "#rrggbb" or "#rrggbbaa"
            image = string?, -- absolute path
        }?,
    },
}
```
