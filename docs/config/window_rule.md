# gnoblin.window_rule

Add an ordered window or layer-surface rule. All fields in `match` must
match. Later matching rules override only the fields they provide.

See the [window rules guide](/guides/window_rules) for usage patterns.

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.App$"},
    opacity = 0.9,
}
```

## Match fields

| Field              | Values                                                         |
| ------------------ | -------------------------------------------------------------- |
| `type`             | `"window"` or `"layer"`                                        |
| `focused`          | Boolean                                                        |
| `app_id`           | Lua pattern against GTK app ID, falling back to WM class       |
| `title`            | Lua pattern against the window title                           |
| `layer`            | Lua pattern against the layer-shell namespace                  |
| `workspace_id`     | Exact workspace ID; declared or explicitly assigned at runtime |
| `workspace_number` | Current one-based workspace position, 1–1024                   |

All supplied match fields must match. Text patterns use Lua 5.4
`string.find` syntax. They are case-sensitive and search anywhere by default.
Use `^` and `$` to match the whole value.

In Lua patterns, `.` matches any character and `%.` matches a literal dot.
For example, `^org%.example%.App$` matches the complete ID
`org.example.App`. JavaScript and PCRE regex syntax is not supported.
`workspace_id` matches an exact, case-sensitive ID.

See the [window rules guide](/guides/window_rules#find-the-values) for examples
and ways to find a live app ID, title, or layer namespace.

Declare persistent workspace IDs in `gnoblin.configure.workspaces`. A matcher
can also name an explicitly assigned runtime ID. Generated `@session-N` IDs
are temporary and should not be saved in configuration. A `{id = ...}`
placement target must exist when a matching window opens; if it does not,
Gnoblin leaves the window in place and logs a warning.

## Rule fields {#rule-fields}

| Field                 | Values                                                                     |
| --------------------- | -------------------------------------------------------------------------- |
| `blur`                | Integer 0–100; 0 disables blur                                             |
| `opacity`             | Number 0–1; affects content and text                                       |
| `blur_ignore_shadows` | Boolean; default `false`                                                   |
| `corners`             | Corner fields below                                                        |
| `shader`              | GLSL file path; `""` clears it                                             |
| `shader_uniforms`     | Up to 64 uniform names mapped to finite numeric values                     |
| `animation`           | Built-in or registered animation name, or an event map                     |
| `workspace`           | `{id = "code"}` or `{number = 1..1024}`; place a new window on a workspace |
| `frame`               | Frame fields below                                                         |

Uniform names must be valid GLSL-style identifiers; the `gnoblin_` prefix is
reserved. Values must fit in a finite 32-bit float.

Workspace match fields select a window's current workspace and update when
the window changes workspaces.

The `workspace` placement effect runs once when a normal window is created. It
is not reapplied when its title or focus
changes or when the configuration reloads. Transient and modal windows stay
with their parent. See the [workspaces section of the window rules guide](/guides/window_rules#workspaces).

## Corners

| Field                           | Default          | Values                                                                    |
| ------------------------------- | ---------------- | ------------------------------------------------------------------------- |
| `radius`                        | `0`              | 0–200 logical pixels                                                      |
| `smoothing`                     | `0`              | 0–1; circular to a squarer curve                                          |
| `mode`                          | `"auto"`         | `auto`, `force`, `off`; `off` disables corner, border, and shadow effects |
| `padding`                       | `{0, 0, 0, 0}`   | Top/right/bottom/left inset, −128–128 logical pixels                      |
| `keep_maximized`                | `true`           | Keep rounding when maximised                                              |
| `keep_fullscreen`, `keep_tiled` | `false`          | Keep rounding in those states                                             |
| `skip_libadwaita`               | `true`           | Preserve libadwaita corners in auto mode                                  |
| `skip_libhandy`                 | `false`          | Skip libhandy windows in auto mode                                        |
| `remove_csd`                    | `false`          | [Detect and replace client-drawn rounded corners](#remove-csd)            |
| `border_width`, `border_color`  | `0`, `#808080ff` | Single native outline; width −40–40 logical pixels                        |
| `shadow`                        | `false`          | `true` for the default shadow, one table, or a list of 1–4 layers         |
| `keep_shadow`                   | `false`          | Keep replacement shadows in maximised/fullscreen/tiled states             |

## Shadows

The single native outline is configured by `corners.border_width` and
`corners.border_color` above. Positive widths draw inward; negative widths
draw outward where the client's existing buffer has room. The `corners.shadow`
field accepts `false`, `true` for the default, one table, or a list of 1–4
shadow layers.

| Field                                | Values / default                                           |
| ------------------------------------ | ---------------------------------------------------------- |
| `corners.shadow.{x, y, spread}`      | Defaults `0`, `4`, `4`; −100–100                           |
| `corners.shadow.blur`                | Default `28`; 0–100                                        |
| `corners.shadow.opacity`             | Default `0.6`; 0–1                                         |
| `corners.shadow.color`               | Default `#000000ff`; `#RRGGBB` or `#RRGGBBAA`              |
| `corners.shadow_animation.animation` | Animation name that supports the `shadow-change` event     |
| `corners.shadow_animation.duration`  | 0–2000 ms; default 0                                       |
| `corners.shadow_animation.easing`    | Same easing values as `gnoblin.configure` shell animations |

## Animation fields

Choose a built-in or registered animation name. To select names by event, use
a table:

- `in` and `out` alias `layer-open` and `layer-close`.
- `duration` and `easing` (or `ease`) override the selected animation.

See the [animation guide](/guides/animations#register-a-custom-animation) for
supported events and presets.

| Field                              | Values                                                     |
| ---------------------------------- | ---------------------------------------------------------- |
| `animation["in"]`, `animation.out` | Built-in or registered animation name                      |
| `animation.duration`               | 0–5000 ms                                                  |
| `animation.easing`                 | Same easing values as `gnoblin.configure` shell animations |

Omitted values inherit shell settings. `in` needs brackets because it is a Lua keyword.

## Frame fields

| Field                 | Default                             | Values                                               |
| --------------------- | ----------------------------------- | ---------------------------------------------------- |
| `mode`                | `"off"`                             | `"off"`, `"auto"`, `"prefer-server"`, `"replace"`    |
| `extents`             | `{32, 1, 1, 1}`                     | Top/right/bottom/left frame size; integers 0–256     |
| `crop`                | `{0, 0, 0, 0}`                      | Removed client margins; same order and range         |
| `renderer`            | `"native"`                          | Registered service name                              |
| `style`               | `"default"`                         | Style understood by that renderer                    |
| `background`          | `"#242424"`                         | Active background colour                             |
| `foreground`          | `"#eeeeee"`                         | Foreground colour                                    |
| `inactive_background` | `"#303030"`                         | Unfocused background colour                          |
| `button_layout`       | `{"minimize", "maximize", "close"}` | Ordered buttons, without duplicates; `{}` hides them |

`auto` supplies SSD only for explicit client requests. `replace` crops client
pixels and adds a frame.

See the [window frames guide](/guides/window_frames) for frame modes and
extents. Register renderers with
[`gnoblin.configure`](/config/configure#window-management).

### `remove_csd`

Enable `remove_csd` when an application draws its own rounded corners and you
want Gnoblin's configured shape to control the result. Gnoblin inspects the
window's rendered pixels to detect the corner cutouts, then fills those gaps
from the app's nearby background before applying the configured corners.

Gnoblin adapts to each window's actual content instead of assuming a
particular toolkit, corner radius, or background colour.

It also handles the narrow antialiased edge around the detected curve while
preserving opaque content.

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.App$"},
    corners = {
        radius = 12,
        mode = "force",
        remove_csd = true,
    },
}
```

This is opt-in because detecting and sampling the window image has a cost. Use
it on rules for apps whose own rounded corners conflict with Gnoblin's shape;
leave it off for other windows. The option changes corner rendering only. It
does not remove client-side titlebars or other decorations.

If the app's corner background varies sharply near the edge, Gnoblin may not
find a safe fill, so the original corner can remain visible.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`|` separates accepted alternatives. Every supplied `match` field must match.

```lua
gnoblin.window_rule {
    match = {
        type = "window" | "layer"?,
        focused = boolean?,
        app_id = string?, -- Lua 5.4 pattern
        title = string?, -- Lua 5.4 pattern
        layer = string?, -- Lua 5.4 pattern
        workspace_id = string?,
        workspace_number = integer?, -- 1–1024
    },
    blur = integer?, -- 0–100
    opacity = number?, -- 0–1
    blur_ignore_shadows = boolean?,
    shader = string?,
    shader_uniforms = {string = number}?,
    animation = string | {
        ["in"] = string?, out = string?,
        open = string?, close = string?,
        dialog_open = string?, dialog_close = string?,
        layer_open = string?, layer_close = string?,
        minimize = string?, restore = string?, workspace_switch = string?,
        shadow_change = string?,
        layer_companion_close = string?, resize = string?,
        duration = integer?, -- 0–5000 ms
        easing = string?,
        ease = string?, -- alias for easing
    }?,
    workspace = {id = string} | {number = integer}?,
    corners = {
        radius = number?, -- 0–200
        smoothing = number?, -- 0–1
        mode = "auto" | "force" | "off"?,
        padding = {number, number, number, number}?,
        border_width = number?, -- -40 to 40
        border_color = string?, -- #RRGGBB or #RRGGBBAA
        keep_maximized = boolean?,
        keep_fullscreen = boolean?,
        keep_tiled = boolean?,
        skip_libadwaita = boolean?,
        skip_libhandy = boolean?,
        remove_csd = boolean?,
        shadow = false | Shadow | {Shadow, ...}?,
        keep_shadow = boolean?,
        shadow_animation = {
        animation = string?,
            duration = integer?, -- 0–2000 ms
            easing = "linear" | "ease-out-cubic" | "ease-out-quad" | "ease-in-out-cubic"?,
        }?,
    }?,
    frame = {
        mode = "off" | "auto" | "prefer-server" | "replace"?,
        extents = {integer, integer, integer, integer}?, -- 0–256 each
        crop = {integer, integer, integer, integer}?, -- 0–256 each
        renderer = string?,
        style = string?,
        background = string?,
        foreground = string?,
        inactive_background = string?,
        button_layout = {"minimize" | "maximize" | "close", ...} | {}?,
    }?,
}

-- Shadow = {x = number?, y = number?, blur = number?, spread = number?,
--           opacity = number?, color = string?}
```
