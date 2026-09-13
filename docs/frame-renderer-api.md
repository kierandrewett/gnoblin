# Writing an SSD renderer

There are two independent choices: **whether a window has SSD**, and **who draws
it**. Gnoblin defaults to no SSD. The renderer never changes application geometry
or executes window actions itself.

```lua
return {
    ["frame-renderers"] = { my_frame = { "/absolute/path/my-frame" } },
    ["window-rules"] = {
        { match = { ["app-id"] = "^spotify$" },
          frame = { mode = "prefer-server", renderer = "my_frame",
                    extents = { 48, 1, 1, 1 } } },
    },
}
```

`gnoblinctl config reload` and watched config edits reload SSD renderers too,
including executables rebuilt at unchanged paths. There is no separate SSD reload
command. Changed or removed services retire safely. Windows stay open with native fallback
until the new renderer presents. Invalid configuration keeps the old registry.
Enabling the protocol itself, or upgrading compositor code, still needs a session
restart. `mode = "off"` removes SSD; `renderer = "native"` selects
the small built-in renderer only after a mode has opted in.

## Any language

Generate bindings from [the protocol XML](../src/protocols/window-frame/gnoblin-window-frame-v1.xml).
Gnoblin starts your argv with a private Wayland connection in `WAYLAND_SOCKET`.
Connect once; do not give that descriptor to a second toolkit display connection.

1. Bind `gnoblin_window_frame_manager_v1`, `wl_compositor` and your buffer facility.
2. For each `frame` event, create a fresh unroled surface and call `attach_surface`.
3. On `configure`, draw the supplied outer size. Extents are top/right/bottom/left
   logical pixels. `state` includes focus, maximized and allowed-action flags.
4. Acknowledge its serial, clear/redeclare hit regions, attach a matching buffer,
   damage and commit. Keep buffers alive until `wl_buffer.release`.
5. On `interaction`, redraw hover/pressed appearance. On `closed`, destroy the
   frame handle and surface and release your per-frame resources.

The compositor enforces a hole for application content, clips the outer radius,
checks action permissions and owns drag/resize grabs. Regions are last-defined
wins and commit atomically with pixels. Action numbers: drag=1, close=2,
maximize/restore=3, minimize=4, resize N/NE/E/SE/S/SW/W/NW=5..12.
Interaction action 0 means no region is hovered. No keyboard focus is transferred.
Supply real button bounds, not approximate hit rectangles.

Never attach an existing toolkit xdg-toplevel. A surface cannot have two roles.
Slow or dead renderers do not block application commits: an opted-in window gets
the native fallback until a matching buffer is ready.

## Small C helper

[client.c](../src/tools/frame-renderer/client.c) owns the connection, bounded
buffers, configure handling and lifetime. Implement the functions in
[paint.h](../src/tools/frame-renderer/paint.h); link the helper and generated
protocol code into your executable. No GNOME or JavaScript dependency is needed.

Required hooks:

- `frame_paint_init`: initialize your drawing library.
- `frame_paint(pixels, model)`: paint premultiplied ARGB8888, stride `width * 4`.
  The model supplies dimensions, extents, title, style, state, hover and pressed.

Optional hooks have defaults:

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

See `scripts/build-frame-renderers.sh` and `paint-cairo.c` for a compilable example.
Bingux's `packages/bingux-frame/paint-gtk.c` is the actual-widget adapter. It vendors
the MIT helper/XML snapshot for standalone packaging. Other languages need only
the XML, not this C helper. The protocol is experimental: pin its version and
test startup, transitions, input, late buffers, failure and shutdown.
