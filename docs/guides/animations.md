# Animation guide

Register a named animation once. The first registration for an event becomes
its default. Shell settings and window rules can select a different name.

The shared registry drives compositor-owned motion for windows, layer-shell
surfaces, the developer console, shadows, resizing and workspaces. Window and
layer-shell surfaces use the same keyframe properties, but have different
lifecycle events.

## Register a custom animation

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

Later config fragments can replace a registration with the same name or remove
it with `gnoblin.remove_animation("soft-open")`.

Events are `minimize`, `restore`, `open`, `close`, `dialog-open`,
`dialog-close`, `layer-open`, `layer-close`, `workspace-switch`,
`console-open`, `console-close`, `shadow-change`, `layer-companion-close`,
`resize`, `tile-preview-open`, `tile-preview-close`, `dialog-dim`, and
`dialog-undim`.

A declaration has one event. These cover Gnoblin-managed window lifecycle and
resize motion, workspace transitions, layer surfaces and companion dismissal,
the developer console, shadow changes, tile previews and dialog dimming.
Bingux-owned UI transitions remain Bingux's responsibility.

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

Animation properties depend on the event target:

- Windows, layers, the console and companions: `x`, `y`, `scale`, `scale_x`,
  `scale_y`, `rotation` (degrees) and `opacity` (0 to 1).
- Tile previews: `x`, `y`, `width`, `height` and `opacity`.
- Workspace switching, resizing, shadow changes and dialog dimming: `progress`
  from 0 to 1.

`scale` affects both axes. Use `scale_x` and `scale_y` separately for squash
and stretch.

`origin` accepts a named position or a normalized `{x, y}` pivot:

- Named positions: `center`, `top-left`, `top-center`, `top-right`,
  `bottom-left`, `bottom-center` and `bottom-right`.
- `ease` accepts:
  `linear`, `ease-in-quad`, `ease-out-quad`, `ease-in-cubic`, `ease-out-cubic`,
  `ease-in-out-cubic`, `ease-out-expo` or `ease-out-back`.

For a custom timing curve, use a cubic Bézier value:

```lua
ease = {type = "cubic-bezier", x1 = 0.2, y1 = 0.8, x2 = 0.25, y2 = 1}
```

You can also put `ease` on a keyframe to shape the segment ending there. The
x control points represent time and must stay between 0 and 1. The y values may
overshoot for a bounce.

Workspace transitions animate a scalar `progress` from 0 to 1. Custom
`workspace-switch` keyframes can shape that progress.

`target` labels a transition destination for inspection. GNOME minimize and
restore presets calculate their endpoint from icon or monitor geometry. Custom
`x` and `y` keyframes are actor translations in logical pixels.

## GNOME presets

GNOME-style presets use the `gnome-` prefix: `gnome-minimize`, `gnome-restore`,
`gnome-open`, `gnome-close`, `gnome-dialog-open`, `gnome-dialog-close`, and
`gnome-workspace-switch`. Gnoblin's layer presets are `gnoblin-layer-open` and
`gnoblin-layer-close`; GNOME has no layer-shell animation to reproduce.

GNOME-style presets also include `gnome-resize`,
`gnome-tile-preview-open`, `gnome-tile-preview-close`, `gnome-dialog-dim`,
and `gnome-dialog-undim`. Aliases `gnome`, `zoom`, `fade`, `slide`, and
`none` are also available.

Gnoblin-specific transitions use `gnoblin-console-open`,
`gnoblin-console-close`, and `gnoblin-shadow-change`. Console transitions
animate the internal developer console; shadow changes drive the shadow
effect's progress.

Tile previews, window resizing, dialog dimming, and layer
companion dismissal also use the shared registry and can be customized with
their event names above; the corresponding built-ins include
`gnoblin-layer-companion-close`.

Use `gnoblinctl animation list` to see all registered and built-in names;
`inspect` shows the resolved event and values.

## Select animations

```lua
gnoblin.configure {
    shell = {
        minimize_animation = {minimize = "gnome-minimize", restore = "gnome-restore"},
        layer_animation = {["layer-open"] = "gnoblin-layer-open", ["layer-close"] = "gnoblin-layer-close"},
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    animation = {open = "gnome-open"},
}
```

`minimize_animation` selects minimize and restore transitions.
`layer_animation` selects layer-shell entry and exit. Each accepts one built-in
name or an event map when the two phases need different names.

Window rules select registered animations by event. For a layer rule, match
the animation events to the layer's entry and exit. Set a rule to `"none"` when
the shell already animates that surface.

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
session=$(gnoblinctl animation preview gnome-open --window active --format json | python3 -c 'import json,sys; print(json.load(sys.stdin)["session"])')
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

Console, shadow, tile-preview, dialog-dimming,
and layer-companion transitions animate internal actors or effects and cannot
currently be previewed against those targets. See
[gnoblinctl animation commands](/gnoblinctl#animations).
