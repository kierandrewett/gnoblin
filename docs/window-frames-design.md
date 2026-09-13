# Custom window frames and client cropping

Status: proposed design, based on the running Spotify client and the current
Gnoblin/Mutter source. This document does not enable SSD or introduce working
configuration keys.

## Result

Spotify should have a Gnoblin-owned title bar, controls, radius, borders and
shadow. Applications which negotiate server-side decorations (SSD) should stop
drawing their own frame. An explicit crop rule should support applications that
continue drawing client-side decorations (CSD). Both paths use the same frame
layout and renderer.

## Findings

- Gnoblin's current extra-protocol registration has no `xdg-decoration`
  implementation: `src/protocols/aggregator/meta-gnoblin-protocols.c`.
- The running Flatpak Spotify loads `/app/extra/share/spotify/libcef.so`.
  That library contains `zxdg_toplevel_decoration_v1` and a decoration-manager
  binding diagnostic. This establishes bundled support, not proof that Spotify
  will negotiate SSD successfully in this configuration.
- `gnoblinCorners.js` supplies clipping, CSD gap reconstruction, shadows and
  borders. Its `padding` changes effect bounds, not the managed window's size
  or input region. It is not a complete crop implementation.
- `meta_window_wayland_finish_move_resize()` now saves the original client
  geometry in native frame state before deriving the effective outer geometry.
  Existing `custom_frame_extents` then describes buffer-to-outer-frame margins;
  it is not the source of truth for either SSD extents or client CSD margins.
- Existing window actions and `meta_window_begin_grab_op()` supply close,
  minimize, maximize and interactive move/resize. Themes need not implement
  independent window-management loops.

The protocol specification requires decoration-mode changes to participate in
the xdg configure/acknowledge/commit sequence. Absence of negotiation leaves
clients responsible for their decorations. Start with protocol version 1;
advertise version 2 only after implementing its mapped-window lifecycle rules.
The locally installed protocol XML already includes version 2.

Source: `/usr/share/wayland-protocols/unstable/xdg-decoration/xdg-decoration-unstable-v1.xml`.
Upstream: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/unstable/xdg-decoration/xdg-decoration-unstable-v1.xml

## One frame module, two client paths

Introduce a native `WindowFrame` module with a small shell-facing interface:
set policy/layout, obtain committed frame state, subscribe to changes, and
invoke window actions. It owns decoration negotiation, crop state, geometry
conversion, input clipping and lifecycle. Its committed layout is the source
of truth for the shell renderer and existing effects.

1. Negotiated SSD: client supplies undecorated content; crop is zero.
2. Explicit CSD replacement: client retains its decorations; clip the configured
   margins, retain its original surface coordinates, and surround the visible
   content with the same SSD layout.

Keep client geometry, visible content geometry, and managed outer geometry
distinct. In logical coordinates:

```
visible size = client geometry size - crop margins
managed outer size = visible size + SSD extents
requested client size = requested outer size - SSD extents + crop margins
```

The content's buffer translation also includes client-provided shadow offsets.
An input point inside visible content maps back to its original surface
coordinates by adding the crop offset after removing the content's stage
origin. Use Mutter's surface transforms, rather than manually adjusting events
in a JavaScript pointer handler.

Painting and picking must share the crop. Cropped controls receive no input;
the hidden portion does not leave an invisible strip obstructing another
window. Preserve popup and subsurface coordinates relative to the original
client surface. Popups remain separate surfaces and must not inherit the
toplevel content clip accidentally.

Snapping, work-area constraints, maximizing, window placement, thumbnails and
window capture must consume the committed frame layout. Transform client
minimum/maximum sizes through the same geometry. Reject crops that remove all
content. Recompute logical-to-buffer conversion across output/scale changes.

Crop and SSD transitions are atomic with the acknowledged client commit.
Until then, keep the old layout and mode together. Do not expose a new outer
frame around an old, differently sized buffer.

## Rendering and customization

Use a compositor-owned actor subtree associated with each window, so its
decorations follow stacking, workspaces, minimization and overview animations.
The existing radius, border, shadow and CSD reconstruction effects should read
the shared frame layout rather than independently calculating three outlines.

Ship a default St-based title bar with configurable height, buttons/order,
alignment, font, colours, icons, active/inactive states and animations. Offer a
trusted GJS renderer factory for arbitrary layouts and custom Clutter drawing
or shader effects. This uses the same execution model as existing user scripts.
Do not promise browser CSS support: St supports its own CSS subset.

Proposed renderer interface:

```
create(context) -> { actor, update(model), destroy() }
```

`model` contains title, app identity, focus/state, allowed actions, committed
layout and theme settings. `context` supplies semantic actions and registration
of drag/resize/button regions. The frame module owns input grabs and validates
allowed actions; themes own appearance and layout. Button input is consumed
before drag handling. Changes in layout extents schedule native reconfiguration,
not a window resize from each paint callback.

Theme reload builds a replacement renderer before disposing of the previous
one. A failed renderer retains the last usable frame or the built-in fallback.
No polling and no separate process per window.

A future Quickshell/external-surface renderer can satisfy the same interface,
but needs surface attachment, input routing and renderer-failure handling. It
should not be required for the first working SSD implementation.

## Proposed configuration semantics

The GJS renderer proposal below is superseded by
[language-independent renderer services](window-frame-renderers.md).

These names are illustrative and are not accepted by the current parser:

```lua
{
    match = { ["app-id"] = "^com\\.spotify\\.Client(?:\\.desktop)?$" },
    frame = {
        mode = "prefer-server",
        renderer = "~/.config/gnoblin/frames/compact.js",
        height = 28,
        buttons = { "minimize", "maximize", "close" },
        fallback = {
            -- Explicit margins measured for the selected client/theme.
            -- Applied only while the client still draws CSD.
            crop = { top = 24, right = 4, bottom = 4, left = 4 },
        },
    },
}
```

The example margins are placeholders, not a verified Spotify crop. Cropping
must also work independently of a title bar. Define state-specific crop
profiles because client decorations can disappear or change while maximized,
tiled or fullscreen. Default fullscreen to zero crop and no SSD. A negotiated
SSD client never gets its CSD fallback crop applied as well.

Default negotiation respects a client's preference. `prefer-server` selects
SSD for a cooperating client; fallback cropping requires an explicit rule.
Do not automatically crop GNOME header bars: they can contain application
controls, tabs and search that a generic SSD cannot replace. CSD gap filling
continues to support those clients without removing their controls.

## Implementation sequence and acceptance

1. Add the native frame geometry module and an opt-in crop rule. Test actual
   paint/picking, popup placement, resize limits, scaling and state transitions
   with a client fixture whose cropped area contains a clickable control.
2. Add the compositor-owned default frame, reusing existing move/resize grabs
   and window actions. Verify title-bar drag, edge/corner resize, double-click
   maximize, buttons, focus, overview and lock/unlock lifecycle.
3. Implement `xdg-decoration` via the existing protocol aggregator, with
   pending/committed mode state tied to xdg configure serials. Exercise object
   destruction, invalid requests, mode changes and configure ordering using a
   protocol client fixture. Do not advertise SSD before the renderer works.
4. Test Spotify's bundled CEF negotiation in an isolated compositor. If it
   produces an undecorated surface, use it directly. Otherwise measure and test
   an explicit Spotify crop profile against its actual client frame.
5. Add the renderer factory and hot reload. Validate a second genuinely
   different layout, such as a vertical title strip, so custom layout extents
   are exercised rather than merely recolouring the default title bar.

Completion means Spotify has a working Gnoblin frame with no duplicate title
bar, dead strip or click-coordinate offset, both negotiated and fallback
fixtures pass, and disabling the rule restores the client's normal frame.
Native protocol and geometry changes require starting a new compositor;
subsequent theme/rule changes should be live.
