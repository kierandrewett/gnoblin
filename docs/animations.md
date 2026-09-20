# Animations

[Configuration reference](configuration-reference.md)

Choose how windows minimise and how bars or launchers appear. Add these
examples to `~/.config/gnoblin/init.lua`; changes reload on save.
GNOME's reduced-motion setting can disable animations regardless of these values.

## Minimise and restore

```lua
gnoblin.configure {
    shell = {
        minimize_animation = "zoom",
        minimize_duration = 150,
    },
}
```

| Mode               | Result                                       |
| ------------------ | -------------------------------------------- |
| `"zoom"` (default) | Move toward the dock icon or fallback target |
| `"fade"`           | Fade in place                                |
| `"none"`           | No animation                                 |
| `"gnome"`          | GNOME's native icon-target animation         |

Duration is 0–5000 milliseconds; the default is 200.

Gnoblin moves the window toward its dock icon when the dock supplies that
position. Otherwise it uses `minimize_target`, if set, or the monitor's bottom
centre. For a fixed target, add `minimize_target = {800, 900}` inside `shell`.
Coordinates use logical pixels measured across the whole desktop, not from the
corner of each monitor.

## Bars, launchers and other layer surfaces

```lua
gnoblin.configure {
    shell = {
        layer_animation = "slide",
        layer_duration = 220,
        layer_easing = "ease-out-cubic",
    },
}
```

These are the defaults. The mode accepts `"slide"`, `"fade"` or `"none"`.
Duration accepts 0–5000 milliseconds.

Sliding follows the surface's anchor: a top panel enters from the top.
Full-screen input overlays fade. Closing reverses the movement.

Animations run when a bar or popup appears or disappears. Changing text or
other content inside an already visible panel does not restart them. Your shell may also animate its own contents.

## Per-surface animations

Add a rule after your includes:

```lua
gnoblin.window_rule {
    match = {type = "layer"},
    animation = {
        ["in"] = "slide",
        out = "fade",
        duration = 180,
        easing = "ease-out-cubic",
    },
}
```

`in` is a Lua keyword, so it needs brackets. Both directions accept the same
three modes. Omitted fields inherit the shell settings.

For one mode in both directions, use `animation = "fade"`.
Use `"none"` if your shell already animates that panel appearing and disappearing.

## Easing

| Value                 | Motion                     |
| --------------------- | -------------------------- |
| `"linear"`            | Constant speed             |
| `"ease-out-quad"`     | Gentle slowdown            |
| `"ease-out-cubic"`    | Stronger slowdown; default |
| `"ease-in-out-cubic"` | Accelerate, then slow down |

Shell authors can supply [dock icon targets](shell-integration.md#dock-animation-targets)
without animating application windows themselves.
