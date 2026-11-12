# window_effects

> **Status:** partly available. A shell gives a node a blur or a shadow through the node's `effects` list. Window rules are not a way to do it: `gnoblin.window_rule` was removed (rules are `seat.data.window_rules` in a [shell file](/user-shell)), and the stock shell draws blur and shadow only.

This page says which window effects work today and which do not. The source of
truth is the code, not this page.

## What works now

| Effect                 | Where it works                                  | Set from                               |
| ---------------------- | ----------------------------------------------- | -------------------------------------- |
| Opacity, scale, offset | Animated tweens and open/close fades            | `opacity` rule: yes. Scale, offset: no |
| Rounded corners        | Software, GLES and Vulkan renderers (see below) | `corners = {radius = n}` rule: yes     |
| Tint                   | Software, GLES and Vulkan renderers             | No window rule key                     |
| Blur behind a node     | GLES renderer only                              | The node's `effects` list              |
| Shadow                 | GLES renderer only                              | The node's `effects` list              |

## The node `effects` list

Every node has an `effects` list, in paint order, first to last. A shell sets it
like any other property:

```lua
node.effects = {
    { kind = "shadow", offset_y = 4, blur_radius = 16 },
    { kind = "blur", passes = 3, tint = "#10203040" },
}
```

It is the only effects path. The kinds, their fields and ranges, the `skipped`
list that names an effect the renderer will not draw, and `seat.effect_kinds`
are on the [Effects](/shell-api/effects) page. The removed `window.add_effect`
call, user shaders and the `effect.toml` schema file are listed there with what
replaces each.

## Window rules that do not apply

These keys are refused when the config loads. Remove them from your rules; they
have no effect today:

- `blur`, `blur-ignore-shadows`
- `corners` keys other than `radius`, such as `corners.shadow` and `smoothing`
- `shader`, `shader-uniforms`: there are no user shaders. The `shader` effect
  kind and the shader guide were removed; use a `blur` effect's `tint` or draw a
  `Canvas`.
- `animation`
- `frame`
- `match.layer`

A rule that names one of them is refused, and the config reports an error at load.
The reason for each key is in the [refused table](/config/window_rule).

## Background blur from applications {#blur-and-opacity}

Applications and shells can ask for blur behind a region they draw, through the
`ext_background_effect` protocol. That path is separate from the `effects` list and is
described in [background effects](/background-effects).
