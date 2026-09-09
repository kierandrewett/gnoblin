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
The alpha silhouette masks the blur; client transparency does not weaken it.
Rule opacity changes client pixels without mixing sharp background into the blur.
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
`shader`, `shader-uniforms`, and `corners`. A later uniforms table replaces the earlier
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

Blur uses the existing separable Gaussian filter, with its existing linear sampling
and radius-dependent downsampling. The effect retains its GPU textures, framebuffers
and filter between frames. It rebuilds them when dimensions or radius change.

Before each monitor paint, Gnoblin expands scene damage through overlapping blur
source rectangles. An unchanged backdrop keeps its filtered image. Damage outside
that rectangle does not recompute the blur. Damage inside it refreshes the complete
sample area, including the filter margin, so partial updates cannot leave sharp
patches. Changes to the foreground within the rectangle conservatively refresh it
too. A surface spanning monitors refreshes when switching views; it does not reuse
pixels captured from another monitor. There is no refresh timer.

Bingux supplies bounds for its fullscreen sidebar and notification surfaces through
the compositor bridge's `blur-regions` capability. Bounds use logical surface
coordinates and include visible shadows. The compositor adds the sampling margin;
client alpha still determines the visible silhouette. Hints apply only to layer
surfaces owned by the socket peer's process, identified by namespace and monitor
origin. Disconnecting removes the hints. Rules still decide whether blur is enabled.
Older native sessions retain the full-redraw fallback until the next login.

Compositor opacity fades the masked blur and client as one composed surface.
The mask compensates for drawing the foreground separately, so opacity is not
applied twice. This needs no client protocol or namespace-specific animation.
`scripts/test-blur-fade.py` compares eight fade and reversal samples against
an image of the completed surface blended over the unchanged background.

Client-rendered fades are different: if a client changes its buffer alpha, the
compositor cannot distinguish that from changing material transparency in one
frame. Such fades are not claimed to be fixed by the compositor-opacity change.
Bingux currently animates some popup opacity inside its buffers. The live mask
correction improves their settled blur, but their client-rendered fade still
needs a separate resolution. No alpha-modifier integration is installed.


Run native checks with the repo installation, without restarting the desktop:

```sh
GNOBLIN_PREFIX="$PWD/install" GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-blur-regions.py" scripts/run-gnome-shell.sh
GNOBLIN_PREFIX="$PWD/install" GNOBLIN_BLUR_REQUIRE_CACHE=1 GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-blur-performance.py" scripts/run-gnome-shell.sh
```

The region test compares cropped and full rendered pixels, moves a panel, changes
pixels behind it, and checks that unrelated animation does not recompute its blur.
The performance fixture reports paint time, process CPU ticks and resource builds
for a fixed five-second animation. It measures the isolated fixture, not desktop
CPU usage with applications running.

## Rounded window corners

Rounding is built into Gnoblin's window rules; no extension is required. For example:

```toml
[[window-rules]]
match = { type = "window" }
corners = { radius = 14, smoothing = 0.6 }

[[window-rules]]
match = { app-id = "^my-game$" }
corners = { mode = "off" }

[[window-rules]]
match = { type = "window", focused = false }
corners = { shadow = { blur = 24, opacity = 0.25 } }

[[window-rules]]
match = { type = "window", focused = true }
corners = { shadow = { blur = 28, opacity = 0.5 } }
```

Later rules merge individual corner settings, including individual shadow fields.
Use existing app-ID/title matchers for exceptions and `focused` for active/inactive
borders and shadows. Desktop surfaces, menus and override-redirect windows are
excluded. The setting has no effect until `radius` is greater than zero.

| Setting | Default | Meaning |
| --- | --- | --- |
| `radius` | `0` | Corner radius in logical pixels, 0–200; zero disables rounding. |
| `smoothing` | `0` | 0–1; circular through progressively smoother superellipse corners, using Reborn's parameterisation. |
| `mode` | `"auto"` | `auto` preserves existing transparent/rounded corners; `force` applies the requested mask; `off` disables it. |
| `padding` | `[0, 0, 0, 0]` | Top, right, bottom, left inset from the compositor's window frame, −128–128 logical pixels. |
| `border-width` | `0` | −40–40 pixels. Positive draws inside, negative outside; zero disables. Outset borders need space within the surface buffer, or an inset via `padding`. |
| `border-color` | `"#808080ff"` | `#RRGGBB` or `#RRGGBBAA`. |
| `keep-maximized`, `keep-fullscreen`, `keep-tiled` | `false` | Keep rounding in those window states. |
| `skip-libadwaita` | `true` | Preserve libadwaita corners in automatic mode. |
| `skip-libhandy` | `false` | Skip libhandy applications in automatic mode. |
| `shadow` | `false` | Optional table: `x`, `y`, `blur`, `spread`, `opacity`, `color`. |
| `shadow-animation` | `{ duration = 0, easing = "ease-out-cubic" }` | Fade between shadow styles. Duration: 0–2000 ms. Easing: `linear`, `ease-out-cubic`, `ease-out-quad`, `ease-in-out-cubic`. Respects reduced motion. |
| `keep-shadow` | `false` | Keep the replacement shadow in maximized, fullscreen or tiled states. |

## Inner and outer window borders

Gnoblin and Bingux leave compositor borders disabled by default. To opt into
one 1px grey inner stroke, use this window rule:

```toml
[[window-rules]]
match.type = "window"
borders = { inner-width = 1, inner-color = "#505050ff", outer-width = 0, outer-color = "#00000000", radius = 14, smoothing = 0.0 }
```

Edit `~/.config/gnoblin/gnoblin.toml`; valid changes reload automatically.
Colours use `#RRGGBB` or `#RRGGBBAA`, with alpha last (unlike QML).
Widths accept 0 to 40 logical pixels. Set both widths to zero to disable them.
Radius accepts 0 to 200, smoothing 0 to 1, and padding is
`[top, right, bottom, left]`, each from -128 to 128 logical pixels.

Borders do not clip the application, change its input region or reserve space.
The outer stroke extends beyond the frame. Both strokes follow the window actor
through animations. By default they disappear when maximized, fullscreen or
tiled. Set `keep-maximized`, `keep-fullscreen` or `keep-tiled` to true to retain
them. Later matching rules override only the border fields they specify.

Bingux requests real clipping with the same 14px circular shape as its borders:

```toml
[[window-rules]]
match.type = "window"
corners = { radius = 14, smoothing = 0.0, mode = "force", shadow = { blur = 24, spread = 0, opacity = 0.35 } }
borders = { inner-width = 1, inner-color = "#505050ff", outer-width = 0, outer-color = "#00000000", radius = 14, smoothing = 0.0 }
```

The replacement shadow also removes the original client shadow outside the
frame, which can otherwise leave a rectangular outline visible on wallpaper.
Keep radius, smoothing and padding equal for clipping and borders. Force mode
clips the client even when it already draws corners; automatic mode preserves
native corners and can leave a different curve underneath a configured border.

A rebuilt shell must be started once to load the native renderer. Subsequent
style changes need no logout. The current development session uses a temporary
user-script bridge until the native build is installed system-wide.


### Layered shadows

`corners.shadow` accepts its original table or a list of one to four tables.
Layers draw in list order, with each later layer over the earlier ones. Each
layer has `x`, `y`, `blur`, `spread`, `opacity` and `color`. Rendering uses one
actor and one shader pass for the whole stack. A later rule replaces an entire
list; single-table rules retain their existing field-merge behaviour.

Bingux uses a softer inactive shadow and this deeper focused-window treatment:

```toml
[[window-rules]]
match.type = "window"
match.focused = true
corners.shadow = [
    { x = 0, y = 10, blur = 36, spread = 0, opacity = 0.22 },
    { x = 0, y = 2, blur = 5, spread = 0, opacity = 0.28 },
]
```

The broad layer provides depth; the smaller layer defines the edge. These are
Bingux design values inspired by macOS, not Apple's private rendering values.
The current inner border uses `#505050bf` (approximately 75% alpha).


Shadow fades can be enabled independently of the shadow layers:

```toml
[[window-rules]]
match.type = "window"
corners.shadow-animation = { duration = 180, easing = "ease-out-cubic" }
```

The compositor crossfades complete shadow styles, including changes to blur,
position, colour and layer count. It retains only the latest pending style during
an active fade. Window close/minimise opacity also applies to the shadow.

Client frame geometry excludes shadow margins when the client reports them.
For clients that report the whole buffer as their frame, the compositor checks
three alpha scan lines on each axis. All three must agree on a sharp transition
to stable window content. It then uses that edge for clipping, both borders and
the replacement shadow. Uncertain edges retain the reported frame. Explicit
`padding` bypasses this detection. The result is shared and cached across focus,
move and resize events; state and scale changes trigger a new measurement.
Capture waits until the client has an image buffer. There is no per-frame scan.


`blur-ignore-shadows = true` excludes translucent black pixels from the backdrop
mask. Their original colour and opacity remain in the foreground pass. Bingux
uses black shadows and coloured panel tints, so this distinguishes its shadows
from even faint coloured glass. It is opt-in: translucent black glass is also
excluded, and coloured shadows are not detected. The default is `false`.
Native blur-mask changes take effect after starting the rebuilt compositor.

The backdrop can be downsampled for the Gaussian blur, but the client coverage
mask stays at output resolution. At antialiased edges, neighbouring alpha
samples estimate the material opacity separately from pixel coverage. This
keeps the blur inside rounded edges and preserves their coverage during fades.
No extra corner setting is needed for layer surfaces.
