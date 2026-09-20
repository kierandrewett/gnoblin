# Window effects

[Configuration reference](configuration-reference.md)

Use [window rules](window-rules.md) to apply effects. Edits reload on save.
These settings change application windows; your shell controls its own panel styling.

## Add an effect

Append after your component includes:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 14, smoothing = 0.6},
}
```

The examples below show fields to put inside a rule. Later matching rules
override the fields they specify.

## Blur and opacity

| Field                 | Values                   | Effect                                              |
| --------------------- | ------------------------ | --------------------------------------------------- |
| `blur`                | Integer 0–100            | Background blur strength; 0 disables it             |
| `opacity`             | Number 0–1               | Opacity of the whole window, including text         |
| `blur_ignore_shadows` | Boolean; default `false` | Exclude translucent black pixels from the blur mask |

The client needs a translucent background for blur to show through.
Keep rule opacity at 1 for readable text; adjust the client's background alpha
to change the glass tint.

`blur_ignore_shadows` also excludes black translucent glass. Prefer
[explicit blur regions](background-effects.md) when writing a shell.

## Rounded window corners

The `corners` table controls shape. Radius 0 disables rounding.

| Field                           | Default        | Values                                                        |
| ------------------------------- | -------------- | ------------------------------------------------------------- |
| `radius`                        | `0`            | 0–200 logical pixels                                          |
| `smoothing`                     | `0`            | 0–1; circular to a squarer curve                              |
| `mode`                          | `"auto"`       | `auto`, `force`, `off`                                        |
| `padding`                       | `{0, 0, 0, 0}` | Top/right/bottom/left inset, −128–128 logical pixels          |
| `keep_maximized`                | `true`         | Keep rounding when maximised                                  |
| `keep_fullscreen`, `keep_tiled` | `false`        | Keep rounding in those states                                 |
| `skip_libadwaita`               | `true`         | Preserve libadwaita corners in auto mode                      |
| `skip_libhandy`                 | `false`        | Skip libhandy windows in auto mode                            |
| `remove_csd`                    | `false`        | Reconstruct supported client corner gaps                      |
| `shadow`                        | `false`        | A shadow table or 1–4 shadow layers                           |
| `keep_shadow`                   | `false`        | Keep replacement shadows in maximised/fullscreen/tiled states |

**Auto** preserves existing client corners. **Force** clips to Gnoblin's shape
and removes the original shadow outside it. **Off** disables the mask.

`remove_csd` treats client **corners**, not titlebars. It works conservatively
with stable client edges; uncertain backgrounds keep their native shape.

Legacy `corners.border-width` accepts −40–40 pixels and
`corners.border-color` defaults to `"#808080ff"`.
Use the separate `borders` table for new configurations.

## Inner and outer window borders

Put a `borders` table in the same rule:

```lua
local borders = {
    inner_width = 1,
    inner_color = "#505050ff",
    outer_width = 0,
}
```

This is a field value, not a complete rule.

Widths accept 0–40 logical pixels. Both default to zero.
Colours use `#RRGGBB` or `#RRGGBBAA`, with alpha last.

Borders inherit `radius`, `smoothing` and `padding` from corners unless
you override them. Their ranges are the same as the corner fields above.

Borders do not clip content or reserve space. The outer stroke extends beyond
the frame. Set `keep_maximized`, `keep_fullscreen` or `keep_tiled` to
`false` to hide a border in that state.

## Shadows

A `corners.shadow` table accepts `x`, `y`, `blur`, `spread`,
`opacity` and `color`. For multiple layers:

```lua
local shadow = {
    {x = 0, y = 10, blur = 36, spread = 0, opacity = 0.22},
    {x = 0, y = 2, blur = 5, spread = 0, opacity = 0.28},
}
```

Assign this value to `corners.shadow` in a rule.
Layers draw in order. A later rule replaces a whole layer list; a single-table
shadow merges field by field.

To fade between focused and unfocused shadow styles, set:

```lua
local shadowAnimation = {
    duration = 180,
    easing = "ease-out-cubic",
}
```

Assign it to `corners.shadow_animation`.
Duration accepts 0–2000 ms and defaults to 0. It uses the same
[easing names](animations.md#easing) as layer animations.

## Custom fragment shaders

A rule's `shader` names a GLSL file; `shader_uniforms` supplies numeric
parameters. Use `shader = ""` to clear a shader.

See [custom shaders](shaders.md) for a complete example and reload behavior.
For compositor implementation and test coverage, see [effect rendering](effects-rendering.md).
