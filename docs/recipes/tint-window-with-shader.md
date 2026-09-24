# Tint windows with a shader

Save a fragment shader as `~/.config/gnoblin/shaders/tint.frag`:

```glsl
uniform float strength;

vec4 gnoblin_effect(vec4 color, vec2 uv) {
    return vec4(mix(color.rgb, vec3(0.24, 0.30, 0.40), strength), color.a);
}
```

Apply it to windows from a chosen app by adding this rule to `init.lua`. Replace
the example app ID with the one used by your app:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org\.example\.Editor$]]},
    shader = "shaders/tint.frag",
    shader_uniforms = {strength = 0.08},
}
```

Paths are relative to `init.lua`. `strength` is the custom float supplied to
the shader; change it to adjust the tint. See the [shader guide](/guides/shaders)
for inputs, limits and reload behavior.
