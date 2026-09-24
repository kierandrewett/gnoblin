# gnoblin.configure.shell

Configure this part of `gnoblin.configure` with the `shell` key.

Inside `gnoblin.configure {shell = {...}}`.

| Key                     | Values                                                                   | Default                                   |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------- |
| `minimize_animation`    | Built-in animation name or `{minimize = name, restore = name}`           | `"zoom"`                                  |
| `minimize_duration`     | 0–5000 ms                                                                | `200`                                     |
| `minimize_target`       | `{x, y}` in desktop logical pixels                                       | Dock target, then bottom centre           |
| `layer_animation`       | Built-in animation name or event map for `layer-open` and `layer-close`  | `"slide"`                                 |
| `layer_duration`        | 0–5000 ms                                                                | `220`                                     |
| `layer_easing`          | `"linear"`, `"ease-out-quad"`, `"ease-out-cubic"`, `"ease-in-out-cubic"` | `"ease-out-cubic"`                        |
| `window_menu`           | Command argument list                                                    | Empty                                     |
| `window_switcher`       | Boolean                                                                  | `false`                                   |
| `notifications`         | Boolean                                                                  | Initially disabled; persists in GSettings |
| `input_source_switcher` | Boolean                                                                  | Initially disabled; persists in GSettings |

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[window menu](/guides/window_menu).

Animation names include `slide`, `fade`, `zoom`, `none`, and the `gnome-*` and
`gnoblin-*` presets. Use event maps when entry and exit need different
animations. Declare custom animations with [`gnoblin.animation`](/config/animation);
the [animation guide](/guides/animations) lists events and presets. The
`layer_animation` map accepts `in`/`out` as aliases for `layer-open` and
`layer-close`.

`minimize_target` is a `{x, y}` position in logical desktop pixels. Coordinates
must be integers whose absolute value is at most 1,000,000. If no dock target
is available, Gnoblin uses the bottom-centre of the screen. `window_menu` is a
command argv list (up to 32 arguments); an empty list disables it. Arguments
are passed directly, without shell expansion. `window_switcher` controls
GNOME's app, window and group switchers. Leave it disabled when another shell
component provides those interfaces.
