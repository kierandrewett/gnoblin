# gnoblin.configure.shell

Configure this part of `gnoblin.configure` with the `shell` key.

Inside `gnoblin.configure {shell = {...}}`.

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

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[window menu](/guides/window_menu).
