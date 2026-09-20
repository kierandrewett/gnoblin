# Animations

[Configuration reference](configuration-reference.md)

Configure minimise/restore and layer-shell animations in `shell`.
Changes reload on save. GNOME's reduced-motion setting takes precedence.

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

The target is the dock-supplied icon rectangle, then `minimize_target`,
then the monitor's bottom centre. A configured target is `{x, y}` in logical
desktop coordinates, including monitor offsets.

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

Animations run when a surface maps or unmaps. Content changes inside an existing
surface do not restart them. Your shell may also animate its own contents.

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
Use `"none"` when the client owns its whole-surface transition.

## Easing

| Value                 | Motion                     |
| --------------------- | -------------------------- |
| `"linear"`            | Constant speed             |
| `"ease-out-quad"`     | Gentle slowdown            |
| `"ease-out-cubic"`    | Stronger slowdown; default |
| `"ease-in-out-cubic"` | Accelerate, then slow down |

Shell authors can supply [dock icon targets](shell-integration.md#dock-animation-targets)
without animating application windows themselves.
