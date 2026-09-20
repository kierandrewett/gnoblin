# Custom shaders

[Configuration reference](configuration-reference.md)

Shaders change a window's pixels. Background blur is a separate effect.

## 1. Create the shader

Save this as `~/.config/gnoblin/shaders/tint.frag`:

```glsl
uniform float strength;

vec4 gnoblin_effect(vec4 color, vec2 uv) {
    return vec4(mix(color.rgb, vec3(0.24, 0.30, 0.40), strength), color.a);
}
```

Create the directory first. Do not add `#version` or `main`; Gnoblin supplies them.

## 2. Apply it to a window

After your config includes:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    shader = "shaders/tint.frag",
    shader_uniforms = {strength = 0.08},
}
```

Paths are relative to the configuration file. Absolute and `~/` paths also work.
Set `shader = ""` in a later matching rule to remove it.

## Inputs and limits

| Input                             | Meaning                                          |
| --------------------------------- | ------------------------------------------------ |
| `color`                           | Straight RGBA; Gnoblin handles premultiplication |
| `uv`                              | Texture coordinates from 0 to 1                  |
| `gnoblin_width`, `gnoblin_height` | Surface size in logical pixels                   |
| Custom uniforms                   | Floats; unset values are zero                    |

Names beginning with `gnoblin_` are reserved.
A later rule's uniform table replaces the earlier one.

Use GLSL 120 for desktop GL or GLSL 100 for GLES.
Files must be UTF-8 and at most 64 KiB. There is no time loop,
geometry shader or multi-pass API.

## Reload

Files reload after edits settle for 100 ms. Invalid or missing shader files
keep the previous working effect and log a `gnoblin-shader` warning.
Fix the file to retry.

See [effect tests](effects-rendering.md#tests) for pixel and reload coverage.
