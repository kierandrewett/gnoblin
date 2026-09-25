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
match. `workspace_id` is an exact, case-sensitive ID. For examples and
instructions to find the live app ID, title, or layer namespace, see the
[window rules guide](/guides/window_rules#find-the-values).

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
| `animation`           | `"slide"`, `"fade"`, `"none"`, or a per-phase table                  |
| `workspace`           | `{id = "code"}` or `{number = 2}`; place a new window on a workspace |
| `frame`               | Frame fields below                                                   |

Workspace match fields select a window's current workspace and update when
the window changes workspaces. The `workspace` placement effect runs once
when a normal window is created. It is not reapplied when its title or focus
changes or when the configuration reloads. Transient and modal windows stay
with their parent. See the [workspaces section of the window rules guide](/guides/window_rules#workspaces).

## Corners

| Field                           | Default        | Values                                                        |
| ------------------------------- | -------------- | ------------------------------------------------------------- |
| `radius`                        | `0`            | 0–200 logical pixels                                          |
| `smoothing`                     | `0`            | 0–1; circular to a squarer curve                              |
| `mode`                          | `"auto"`       | `auto`, `force`, `off`                                        |
| `padding`                       | `{0, 0, 0, 0}` | Top/right/bottom/left inset, −128–128 logical pixels          |
| `keep_maximized`                | `true`         | Keep rounding when maximised                                  |
| `keep_fullscreen`, `keep_tiled` | `false`        | Keep rounding in those states                                 |
| `skip_libadwaita`               | `true`         | Preserve libadwaita corners in auto mode                      |
| `skip_libhandy`                 | `false`        | Skip libhandy windows in auto mode                            |
| `remove_csd`                    | `false`        | Reconstruct supported client corner gaps                      |
| `shadow`                        | `false`        | A shadow table or 1–4 shadow layers                           |
| `keep_shadow`                   | `false`        | Keep replacement shadows in maximised/fullscreen/tiled states |

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

| Field                              | Values                                                     |
| ---------------------------------- | ---------------------------------------------------------- |
| `animation["in"]`, `animation.out` | `"slide"`, `"fade"`, `"none"`                              |
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
