# gnoblin.configure.shell

Put these fields inside `gnoblin.configure {shell = {...}}`. Configuration
changes apply on reload. Rows marked as persistent are stored in GSettings.

| Key                     | Accepted values                                                          | Default                                   | Effect                                                               |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------- | -------------------------------------------------------------------- |
| `minimize_animation`    | `"zoom"`, `"fade"`, `"none"`, `"gnome"`, or an event map                 | `"zoom"`                                  | Selects the minimize and restore animation.                          |
| `minimize_duration`     | Integer milliseconds, 0–5000                                             | `200`                                     | Sets the minimize and restore duration.                              |
| `minimize_target`       | `{x, y}` in desktop logical pixels                                       | Dock target, then bottom centre           | Sets where minimized windows animate toward.                         |
| `layer_animation`       | `"slide"`, `"fade"`, `"none"`, or an event map                           | `"slide"`                                 | Selects layer-surface open and close animations.                     |
| `layer_duration`        | Integer milliseconds, 0–5000                                             | `220`                                     | Sets the layer-surface animation duration.                           |
| `layer_easing`          | `"linear"`, `"ease-out-quad"`, `"ease-out-cubic"`, `"ease-in-out-cubic"` | `"ease-out-cubic"`                        | Controls the layer-surface animation curve.                          |
| `window_menu`           | Command argument list                                                    | Empty                                     | Runs the command when a window menu is requested.                    |
| `window_switcher`       | Boolean                                                                  | `false`                                   | Enables GNOME's app, window, and group switchers.                    |
| `notifications`         | Boolean                                                                  | Initially disabled; persists in GSettings | Enables Gnoblin's notification service.                              |
| `input_source_switcher` | Boolean                                                                  | Initially disabled; persists in GSettings | Enables GNOME's keyboard-layout switcher.                            |
| `wallpaper`             | Boolean                                                                  | `true`; persists in GSettings             | Shows GNOME backgrounds; uses the configured color without an image. |

## Animation choices

Animation settings also accept a map of event names to custom animation
names. See the [animation guide](/guides/animations) for events and examples.
`minimize_target` uses logical desktop pixels; if no dock target is available,
Gnoblin uses the bottom-centre of the screen.

## Feature preferences

Feature enablement preferences persist in GSettings. The other settings apply
on configuration reload.

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[wallpapers](/guides/wallpapers), [window menu](/guides/window_menu).
