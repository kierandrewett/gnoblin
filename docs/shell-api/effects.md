# Effects

Every node has an `effects` list. It is the one way to give a node a blur or a shadow. The list is drawn in order,
first to last.

```lua
node.effects = {
    { kind = "shadow", offset_y = 4, blur_radius = 16 },
    { kind = "blur", passes = 3, tint = "#10203040" },
}
```

A window, a `Canvas` and a `View` all carry the list. A blurred node fades as one group with its `opacity`: at half
`opacity` the node and the blur of what is behind it fade together, not the node alone.

## Kinds

There are two kinds. A field a kind does not use is ignored.

| `kind`   | Draws                                                        |
| -------- | ------------------------------------------------------------ |
| `blur`   | A blur of what is behind the node, graded and tinted.        |
| `shadow` | A soft shadow behind the node, offset and optionally spread. |

### `blur`

| Field        | Default | Range                    | Drawn                                                                     |
| ------------ | ------- | ------------------------ | ------------------------------------------------------------------------- |
| `passes`     | `3`     | 1 to 8                   | Yes. More passes spread the blur further.                                 |
| `tint`       | none    | `#rrggbb` or `#rrggbbaa` | Yes. A colour mixed over the blurred backdrop. The empty text is no tint. |
| `brightness` | `1.0`   | 0 to 2                   | Yes. Multiplier on the blurred backdrop. `1.0` leaves it unchanged.       |
| `contrast`   | `1.0`   | 0 to 2                   | Yes. Multiplier on the blurred backdrop. `1.0` leaves it unchanged.       |
| `downscale`  | `2`     | 1 to 8                   | No. The blur always runs at full resolution.                              |
| `saturation` | `1.0`   | 0 to 2                   | No. Accepted and kept, but no renderer applies it yet.                    |
| `noise`      | `0.0`   | 0 to 1                   | No. Accepted and kept, but no renderer applies it yet.                    |

The renderer has one blur kernel and one blur radius (12 logical pixels). `algorithm` is accepted and ignored, so a
list written for a renderer with a choice still validates.

### `shadow`

| Field         | Default     | Range                    | Meaning                                                                           |
| ------------- | ----------- | ------------------------ | --------------------------------------------------------------------------------- |
| `offset_x`    | `0.0`       | any finite number        | Horizontal offset in logical pixels. Positive is right.                           |
| `offset_y`    | `0.0`       | any finite number        | Vertical offset in logical pixels. Positive is down.                              |
| `blur_radius` | `12.0`      | 0 to 100                 | Blur of the shadow's edge in logical pixels. `0` is a hard edge.                  |
| `spread`      | `0.0`       | -100 to 100              | How far the shadow grows past the node before it is blurred. Negative shrinks it. |
| `colour`      | `#00000080` | `#rrggbb` or `#rrggbbaa` | The shadow colour.                                                                |

## What a write is checked against

A value outside its range refuses the whole list, and the node keeps the list it had. These are refused: a colour that
is not `#rrggbb` or `#rrggbbaa`, zero blur passes, and a shadow radius over 100.

The list holds at most 16 effects. Setting it replaces it, and a snapshot carries what was set. An effect is switched
off by setting the list without it: there is no `enabled` field.

Any kind other than `blur` or `shadow` is refused, and so is a field an effect does not have. A shell that sets a
`shader` effect is told what replaced it:

```text
user shaders were removed: an effect's kind is blur or shadow, not "shader" (draw a Canvas for anything else)
```

## Effects that are not drawn

An effect can be valid and still not be drawn. A node has a read-only `skipped` list that says which and why, as
`<kind>: <reasons>`, in list order. It is empty when everything is drawn. A shell cannot write it.

| Reason                     | Meaning                                                                                      |
| -------------------------- | -------------------------------------------------------------------------------------------- |
| `unsupported-by-renderer`  | The active renderer draws no effect of that kind (see below).                                |
| `unsupported-effect-on-3d` | The node's placement carries depth, and the renderers have no depth pass for it (see below). |

Which renderers draw what: GLES draws blur and shadow. The software renderer (the default, and what a nested session
uses) and the Vulkan renderer draw neither.

Depth means a rotation about x or y, a `z`, or a perspective `Camera` above the node.

An effect with both reasons names both on one entry, joined by `, `: `blur: unsupported-by-renderer,
unsupported-effect-on-3d`. The compositor keeps the list current as the node moves, so a node that is flat again gets
the empty list back.

The main seat has a read-only `effect_kinds` list of the kinds the active renderer draws: `{ "blur", "shadow" }` on
GLES, empty on the software and Vulkan renderers. It is set once at startup. A shell reads it to decide whether to set
an effect or to draw a flat fallback, such as a tinted `Canvas`.

## What was removed

| Removed                                                                                              | Use instead                                                                                                                |
| ---------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| User shaders and the `shader` effect kind                                                            | A `blur` or `shadow` effect, or a `Canvas` for anything the two cannot do. The compositor runs no shader a shell supplies. |
| `window.add_effect` and the window's `effect` objects (`Node:add_effect` raises and names this page) | The node's `effects` list.                                                                                                 |
| The `effect.toml` schema file and its `too-many-passes` and `unknown-effect-kind` errors             | The list's own checks, above.                                                                                              |
| Per-effect `enabled`, `active`, `reason` and `closed` fields                                         | Set the list without the effect. Read `skipped` for what is not drawn.                                                     |

Background blur that an application asks for through the `ext_background_effect` protocol is a separate path. It is
described in [background effects](/background-effects).
