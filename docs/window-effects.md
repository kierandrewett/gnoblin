# Window effects

Gnoblin applies effects in the compositor. Rules in
`~/.config/gnoblin/gnoblin.toml` apply to existing windows when the file changes.
This requires the Gnoblin native build. After an upgrade, log out and in once
to load the native code. Subsequent rule and shader edits do not need a restart.

For Bingux's dock, search and shared context menus:

```toml
[[window-rules]]
match.type = "layer"
match.layer = "^(bingux-dock|bingux-search|gnoblin-shell-popup)$"
blur = 24
opacity = 1.0
# Optional custom effect, relative to this config file:
# shader = "shaders/tint.frag"
# shader-uniforms.strength = 0.08
```

Client backgrounds must have some transparency for background blur to show.
Rule opacity changes client pixels without weakening the backdrop blur mask.
Keep rule opacity at 1.0 to preserve the opacity of text and icons. Use the
client's background colour alpha to control the amount of background visible.
Fully transparent parts of a layer surface remain transparent, including
outside rounded cards. The custom shader changes client pixels; the built-in
blur processes the background separately.

Rules can match `type` (`"layer"` or `"window"`), `layer` (layer namespace),
`app-id`, `title`, and `focused` (boolean). String matchers other than `type`
are regular expressions. Every matcher in a rule must match. Later rules
override each effect they specify. Available effects are `blur` (integer
0–100), `opacity` (0–1), `animation` (`"slide"`, `"fade"`, or `"none"`),
`shader`, and `shader-uniforms`. A later uniforms table replaces the earlier
table. Set `shader = ""` to remove a shader.

## Custom fragment shaders

Create the shader directory, then copy [the tint example](../src/data/shaders/tint.frag)
to `~/.config/gnoblin/shaders/tint.frag`. A shader supplies this function:

```glsl
uniform float strength;

vec4 gnoblin_effect(vec4 color, vec2 uv) {
    return vec4(mix(color.rgb, vec3(0.24, 0.30, 0.40), strength), color.a);
}
```

`color` is straight RGBA, and `uv` is the surface texture coordinate from 0 to 1.
Gnoblin manages premultiplied alpha and prevents the shader from filling
transparent areas. `gnoblin_width` and `gnoblin_height` are float uniforms for
the surface dimensions in logical pixels. Names beginning with `gnoblin_`
are reserved. User uniforms are floats; declare them in GLSL and set their
values in `shader-uniforms`. Unset uniforms default to zero.

Use GLSL compatible with version 120 (desktop GL) or 100 (GLES). Do not supply
`#version` or `main`; Gnoblin supplies these. Files must be UTF-8 and at most
64 KiB. Absolute paths and `~/` paths also work. These are per-surface fragment
effects, without a time loop, geometry changes or a multi-pass shader API.

Gnoblin watches the shader's parent directory, including atomic file replacement.
Edits settle for 100 ms before compilation. Create the directory before enabling
the rule. Invalid or missing shader files keep the previous working effect and
log a `gnoblin-shader` warning. Fixing the file restores normal hot reload.
Invalid TOML keeps the previous configuration.

## Validation

`tests/window-shader-config-test.js` checks configuration and rule precedence.
`scripts/test-window-shaders.py` runs in an isolated headless Gnoblin session
through `scripts/run-gnome-shell.sh`. It checks rendered pixels, transparency,
blur composition, atomic edits, invalid and deleted files, uniforms, and removal.
`scripts/test-window-effects.py` checks background blur against a checkerboard.

Bingux context menus use full redraws while mapped with background blur. This
prevents the blur from sampling stale pixels around window shadows during
partial redraws. Partial redraws resume when the last blurred menu closes.
The policy does not disable window culling or force continuous rendering.
