# Titlebars and window frames

[Configuration reference](configuration-reference.md)

**CSD:** the app draws its titlebar. **SSD:** Gnoblin draws a frame around it.

SSD is off by default. Your desktop shell's config may enable it.
Choosing a renderer only changes how an enabled frame looks.

## Choose a mode

| Mode            | Behavior                                                    |
| --------------- | ----------------------------------------------------------- |
| `off`           | Request CSD; disable Gnoblin's frame and crop               |
| `auto`          | Supply SSD only for an explicit client request              |
| `prefer-server` | Choose SSD when the client has an xdg-decoration object     |
| `replace`       | Keep protocol CSD mode, crop configured margins and add SSD |

An unset client preference does not prove that the app draws a titlebar.
Check the visible result when changing policy.

## Enable negotiated SSD

Append after your config includes:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    frame = {
        mode = "auto",
        renderer = "native",
        extents = {36, 0, 0, 0},
    },
}
```

Save and run `gnoblinctl config reload`. Negotiation also needs a client commit.
This policy applies to clients generally; it does not need an app-name list.

## Frame options

All fields below go inside `frame`. Later matching rules override individual
fields. Sizes are logical pixels.

| Field                 | Default                             | Values                                               |
| --------------------- | ----------------------------------- | ---------------------------------------------------- |
| `mode`                | `"off"`                             | Modes listed above                                   |
| `extents`             | `{32, 1, 1, 1}`                     | Top/right/bottom/left frame size; integers 0–256     |
| `crop`                | `{0, 0, 0, 0}`                      | Removed client margins; same order and range         |
| `renderer`            | `"native"`                          | Registered service name                              |
| `style`               | `"default"`                         | Style understood by that renderer                    |
| `background`          | `"#242424"`                         | Active background colour                             |
| `foreground`          | `"#eeeeee"`                         | Foreground colour                                    |
| `inactive_background` | `"#303030"`                         | Unfocused background colour                          |
| `button_layout`       | `{"minimize", "maximize", "close"}` | Ordered buttons, without duplicates; `{}` hides them |

Colours accept `#RRGGBB` or `#RRGGBBAA`.
Renderer and style names accept 1–64 letters, digits, `_` or `-`.

## Cropping client decorations

Crop removes pixels **and their input targets**. Measure the client margins
before using it. Header bars can contain tabs, search and other app controls.

With `prefer-server`, explicit crop provides a fallback for CSD-only clients;
negotiated SSD clients are not cropped. With `replace`, zero extents give a
crop-only window. Fullscreen temporarily removes crop and frame extents.

## Custom renderers

Register a service at the config root, then select its name in a frame rule:

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"/absolute/path/gnoblin-frame-cairo"},
    },
}
```

Use an absolute executable path and separate arguments. `native` is reserved.
[Bingux](bring-your-own-shell.md) supplies its own styled renderer.

Config reload restarts external renderers, including rebuilt executables at
unchanged paths. The native frame keeps controls available during replacement
or failure. A compositor upgrade still needs logout and login.

## Limits

Frames support Wayland xdg-toplevels, not Xwayland, popups or layer surfaces.
Mixed-scale moves and popup-heavy cropped apps need more coverage.

For renderer implementation and tests, see
[renderer architecture](window-frame-renderers.md) and the [author guide](frame-renderer-api.md).
