# gnoblin.configure.shell

Configure this part of `gnoblin.configure` with the `shell` key.

Put these fields inside `gnoblin.configure {shell = {...}}`. Changes apply on
configuration reload unless the row says the preference is persisted.

| Key                     | Values                                                                   | Default                                   |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------- |
| `minimize_animation`    | `"zoom"`, `"fade"`, `"none"`, `"gnome"`                                  | `"zoom"`                                  |
| `minimize_duration`     | 0–5000 ms                                                                | `200`                                     |
| `minimize_target`       | `{x, y}` in desktop logical pixels                                       | Dock target, then bottom centre           |
| `layer_animation`       | `"slide"`, `"fade"`, `"none"`                                            | `"slide"`                                 |
| `layer_duration`        | 0–5000 ms                                                                | `220`                                     |
| `layer_easing`          | `"linear"`, `"ease-out-quad"`, `"ease-out-cubic"`, `"ease-in-out-cubic"` | `"ease-out-cubic"`                        |
| `window_menu`           | Command argument list                                                    | Empty                                     |
| `window_switcher`       | Boolean                                                                  | `false`                                   |
| `notifications`         | Boolean                                                                  | Initially disabled; persists in GSettings |
| `input_source_switcher` | Boolean                                                                  | Initially disabled; persists in GSettings |
| `wallpaper`             | Boolean                                                                  | `true`; persists in GSettings             |

`minimize_animation` and `layer_animation` select a built-in name or a map of
event names to custom animation names. Their accepted values and motion are
described in the [animation guide](/guides/animations). `minimize_target` is a
two-number `{x, y}` position in logical desktop pixels; omitted coordinates use
the dock target or bottom-center fallback.

Notification and input-source preferences persist in GSettings. With no usable
picture configured, GNOME displays its configured background color.

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[window menu](/guides/window_menu).
