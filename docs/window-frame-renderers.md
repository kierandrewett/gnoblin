# Language-independent SSD renderers

Implemented, experimental v1. Rendering and standard window controls are native;
the Shell JavaScript module only translates configuration. Neither GJS nor Node
is needed to implement a renderer. Private tests do not install into the desktop.

Nothing draws by default: `frame.mode = "off"` is the native and Lua default.
The built-in opt-in fallback draws its own vector icons without an icon-theme or
GTK dependency. Bingux's styled renderer renders actual libadwaita widgets.
Start with the [renderer author quickstart](frame-renderer-api.md).

## Ownership and protocol

Gnoblin owns xdg-decoration negotiation, committed crop/extents, native window
attachment, input/grabs, content exclusion and the final rounded mask. An external
process owns appearance. One process can render many frames.

The compositor launches a configured argv using its socketpair-backed
MetaWaylandClient facility. Only that exact connection sees the private global.
There is no target-window-ID claim operation. This restricts compositor
capabilities; it is not an OS sandbox for the configured executable.

Source of truth: [gnoblin-window-frame-v1.xml](../src/protocols/window-frame/gnoblin-window-frame-v1.xml).

The manager creates frame handles. Each receives title, app ID, state and allowed
actions, style name, logical dimensions, committed extents and a configure serial.
The renderer attaches a previously unroled wl_surface, acknowledges configure,
sets semantic regions, then submits ordinary Wayland buffers/damage. A normal
toolkit xdg-toplevel cannot also take this role.

Acknowledgements and regions are snapshotted with surface commits, including
merged transactions. Regions are double-buffered, bounded to 64, last-defined
wins. Actions are drag, close, maximize/restore, minimize and eight resize
directions. Native code validates allowed actions and starts grabs from the
actual input event. Hover/press events allow renderer visuals; the renderer
never takes keyboard focus. Arbitrary command requests are not part of v1.

A compositor mask removes pixels inside the client body even when a renderer
paints an opaque full-window buffer. Native reactive strips also exclude the
body. Native radii restrict frame hit testing. Existing border/shadow rules
remain separate; no window blur is introduced.

## Failure and lifetime

Application commits never wait for a renderer. Geometry changes immediately use
native fallback until a matching buffer arrives; stale generations cannot replace
a newer layout. Invalid dimensions/serials disconnect the renderer. The fallback
keeps the same committed extents and native controls.

On process exit the connection is destroyed and fallback appears. Restart uses
bounded exponential backoff, at most four launch attempts per session. A configure
timeout retains fallback; there is no continuous process heartbeat. A stalled
renderer with valid current pixels may keep them, but native controls remain
usable. New geometry selects fallback. Compositor shutdown cancels retries and
terminates only its renderer children.

The reference transport limits each frame to two outstanding shared-memory
buffers, coalescing newer model/theme changes. It uses a full-size canvas rather
than four strips; high-DPI/many-window memory and GPU costs need measurement.

## Non-JS adapters

Run `scripts/build-frame-renderers.sh` to build:

- `build/frame-renderers/gnoblin-frame-cairo`: C, Cairo/Pango.
- `build/frame-renderers/gnoblin-frame-qt`: C++/Qt QImage and QPainter, offscreen.

Both use [client.c](../src/tools/frame-renderer/client.c) for transport and
[paint.h](../src/tools/frame-renderer/paint.h) as their painting boundary. They
accept `--theme-file=/absolute/path`; a six-digit hex background color reloads via
inotify. Invalid edits retain the last valid color. Their button layout is fixed;
custom layouts belong in the renderer. Native fallback independently supports
the Lua button-layout setting.

Other languages can generate bindings and submit their own buffers. Slint, GTK,
Rust, GPU engines, optional QML or web renderers need an adapter; unmodified
toolkit windows are not attachable. Subsurface trees, renderer-proposed extents
and arbitrary action requests are not implemented in v1.

## Configuration and verification

See [window-frames.md](window-frames.md) for named argv services and rules.
Service definitions reconcile live through `gnoblinctl config reload` and watched
config edits. Both restart renderer executables even when their paths are
unchanged; there is no separate SSD reload command. Native fallback keeps window controls available
during replacement. Compositor code and protocol enablement still need a new session.

Private tests cover native/Cairo/Qt pixels, window capture after transitions,
negotiated SSD and explicit crop, client click mapping, fullscreen, standard
controls and theme reload. Failure tests exercise capability restriction,
stalled renderers, native fallback and process restart. Spotify has a separate
optional private-profile test. Mixed-scale moves, popup-heavy cropped apps,
overview, adversarial protocol fuzzing and GPU-buffer adapters need more coverage;
v1 is not claimed stable or universally toolkit-compatible.
