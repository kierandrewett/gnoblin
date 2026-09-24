# Write a frame renderer

[Configuration reference](configuration-reference.md#frames)

A renderer draws server-side decorations (SSD). Gnoblin handles geometry,
window actions and input. Read the [architecture](window-frame-renderers.md)
for ownership and failure behavior.

## Register your executable

Add this to `~/.config/gnoblin/init.lua` after its existing includes. It starts
your renderer and uses it for apps that request a server-drawn frame:

```lua
gnoblin.configure {
    frame_renderers = {my_frame = {"/absolute/path/my-frame"}},
}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {
        mode = "auto",
        renderer = "my_frame",
        extents = {48, 1, 1, 1},
    },
}
```

Replace the executable path, then run `gnoblinctl config reload`. The rule
adds a 48-pixel titlebar and one-pixel edges without removing existing rules.

## Implement the protocol

Generate bindings from [the protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/window-frame/gnoblin-window-frame-v1.xml).
Gnoblin starts the configured command with a private Wayland connection.
`WAYLAND_SOCKET` contains its file descriptor.
Connect once; do not give that descriptor to a second toolkit display connection.

1. Bind `gnoblin_window_frame_manager_v1`, `wl_compositor` and your buffer facility.
2. For each `frame` event, create a new `wl_surface` with no assigned role and call `attach_surface`.
3. On `configure`, draw the supplied outer size. Extents are top/right/bottom/left
   logical pixels. `state` includes focus, maximized and allowed-action flags.
4. Acknowledge its serial, clear/redeclare hit regions, attach a matching buffer,
   damage and commit. Keep buffers alive until `wl_buffer.release`.
5. On `interaction`, redraw hover/pressed appearance. On `closed`, destroy the
   frame handle and surface and release your per-frame resources.

## Input regions

Gnoblin excludes application content, clips the outer radius, checks permissions
and owns move/resize grabs. Renderer regions are last-defined-wins and commit
atomically with pixels. Use actual button bounds.

The native resize perimeter takes priority, even with zero painted side/bottom
extents. It supplies directional cursors and respects non-resizable windows.

| Action                     | Number |
| -------------------------- | ------ |
| Drag                       | 1      |
| Close                      | 2      |
| Maximise/restore           | 3      |
| Minimise                   | 4      |
| Resize N/NE/E/SE/S/SW/W/NW | 5–12   |

Interaction action 0 means no hovered region. Keyboard focus stays with the app.

Never attach an existing toolkit xdg-toplevel. A surface cannot have two roles.
Slow or dead renderers do not block application commits: an opted-in window gets
the native fallback until a matching buffer is ready.

## Small C helper

[client.c](https://github.com/kierandrewett/gnoblin/blob/main/src/tools/frame-renderer/client.c) owns the connection, bounded
buffers, configure handling and lifetime. Implement the functions in
[paint.h](https://github.com/kierandrewett/gnoblin/blob/main/src/tools/frame-renderer/paint.h); link the helper and generated
protocol code into your executable. No GNOME or JavaScript dependency is needed.

### Required hooks

- `frame_paint_init`: initialize your drawing library.
- `frame_paint(pixels, model)`: paint premultiplied ARGB8888, stride `width * 4`.
  The model supplies dimensions, extents, title, style, state, hover and pressed.

### Optional hooks

- `frame_paint_create/destroy`: one private `model.view` per window.
- `frame_paint_regions`: emit your button bounds; return 1 to replace the helper's
  three standard button regions. Drag and edge-resize strips are supplied already.
- `frame_paint_dispatch/timeout`: pump an offscreen toolkit and request repaint on
  theme changes. Simple renderers need neither hook.
- `frame_paint_needs_redraw(view)`: select which views need repaint after dispatch;
  defaults to all views. GTK uses its frame clock's after-paint signal.
- `frame_paint_poll(fds, count, timeout)`: include a toolkit's sources and deadlines
  in the transport poll; defaults to `poll()`. A GTK adapter must complete the
  main-context prepare/query/poll/check/dispatch cycle so its separate Wayland
  read is completed or cancelled correctly.

## Examples and testing

See `scripts/build-frame-renderers.sh` and `paint-cairo.c` for a compilable example.
Bingux's `packages/bingux-frame/paint-gtk.c` is the actual-widget adapter. It vendors
the MIT helper/XML snapshot for standalone packaging. Other languages need only
the XML, not this C helper. The protocol is experimental: pin its version and
test startup, transitions, input, late buffers, failure and shutdown.
