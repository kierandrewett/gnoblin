# Animation guide

Register a named animation once. The first registration for an event becomes
its default. A window rule can select a different registered animation for
matching windows or layer surfaces.

The shared registry drives compositor-owned motion for windows, layer-shell
surfaces, shadows and workspace transitions. Window and layer-shell surfaces use
the same keyframe properties, but have different lifecycle events.

## Register a custom animation

Put declarations in `~/.config/gnoblin/init.lua` or in a file loaded with
`gnoblin.load`.

Each `gnoblin.animation { ... }` call registers one event under a reusable
name. The first declaration for that event supplies its default; use a window
rule to select another registration for matching windows or surfaces.

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

| Field                      | Accepted values                                        |
| -------------------------- | ------------------------------------------------------ |
| `name`                     | Unique string of up to 80 letters, digits, `_` or `-`. |
| `duration`                 | Integer milliseconds from 0 to 10,000.                 |
| `from`, `to`               | Endpoint property maps; use these or `keyframes`.      |
| `keyframes`                | Ordered property maps with `at` positions from 0 to 1. |
| `ease`, `origin`, `target` | Optional curve, transform pivot, and inspection label. |

Declarations with the same name merge when config files are loaded. Disable an
imported registration by name:

```lua
gnoblin.animation {name = "soft-open", enable = false}
```

Each declaration uses one event. Choose the event that describes **when** the
animation runs:

| Event                         | When it runs                                                                   |
| ----------------------------- | ------------------------------------------------------------------------------ |
| `minimize`                    | When a window minimizes                                                        |
| `restore`                     | When a window restores                                                         |
| `open`, `close`               | When a window opens or closes                                                  |
| `dialog-open`, `dialog-close` | When a dialog opens or closes                                                  |
| `layer-open`, `layer-close`   | When a layer-shell surface appears or disappears                               |
| `workspace-switch`            | When the active workspace changes                                              |
| `shadow-change`               | When a window shadow changes                                                   |
| `resize`                      | When Mutter changes a window's geometry, including `window:resize()` requests. |

Bingux-owned UI transitions remain Bingux's responsibility.

Resize animations run for normal windows when Mutter changes geometry for
maximize, unmaximize, fullscreen, unfullscreen, or a monitor move. Interactive
edge-drag resizing does not trigger this event.

`from` and `to` describe endpoints; alternatively provide ordered `keyframes`
with `at` positions from 0 to 1:

```lua
gnoblin.animation {
    name = "springy-open",
    event = "open",
    duration = 360,
    keyframes = {
        {at = 0, y = 24, scale = 0.9, rotation = -2, opacity = 0},
        {at = 0.7, y = -4, scale = 1.03, rotation = 1, opacity = 1},
        {at = 1, y = 0, scale = 1, rotation = 0, opacity = 1},
    },
}
```

## Properties and keyframes

Only use properties supported by the event. Values are numbers; lengths and
translations are logical pixels. `opacity` and `progress` range from 0 to 1.
Scale is a multiplier where 1 is the original size. `rotation` is in degrees.

| Events                                              | Properties                                                     |
| --------------------------------------------------- | -------------------------------------------------------------- |
| Window, dialog, layer, console and companion events | `x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation`, `opacity` |
| `workspace-switch`, `resize`, `shadow-change`       | `progress`                                                     |

| Field                | Meaning and accepted values                                                                                                                   |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `scale`              | Multiplies both axes; `1` keeps the original size.                                                                                            |
| `scale_x`, `scale_y` | Scale each axis independently for squash or stretch.                                                                                          |
| `origin`             | `center`, `top-left`, `top-center`, `top-right`, `bottom-left`, `bottom-center`, `bottom-right`, or a normalized Lua pair such as `{0.5, 1}`. |
| `x`, `y`             | Logical-pixel translation of the actor; does not change window geometry.                                                                      |

`from` and `to` are sparse maps. A property omitted at one endpoint keeps its
current value there.

For intermediate poses, use `keyframes`:

- `at` is normalized timeline time from 0 to 1.
- The first and last frames must be at 0 and 1.
- List frames in increasing time order. Each property interpolates between the
  frames that define it.
- A frame's optional `ease` controls the segment ending at that frame. Otherwise
  the declaration's `ease` applies.

### Easing {#easing}

`ease` accepts `linear`, `ease-in-quad`, `ease-out-quad`, `ease-in-cubic`,
`ease-out-cubic`, `ease-in-out-cubic`, `ease-out-expo` and `ease-out-back`.
For another curve, use cubic Bézier control points:

```lua
gnoblin.animation {
    name = "elastic-open",
    event = "open",
    duration = 420,
    origin = "bottom-center",
    ease = {type = "cubic-bezier", x1 = 0.2, y1 = 1.5, x2 = 0.35, y2 = 1},
    from = {y = 20, scale = 0.88, opacity = 0},
    to = {y = 0, scale = 1, opacity = 1},
}
```

The `x1` and `x2` control points represent time and must be from 0 to 1. The
`y1` and `y2` values range from -2 to 2, allowing a curve to overshoot.

A Bézier curve changes timing; it does not add spring physics. For a bounce,
add an overshooting value in an intermediate keyframe.

Workspace transitions, resize effects, and shadow changes expose `progress`
from 0 to 1. For these events, animate `progress` and let the effect
consume it; actor properties such as `x` and `scale` are not accepted.

Workspace switches use the selected animation's `progress` to move the outgoing
and incoming workspaces in Mutter's switch direction. Custom keyframes change
the transition timing and progress curve.

Resize animations use `progress` to interpolate the window from its previous
buffer geometry to its new geometry. Custom keyframes control how the window
settles into the new size.

`target` is an optional label shown by inspection tools. Minimize and restore
presets calculate their destination from dock/icon or monitor geometry. Custom
animations should supply their own `x` and `y` values when they need to move
toward a specific place.

## Recreate a preset

For example, this gives a custom `open` animation the same duration, curve,
pivot and endpoints as `gnome-open`. Register a second declaration for the
close event, then select both names in one window rule:

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

gnoblin.animation {
    name = "my-close",
    event = "close",
    duration = 150,
    ease = "ease-out-quad",
    from = {scale = 1, opacity = 1},
    to = {scale = 0.8, opacity = 0},
}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org%.example%.Editor$"},
    animation = {open = "my-open", close = "my-close"},
}
```

This copies the preset's duration, curve, pivot, and endpoints. Change those
values or add keyframes without affecting other windows.

`gnome-minimize` and `gnome-restore` calculate their destination from dock-icon
or monitor geometry. A custom animation's `target` is only an inspection label;
it does not make the animation follow the dock. Keep the presets when you need
their geometry-aware behavior.

## Built-in animations

Preset names select definitions in Gnoblin's shared animation registry.
Profiles named `gnome-*` follow GNOME Shell timing and motion where equivalent
transitions exist.

Gnoblin also uses that naming family for its layer-shell and shadow events.
Those profiles belong to Gnoblin.

| Preset                                      | Default profile and motion                                                                                                                  |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnome-minimize`, `gnome-restore`           | 400 ms, ease-out expo; travel to/from dock icon geometry, or the monitor edge if no icon geometry is available. Monitor-sized windows fade. |
| `gnome-open`                                | 150 ms, ease-out expo; scales from 0.01 × 0.05 and fades in around the bottom-center pivot.                                                 |
| `gnome-close`                               | 150 ms, ease-out quad; scales to 0.8 and fades out.                                                                                         |
| `gnome-dialog-open`, `gnome-dialog-close`   | 100 ms, ease-out quad; vertically expands or collapses and fades on open.                                                                   |
| `gnome-workspace-switch`                    | 250 ms, ease-out cubic; eases the workspace transition's `progress`.                                                                        |
| `gnome-resize`                              | 250 ms, ease-out quad; eases the resize geometry supplied by the compositor.                                                                |
| `gnoblin-layer-open`, `gnoblin-layer-close` | 250 ms, ease-out cubic; slides from/to the layer's anchor-derived offset, or fades when the offset is zero.                                 |
| `gnoblin-shadow-change`                     | Uses the configured shadow duration and easing; animates shadow `progress`.                                                                 |

These are resolved defaults. Geometry-dependent destinations vary with the
window, dock, monitor, and layer anchors. Custom registrations can replace an
event's profile.

Inspect the current values with:

- `gnoblinctl animation list`
- `gnoblinctl animation inspect NAME --window active`
- `gnoblinctl animation inspect NAME --layer ID` or `--namespace NAME` for
  layer surfaces.

To see the motion, start a paused preview and step or seek it as described
below.

Aliases `gnome`, `zoom`, `fade`, `slide` and `none` are also available. `none`
completes immediately. `fade` changes opacity only. `zoom` minimizes toward the
dock target. `slide` uses the layer's anchor-derived offset; it is the default
policy for layer surfaces.

## Select animations

```lua
gnoblin.animation {
    name = "soft-minimize",
    event = "minimize",
    duration = 250,
    ease = "ease-out-cubic",
    from = {scale = 1, opacity = 1},
    to = {scale = 0.85, opacity = 0},
}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    animation = {minimize = "soft-minimize"},
}
```

The first registered animation for an event is its default. A window rule
overrides that choice for matching windows or layer surfaces. Set a rule to
`"none"` when the client already animates its own contents.

For shadow transitions, select a `shadow-change` animation in the corner
configuration:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {
        shadow = {x = 0, y = 8, blur = 24, opacity = 0.2},
        shadow_animation = {animation = "gnoblin-shadow-change"},
    },
}
```

Without an explicit animation name, Gnoblin uses a registered `shadow-change`
declaration when available. The existing `duration` and `easing` fields remain
available for a simple fade.

Gnoblin respects GNOME reduced motion and its global animation setting;
disabled animations complete immediately.

## Layer-shell surfaces

Layer surfaces use `layer-open` and `layer-close`. Match a namespace in a
`gnoblin.window_rule` and select entry and exit animations by name:

```lua
gnoblin.window_rule {
    match = {type = "layer", layer = "^my-panel$"},
    animation = { ["in"] = "gnoblin-layer-open", out = "gnoblin-layer-close" },
}
```

For custom motion, register separate `soft-layer-open` and `soft-layer-close`
animations with events `layer-open` and `layer-close`. Use their names in the
`in` and `out` fields.

The layer's anchors determine the default slide direction. Explicit `x` and
`y` keyframes can override it. Select a preview target by layer ID or namespace.

## Preview and step through an animation

```sh
gnoblinctl animation list
gnoblinctl animation surfaces
gnoblinctl animation inspect gnome-open --window active
session=$(gnoblinctl animation preview gnome-open --window active --format table | sed -n 's/^session: //p')
gnoblinctl animation seek "$session" 50
gnoblinctl animation step "$session" 16
gnoblinctl animation play "$session"
gnoblinctl animation pause "$session"
gnoblinctl animation stop "$session"
```

Previews start paused and do not minimize or close the target. Omit
`--window` to use the active window. For layer-shell, use `--layer ID` or
`--namespace NAME`; namespace selection must resolve to one visible surface.

`seek` takes an integer percentage from 0 to 100, `step` advances by integer
milliseconds, and `stop` restores
the original visual state. The CLI lists which animations are previewable
against window or layer targets.

Console, shadow, and layer-companion transitions animate internal actors or
effects and cannot currently be previewed against those targets. See
[gnoblinctl animation commands](/gnoblinctl#animations).
