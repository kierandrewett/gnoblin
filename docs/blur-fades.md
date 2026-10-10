# Blur during client animations

Blur follows the alpha of the surface it sits behind. When a panel fades out by
committing translucent frames, the blur behind it fades at the same time, and
no blur is left behind once the panel is gone. Shells need no protocol or
extra code for this.

Blur reaches full strength once the surface behind it is about 80% opaque, so a
translucent panel keeps its full blur while it is visible. A surface that
animates with a [layer animation](/guides/animations#layer-shell-surfaces) fades
its blur with the window.

## Blur-fade metadata

`gnoblin_blur_fade_manager_v1` is still accepted so existing shells keep
running, but Gnoblin no longer reads the rectangles or opacities it receives.
Its requests and fields are defined in the [protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/blur-fade/gnoblin-blur-fade-v1.xml).
It is optional and unavailable outside Gnoblin mode.

## Foreground details

Icons and separators can be more opaque than the panel. Their alpha must not
create gaps in the backdrop.

The fallback mask only estimates adjacent material at a silhouette edge with
matching colour. Explicit [standard regions](background-effects.md) give the
client direct control of that shape.

## Verify a change

Use the [devkit](/devkit) to inspect fades with the shell client that owns the
affected layer surface. `make` builds the standalone session and `make test` runs
the native runtime checks; neither measures a shell client's rendered fade.
See [testing](/testing) for the available checks and their limits.
