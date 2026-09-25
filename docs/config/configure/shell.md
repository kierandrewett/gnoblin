# gnoblin.configure.shell

Put these fields inside `gnoblin.configure {shell = {...}}`. Configuration
changes apply on reload. Rows marked as persistent are stored in GSettings.

| Key                     | Accepted values                                                          | Default                                   | Effect                                                                  |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------- | ----------------------------------------------------------------------- |
| `minimize_animation`    | `"zoom"`, `"fade"`, `"none"`, `"gnome"`, or an event map                 | `"zoom"`                                  | Selects the minimize and restore animation.                             |
| `minimize_duration`     | Integer milliseconds, 0–5000                                             | `200`                                     | Sets the minimize and restore duration.                                 |
| `minimize_target`       | `[x, y]`, two integers from −1,000,000 to 1,000,000                      | Dock target, then bottom centre           | Sets where minimized windows animate toward, in logical desktop pixels. |
| `layer_animation`       | `"slide"`, `"fade"`, `"none"`, `"gnome"`, or an event map                | `"slide"`                                 | Selects layer-surface open and close animations.                        |
| `layer_duration`        | Integer milliseconds, 0–5000                                             | `220`                                     | Sets the layer-surface animation duration.                              |
| `layer_easing`          | `"linear"`, `"ease-out-quad"`, `"ease-out-cubic"`, `"ease-in-out-cubic"` | `"ease-out-cubic"`                        | Controls the layer-surface animation curve.                             |
| `window_menu`           | Up to 32 command strings; if present, the first must be non-empty        | Empty                                     | Runs the command when a window menu is requested.                       |
| `window_switcher`       | Boolean                                                                  | `false`                                   | Enables GNOME's app, window, and group switchers.                       |
| `notifications`         | Boolean                                                                  | Initially disabled; persists in GSettings | Enables Gnoblin's notification service.                                 |
| `input_source_switcher` | Boolean                                                                  | Initially disabled; persists in GSettings | Enables GNOME's keyboard-layout switcher.                               |
| `wallpaper`             | Boolean                                                                  | `true`; persists in GSettings             | Shows GNOME backgrounds; uses the configured color without an image.    |

## Animation choices

String values select a built-in preset that supports both transitions. Use an
event map to select a custom animation by its registered name; custom
animations apply to one event. See the [animation guide](/guides/animations)
for event names, presets and examples.

`minimize_target = {320, 400}` sets an explicit target in logical desktop
pixels. If no dock target is available, Gnoblin uses the bottom-centre of the
screen.

Use `window_menu = gnoblin.array {}` to clear a previously configured command.

![GNOME Settings open beneath Waybar in a clean Gnoblin session](../../images/gnoblin-waybar-settings.png)

_Waybar is a separate desktop client; Gnoblin's shell settings control compositor behavior._

## Feature preferences

Feature enablement preferences persist in GSettings. The other settings apply
on configuration reload.

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[wallpapers](/guides/wallpapers), [window menu](/guides/window_menu).

## Type definition

This is schema pseudocode in Lua table form. `?` marks optional settings;
`|` separates accepted alternatives.

```lua
gnoblin.configure {
    shell = {
        minimize_animation = string | {minimize = string?, restore = string?}?,
        minimize_duration = integer?, -- 0–5000 ms
        minimize_target = {integer, integer}?, -- each −1,000,000 to 1,000,000
        layer_animation = string | {
            ["layer-open"] = string?, ["layer-close"] = string?,
            ["in"] = string?, out = string?,
        }?,
        layer_duration = integer?, -- 0–5000 ms
        layer_easing = "linear" | "ease-out-quad" | "ease-out-cubic" | "ease-in-out-cubic"?,
        window_menu = {string, ...} | {}?, -- at most 32; first argument nonempty
        window_switcher = boolean?,
        notifications = boolean?,
        input_source_switcher = boolean?,
        wallpaper = boolean?,
    },
}
```
