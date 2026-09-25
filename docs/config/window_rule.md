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

| Field              | Values                                                                     |
| ------------------ | -------------------------------------------------------------------------- |
| `type`             | `"window"` or `"layer"`                                                    |
| `focused`          | Boolean                                                                    |
| `app_id`           | JavaScript regular expression against GTK app ID, falling back to WM class |
| `title`            | JavaScript regular expression against the window title                     |
| `layer`            | Layer-shell namespace matcher                                              |
| `workspace_id`     | Exact stable workspace ID declared in `workspace_ids`                      |
| `workspace_number` | Current one-based workspace position                                       |

All supplied match fields must match. `app_id`, `title`, and `layer` are
case-sensitive JavaScript regular expressions; use `^` and `$` for an exact
match. `workspace_id` is an exact, case-sensitive ID.

See the [window rules guide](/guides/window_rules#find-the-values) for examples
and instructions to find the live app ID, title, or layer namespace.

Declare IDs in `gnoblin.configure.window_management.workspace_ids`. A
`workspace_id` matcher or `{id = ...}` placement target that is not declared
makes the configuration invalid. If a declared placement target has no
currently available workspace when a matching window opens, Gnoblin leaves the
window in place and logs a warning.

## Rule fields {#rule-fields}

| Field                 | Values                                                               |
| --------------------- | -------------------------------------------------------------------- |
| `blur`                | Integer 0–100; 0 disables blur                                       |
| `opacity`             | Number 0–1; affects content and text                                 |
| `blur_ignore_shadows` | Boolean; default `false`                                             |
| `corners`             | Corner fields below                                                  |
| `borders`             | Border fields below                                                  |
| `shader`              | GLSL file path; `""` clears it                                       |
| `shader_uniforms`     | Map of literal uniform names to numeric values                       |
| `animation`           | Built-in or registered animation name, or an event map               |
| `workspace`           | `{id = "code"}` or `{number = 2}`; place a new window on a workspace |
| `frame`               | Frame fields below                                                   |

Workspace match fields select a window's current workspace and update when
the window changes workspaces.

The `workspace` placement effect runs once when a normal window is created. It
is not reapplied when its title or focus
changes or when the configuration reloads. Transient and modal windows stay
with their parent. See the [workspaces section of the window rules guide](/guides/window_rules#workspaces).

## Corners

| Field                           | Default          | Values                                                         |
| ------------------------------- | ---------------- | -------------------------------------------------------------- |
| `radius`                        | `0`              | 0–200 logical pixels                                           |
| `smoothing`                     | `0`              | 0–1; circular to a squarer curve                               |
| `mode`                          | `"auto"`         | `auto`, `force`, `off`                                         |
| `padding`                       | `{0, 0, 0, 0}`   | Top/right/bottom/left inset, −128–128 logical pixels           |
| `keep_maximized`                | `true`           | Keep rounding when maximised                                   |
| `keep_fullscreen`, `keep_tiled` | `false`          | Keep rounding in those states                                  |
| `skip_libadwaita`               | `true`           | Preserve libadwaita corners in auto mode                       |
| `skip_libhandy`                 | `false`          | Skip libhandy windows in auto mode                             |
| `remove_csd`                    | `false`          | [Detect and replace client-drawn rounded corners](#remove-csd) |
| `border_width`, `border_color`  | `0`, `#808080ff` | Legacy border width (−40–40) and colour                        |
| `shadow`                        | `false`          | A shadow table or 1–4 shadow layers                            |
| `keep_shadow`                   | `false`          | Keep replacement shadows in maximised/fullscreen/tiled states  |

## Borders and shadows

| Field                                                                     | Values / default                                                              |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `borders.inner_width`, `borders.outer_width`                              | 0–40 logical pixels; default 0                                                |
| `borders.inner_color`, `borders.outer_color`                              | `#RRGGBB` or `#RRGGBBAA`                                                      |
| `borders.radius`, `borders.smoothing`, `borders.padding`                  | Inherit corners; same ranges                                                  |
| `borders.keep_maximized`, `borders.keep_fullscreen`, `borders.keep_tiled` | Boolean; `false` hides borders in that state                                  |
| `corners.shadow`                                                          | Table or 1–4 layer tables with `x`, `y`, `blur`, `spread`, `opacity`, `color` |
| `corners.shadow_animation.duration`                                       | 0–2000 ms; default 0                                                          |
| `corners.shadow_animation.easing`                                         | Same easing values as `gnoblin.configure` shell animations                    |

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
        app_id = string?, -- regular expression
        title = string?, -- regular expression
        layer = string?, -- regular expression
        workspace_id = string?,
        workspace_number = integer?, -- 1–36
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
        console_open = string?, console_close = string?, shadow_change = string?,
        layer_companion_close = string?, resize = string?,
        tile_preview_open = string?, tile_preview_close = string?,
        dialog_dim = string?, dialog_undim = string?,
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
    borders = {
        inner_width = number?, outer_width = number?,
        inner_color = string?, outer_color = string?,
        radius = number?, smoothing = number?,
        padding = {number, number, number, number}?,
        keep_maximized = boolean?, keep_fullscreen = boolean?, keep_tiled = boolean?,
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
