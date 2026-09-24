# Animation guide

Register a named animation once, then refer to it from shell settings or
window rules. The same registry drives ordinary transitions and
`gnoblinctl` previews. Windows and layer-shell surfaces share the same
keyframe properties; their lifecycle events differ.

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

Events are `minimize`, `restore`, `open`, `close`, `dialog-open`,
`dialog-close`, `layer-open`, `layer-close`, and `workspace-switch`. A
declaration has one event. `from` and `to` describe endpoints; alternatively
provide ordered `keyframes` with `at` positions from 0 to 1:

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

Animated properties are `x`, `y`, `scale`, `scale_x`, `scale_y`,
`rotation` in degrees, and `opacity` from 0 to 1. `scale` affects both axes;
use separate axes for squash and stretch. `origin` accepts `center`,
`top-left`, `top-center`, `top-right`, `bottom-left`, `bottom-center`,
`bottom-right`, or a normalized `{x, y}` pivot. `ease` accepts `linear`,
`ease-out-quad`, `ease-out-cubic`, `ease-out-expo`, or `ease-in-out-cubic`.
`target` can name `dock`, `icon`, or `monitor` geometry for transitions that
move toward a destination. With no explicit target, GNOME presets use the
appropriate icon or monitor geometry when available.

## GNOME presets

GNOME-style presets use the `gnome-` prefix: `gnome-minimize`, `gnome-restore`,
`gnome-open`, `gnome-close`, `gnome-dialog-open`, `gnome-dialog-close`,
`gnome-layer-open`, `gnome-layer-close`, and `gnome-workspace-switch`.
Aliases `gnome`, `zoom`, `fade`, `slide`, and `none` are also available.
Use `gnoblinctl animation list` to see all registered and built-in names;
`inspect` shows the resolved event and values.

## Select animations

```lua
gnoblin.configure {
    shell = {
        minimize_animation = "gnome-minimize",
        layer_animation = "soft-open",
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    animation = "soft-open",
}
```

`minimize_animation` selects minimize and restore transitions;
`layer_animation` selects layer-shell entry and exit. Window rules can select
a registered name or separate entry and exit names, for example
`animation = { ["in"] = "soft-open", out = "gnome-close" }`. Set a rule to
`"none"` when the shell already animates that surface.

Gnoblin respects GNOME reduced motion and its global animation setting;
disabled animations complete immediately.

## Layer-shell surfaces

Layer surfaces use `layer-open` and `layer-close`. Match a namespace in a
`gnoblin.window_rule` and select the registered animation by name. The layer's
anchors determine the default slide direction; explicit `x` and `y` keyframes
can override it. Preview targets can be selected by layer ID or namespace.

## Preview and step through an animation

```sh
gnoblinctl animation list
gnoblinctl animation surfaces
gnoblinctl animation inspect soft-open --window 42
gnoblinctl animation preview soft-open --window 42
# session: 7
gnoblinctl animation seek 7 50
gnoblinctl animation step 7 16
gnoblinctl animation play 7
gnoblinctl animation pause 7
gnoblinctl animation stop 7
```

Previews start paused and do not minimize or close the target. Omit
`--window` to use the active window. For layer-shell, use `--layer ID` or
`--namespace NAME`; namespace selection must resolve to one visible surface.
`seek` takes an integer percentage from 0 to 100, `step` advances by integer
milliseconds, and `stop` restores
the original visual state. See [gnoblinctl animation commands](/gnoblinctl#animations).
