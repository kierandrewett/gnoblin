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

It is the only effects path. The `window.add_effect` call and the window's
`effect` objects are gone, and so is the `effect.toml` schema file.

- The kinds are `blur` and `shadow`. Any other kind, `shader` included, is
  refused `bad-value`, and the whole list is refused with it.
- A value outside its range (a tint that is not `#rrggbb` or `#rrggbbaa`, zero
  blur passes, a shadow radius over 100) refuses the whole list. The old stack's
  separate `too-many-passes` and `unknown-effect-kind` errors are gone with it.
- An effect is switched off by setting the list without it. There is no
  `enabled` flag.
- The list holds at most 16 effects. Setting it replaces it, and a snapshot
  carries what was set.
- The renderer has one blur kernel. `algorithm` is accepted and ignored.

### Effects that are not drawn: `skipped`

A node has a read-only `skipped` list. It names each effect in `effects` the
renderer will not draw, as `<kind>: <reason>`, and it is empty when all are drawn.

- The only reason today is `unsupported-effect-on-3d`.
- It applies to a blur or a shadow on a node whose placement carries depth: a
  rotation about x or y, a `z`, or a perspective `Camera` above it. The renderers
  have no depth pass for a backdrop or a shadow.
- The compositor keeps the list current as the node moves, so a node that is
  flat again gets the empty list back.
- A shell cannot write it.
- It reports placement only. The software and Vulkan renderers draw no blur or
  shadow at all, and they do not say so here.

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
