# Animations

Register an animation in `~/.config/gnoblin/init.lua`, then select it in a
shell setting or window rule. Each registration describes one event. Without
an explicit selection, the first registration for that event is the default.

```lua
gnoblin.animation {
    name = "soft-open",
    event = "open",
    duration = 280,
    ease = "ease-out-cubic",
    origin = "center",
    from = {y = 18, scale = 0.94, opacity = 0},
    to = {y = 0, scale = 1, opacity = 1},
}
```

Names are unique; use letters, digits, `_` or `-` (up to 80 characters).
Disable an imported registration with `gnoblin.animation {name = "soft-open", enable = false}`.

## Events

Choose the event that describes **when** the animation runs:

| Event                                     | When it runs                                     |
| ----------------------------------------- | ------------------------------------------------ |
| `minimize`                                | When a window minimizes                          |
| `restore`                                 | When a window restores                           |
| `open`, `close`                           | When a window opens or closes                    |
| `dialog-open`, `dialog-close`             | When a dialog opens or closes                    |
| `dialog-dim`, `dialog-undim`              | When a dialog dims or returns to normal          |
| `layer-open`, `layer-close`               | When a layer-shell surface appears or disappears |
| `layer-companion-close`                   | When a layer companion is dismissed              |
| `workspace-switch`                        | When the active workspace changes                |
| `console-open`, `console-close`           | When the developer console opens or closes       |
| `shadow-change`                           | When a window shadow changes                     |
| `resize`                                  | While a window resizes                           |
| `tile-preview-open`, `tile-preview-close` | When a tile preview appears or disappears        |

## Properties and timing

Use only the properties supported by the event. Positions and sizes use logical
pixels. `scale` is a multiplier (`1` is unchanged); `rotation` uses degrees.
`opacity` and `progress` range from 0 to 1.

| Events                                                                      | Properties                                                     |
| --------------------------------------------------------------------------- | -------------------------------------------------------------- |
| Window, dialog, layer, console and companion actor transitions              | `x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation`, `opacity` |
| `tile-preview-open`, `tile-preview-close`                                   | `x`, `y`, `width`, `height`, `opacity`                         |
| `workspace-switch`, `resize`, `shadow-change`, `dialog-dim`, `dialog-undim` | `progress`                                                     |

`scale` changes both axes. Use `scale_x` or `scale_y` for one axis. `origin`
sets the pivot: `center`, a corner/edge such as `bottom-center`, or a normalized
pair such as `{0.5, 1}`.

`from` and `to` set the start and end values. For an intermediate pose, use
ordered keyframes from `at = 0` through `at = 1`:

```lua
gnoblin.animation {
    name = "spring-open",
    event = "open",
    duration = 360,
    keyframes = {
        {at = 0, y = 20, scale = 0.9, opacity = 0},
        {at = 0.7, y = -4, scale = 1.03, opacity = 1},
        {at = 1, y = 0, scale = 1, opacity = 1},
    },
}
```

Omitted properties keep their current value. A keyframe's optional `ease`
sets the curve up to that frame.

`duration` is milliseconds (`0`–`10000`). `ease` can be `linear`,
`ease-in-quad`, `ease-out-quad`, `ease-in-cubic`, `ease-out-cubic`,
`ease-in-out-cubic`, `ease-out-expo`, `ease-out-back`, or a cubic Bézier curve:

```lua
ease = {type = "cubic-bezier", x1 = 0.2, y1 = 1.4, x2 = 0.35, y2 = 1}
```

Bezier `x1` and `x2` must be 0–1. The y values may overshoot. This changes the
timing curve; use an intermediate keyframe to make the animated property
overshoot.

## Built-in animations

Use `gnoblinctl animation list` to see available names and
`gnoblinctl animation inspect NAME` to see resolved values. `gnome-*` names
follow GNOME Shell motion where there is a matching transition. Gnoblin also
uses that prefix for related events without a GNOME equivalent.

| Name                                                                  | Default motion                                                                                                     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `gnome-minimize`, `gnome-restore`                                     | 400 ms, ease-out expo; move to/from dock icon geometry. Fall back to the monitor edge; monitor-sized windows fade. |
| `gnome-open`                                                          | 150 ms, ease-out expo; scale from 0.01 × 0.05 and fade in, pivot at bottom-center.                                 |
| `gnome-close`                                                         | 150 ms, ease-out quad; scale to 0.8 and fade out.                                                                  |
| `gnome-dialog-open`, `gnome-dialog-close`                             | 100 ms, ease-out quad; vertically expand/collapse. Open also fades in.                                             |
| `gnome-workspace-switch`                                              | 250 ms, ease-out cubic on `progress`.                                                                              |
| `gnome-resize`, `gnome-tile-preview-open`, `gnome-tile-preview-close` | 250 ms, ease-out quad on compositor-supplied values.                                                               |
| `gnome-dialog-dim`, `gnome-dialog-undim`                              | 500 ms / 250 ms, ease-out quad on `progress`.                                                                      |
| `gnoblin-layer-open`, `gnoblin-layer-close`                           | 250 ms, ease-out cubic; slide from/to the layer's anchor. Fade when there is no offset.                            |
| `gnoblin-console-open`, `gnoblin-console-close`                       | 140 ms, ease-out quad; slide by the console height.                                                                |
| `gnoblin-shadow-change`                                               | Uses the shadow's configured duration and easing on `progress`.                                                    |
| `gnoblin-layer-companion-close`                                       | 180 ms, ease-in quad; move by the dismissal offset.                                                                |

The minimize/restore presets calculate their destination from live geometry.
`target` only labels a destination for inspection; it does not provide live
geometry to a custom animation.

## Copy and customize a preset

This registration copies the `gnome-open` values. Change its duration, curve,
pivot or keyframes to make it your own:

```lua
gnoblin.animation {
    name = "my-open",
    event = "open",
    duration = 150,
    ease = "ease-out-expo",
    origin = "bottom-center",
    from = {scale_x = 0.01, scale_y = 0.05, opacity = 0},
    to = {scale_x = 1, scale_y = 1, opacity = 1},
}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    animation = {open = "my-open"},
}
```

Custom names are event-specific. Use an event map when selecting different
animations for opening and closing:

```lua
animation = {open = "my-open", close = "my-close"}
```

## Layer-shell surfaces

Use `layer-open` and `layer-close` in a rule matching the layer namespace:

```lua
gnoblin.window_rule {
    match = {type = "layer", layer = "^my-panel$"},
    animation = { ["in"] = "gnoblin-layer-open", out = "gnoblin-layer-close" },
}
```

Layer anchors set the default slide direction. Custom `x` and `y` values
override it.

## Preview

Previews start paused. Get the session ID, then seek or step through it. Stop
to restore the target:

```sh
session=$(gnoblinctl animation preview gnome-open --window active --format json | python3 -c 'import json,sys; print(json.load(sys.stdin)["session"])')
gnoblinctl animation seek "$session" 50
gnoblinctl animation step "$session" 16
gnoblinctl animation stop "$session"
```

For a layer, use `--layer ID` or `--namespace NAME` instead of `--window`.
`gnoblinctl animation inspect NAME` shows resolved values before you preview.
Console, shadow, workspace, resize and dialog effects cannot currently be
previewed against a window or layer surface.

Other built-in names include `gnome`, `zoom`, `fade`, `slide` and `none`.

Preset definitions are in
[`gnoblinAnimation.js`](https://github.com/kierandrewett/gnoblin/blob/main/src/gnome-shell-overlay/js/ui/components/gnoblinAnimation.js).
GNOME Shell's window and workspace references are in
[`windowManager.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/cbc0ba9afaf26c0f579da87aca3de6be7ba5d914/js/ui/windowManager.js)
and [`workspacesView.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/cbc0ba9afaf26c0f579da87aca3de6be7ba5d914/js/ui/workspacesView.js).
