# Custom shaders

A shader is a small GPU program that decides the colour of each pixel of a window. A shell can give any window one, to tint
it, to draw a gradient over it, or to build a material from what is behind it.

This page says how to write one, how to attach it to a window's style, what the compositor refuses and what it does not
draw. Custom shaders are off by default: read [Turn them on](#turn-them-on) first.

## Turn them on

The setting is `compositor.user-shaders`. It takes `true` or `false` and defaults to `false`. It is read when the compositor
starts, so a change needs a new session.

`shell.ts` has no object for the `compositor` section yet (see
[settings with no TypeScript home](/js-settings#dropped-settings)), so no line of `shell.ts` turns the setting on. The
compositor reads it from the last configuration document it accepted, and `gnoblinctl config get` prints that document.

Whether the setting is on is also a fact a shell can read. The seat has a read-only `user_shaders` property that is `true`
while the compositor accepts shaders, and `effect_kinds`, which lists the effect kinds the active renderer draws. A shader
is drawn only when both say yes: the setting is on and `effect_kinds` holds `shader`.

## Set one on a window

A `ShaderFilter` is an element of a style's `filter` or `backdropFilter` list. Either list makes the same thing, one
`shader` effect on the window's node. The `backdropFilter`'s shaders come first, after its blur, then the `filter`'s.

In TypeScript:

```ts
import { ShaderFilter } from "gnoblin-std";

const tint = `
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    let tint = vec3<f32>(p[0], p[1], p[2]) * c.a;
    return vec4<f32>(mix(c.rgb, tint, p[3]), c.a);
}
`;

gnoblin.clients.style.filter = [new ShaderFilter({ source: tint, params: [1.0, 0.55, 0.2, 0.25] })];
```

A literal does the same: `{ shader: { source: tint, params: [1.0, 0.55, 0.2, 0.25] } }`.

In Rust, with `gnoblin-std`:

```rust
std.clients()?.style()?.set_filter(&[Filter::shader(TINT, &[1.0, 0.55, 0.2, 0.25])])?;
```

In Python, over the generated bindings of `libgnoblin-std`:

```python
style = Style.scope(StyleKind.SCOPE_COLLECTION, clients().handle)
style.set_list(StyleKind.FILTER, [StyleEntry.of_shader(TINT, [1.0, 0.55, 0.2, 0.25])])
```

The scopes work as they do for every style property: a client's own style beats a `where` selection, which beats its
workspace, which beats the global style. A list a scope sets replaces the inherited list whole. See [window style](/js-style).

The source tree has complete shells that set the tint of this page: `examples/19-shader-tint-ts/shell.ts`,
`examples/19-shader-tint/src/lib.rs` (Rust) and `examples/19-shader-tint-py/shell.py`. See [build a desktop](/build-a-desktop#the-examples).

## What the shader is

The source is WGSL. It defines one function, `effect`, and may add constants, structs and helper functions:

```wgsl
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32>
```

| Name   | Meaning                                                                                                  |
| ------ | -------------------------------------------------------------------------------------------------------- |
| `c`    | The window's own colour at this pixel, premultiplied by its alpha.                                       |
| `uv`   | Where this pixel is over the window's rectangle: `(0, 0)` is the top left and `(1, 1)` the bottom right. |
| `p`    | The numbers the shell gave, `p[0]` to `p[7]`. Numbers the shell did not give are 0.                      |
| `t`    | Time in seconds.                                                                                         |
| result | The colour to draw, premultiplied.                                                                       |

The compositor clamps the result to a valid premultiplied colour, then applies the window's opacity and cuts it to the
window's rounded corners, so a shader does not have to know about either.

The shader may also call `backdrop(uv)`, which returns the premultiplied colour of what is behind the window at `uv`. A
`uv` outside 0 to 1 reads the nearest edge pixel. See [the backdrop](#the-backdrop).

The numbers do not change on their own. A shell animates a shader through `t`, or by writing new `params`. A transition
does not tween `params` yet.

### Limits

The compositor checks every source when the shell writes it, so a bad shader is refused the same way on every renderer.

| Limit                                    | Value                                                                                                                                                                                     |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source size                              | 16 KiB                                                                                                                                                                                    |
| Numbers in `params`                      | 8, each finite                                                                                                                                                                            |
| Shaders on one window                    | 4                                                                                                                                                                                         |
| Effects on one window, shaders included  | 16                                                                                                                                                                                        |
| Loops                                    | Only `for (var i = 0; i < 8; i++)`: a constant start, a constant bound with `<` or `<=`, a constant positive step, and a counter the body does not write. `loop` and `while` are refused. |
| Trips of one loop                        | 64                                                                                                                                                                                        |
| Trips of nested loops, multiplied        | 256                                                                                                                                                                                       |
| Texture reads, `backdrop` calls included | 16 in the worst case of `effect`: a `for` body counts once for each trip, and an `if` counts its larger arm                                                                               |
| Expressions in your functions            | 2048                                                                                                                                                                                      |
| Recursion                                | None                                                                                                                                                                                      |

The source may not declare:

- a module-level `var`
- an `override`
- a texture, a sampler or a buffer
- an entry point

No function may take a pointer. There are no storage buffers, atomics or compute.

The names `shade`, `backdrop` and `GnoblinEffectOut` belong to the compositor, and so does any name that starts with
`gnoblin`.

## Example: a tint

This is the shader of the examples. It moves the window's colour towards the colour in `p[0]` to `p[2]` by `p[3]`:

```wgsl
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    let tint = vec3<f32>(p[0], p[1], p[2]) * c.a;
    return vec4<f32>(mix(c.rgb, tint, p[3]), c.a);
}
```

Set with the numbers `[1.0, 0.55, 0.2, 0.25]`, it moves every pixel a quarter of the way towards orange. `c.a` scales the
tint because `c` is premultiplied: a transparent pixel stays transparent.

What you see on a renderer that draws shaders:

- An opaque white pixel, `(1, 1, 1)`, becomes `(1, 0.8875, 0.8)`.
- An opaque black pixel becomes `(0.25, 0.1375, 0.05)`.
- A window of mixed colours looks warmer, and its shape, corners, shadow and border do not change.

## Example: using the backdrop

This shader draws the window over a smear of what is behind it. It takes seven samples along the row, `p[0]` of the
window's width apart:

```wgsl
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 7; i++) {
        sum = sum + backdrop(uv + vec2<f32>((f32(i) - 3.0) * p[0], 0.0));
    }
    let behind = sum / 7.0;
    return c + behind * (1.0 - c.a);
}
```

Set with `params: [0.01]`. The loop makes seven reads, within the limit of sixteen. `c + behind * (1.0 - c.a)` is the
usual "over": a pixel the window covers fully keeps its own colour, and a pixel it leaves clear shows the smear. A
translucent terminal over a photo shows the photo, softened sideways. An opaque window looks as it did.

## The backdrop

A shader that reads the backdrop reads other windows, so the compositor gives it what a screen capture shows and nothing
more:

- Secure windows, the lock screen's surfaces, authentication prompts and the lock card read as opaque black. This holds
  whatever `capture.secure-treatment` says: a capture may draw a grey mask, but a shader always gets black.
- The cursor is not in it, and neither are the view and the colour matrix.
- The backdrop is the windows below the shader's own window. A window never reads itself or the windows above it.
- A shader writes only its own pixel. It has no buffers and no way to read a result back, so nothing it computes reaches
  the shell.

A renderer builds a backdrop only for a window whose shader can reach `backdrop`, so a shader that does not call it costs
nothing extra.

## When a shader is not drawn

A shader can fail to appear for five reasons. Each one leaves the window drawn without it and says why.

### The setting is off

The library does not write the shader, so the compositor has nothing to refuse. The rest of the style is written.

The client's style warnings say so, and a TypeScript shell logs the same line once:

```text
not applied: style.filter: a ShaderFilter is not drawn because the compositor's custom shaders are off (set `compositor.user-shaders`); the window is drawn without it
```

### The source does not validate

The compositor refuses the window's whole `effects` list. A shadow or border written beside the shader is not applied
either, and the window keeps the effects it had.

The compositor's log says `an effect was refused` and gives the line and column of the source. Fix the source and write
again.

### More than four shaders

The library draws the first four and says so in the style warnings. A shell that writes the effects itself has the fifth
refused as too large.

### The renderer does not draw shaders

The compositor accepts the shader and keeps it on the node. The node's `skipped` list says
`shader: unsupported-by-renderer`, and the client's style warnings repeat it:

```text
the compositor does not draw the effect shader: unsupported-by-renderer
```

The GLES and Vulkan renderers draw shaders. The software renderer has no shader compiler, so it never does. A session with a GPU renderer that draws them
lists `shader` in `effect_kinds`. Results are not bit-identical between renderers: precision and filtering differ a little,
so compare colours with a tolerance.

### The shader is too slow

The compositor times each shader source in every frame it draws. A source that takes more than 50 ms in three frames in a
row is switched off for ten seconds: the window is drawn without it, the node's `skipped` list says `shader: over-budget`,
and the compositor logs a warning with the source's hash. It comes back, at the window's next repaint, when the ten
seconds are up. A source that is switched off three times, or whose frame was in flight when the graphics device was lost
three times, is refused for the rest of the session: `skipped` says `shader: refused-for-session`, and writing the same
source again is refused `bad-value` before it is validated. A different source is not affected. Make the shader cheaper
(fewer texture reads, a smaller loop, fewer `backdrop` calls) and write it again; a changed source has a new hash.

A renderer that cannot build a program for a valid source says why instead: `shader: <reason>`. That is not a strike.

Read the warnings of a client with `client.styleWarnings` in TypeScript and `Client::style_warnings` in Rust.

## Related pages

- [Window style](/js-style): the style scopes, filters, borders and shadows.
- [Window effects](/guides/window_effects): which effects each renderer draws.
