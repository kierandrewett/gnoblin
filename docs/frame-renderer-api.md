# Write a frame renderer

[Configuration reference](/config/configure/frame_renderers)

A renderer draws server-side decorations (SSD). Gnoblin handles geometry,
window actions and input. Set the first `frame_renderers` array item to the
executable name; Gnoblin looks it up on the compositor's `PATH`. Put each
command-line argument in its own later array item. Gnoblin passes those strings
directly, without shell expansion. Read the
[architecture](window-frame-renderers.md) for ownership and failure behavior.

## Register your executable

Add this to `~/.config/gnoblin/init.lua` after its existing includes. It starts
your renderer and uses it for apps that request a server-drawn frame:

```lua
gnoblin.configure {
    frame_renderers = {cairo = {"gnoblin-frame-cairo"}},
}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {
        mode = "auto",
        renderer = "cairo",
        extents = {48, 1, 1, 1},
    },
}
```

Install `gnoblin-frame-cairo` so it is available on the compositor's `PATH`,
then run `gnoblinctl config reload`. The rule adds a 48-pixel titlebar and
one-pixel edges without removing existing rules. Run
`command -v gnoblin-frame-cairo` in a terminal to find the executable provided
by your installation.

## Implement the protocol

Generate bindings from [the protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/window-frame/gnoblin-window-frame-v1.xml).
Gnoblin starts the configured command with a private Wayland connection.
`WAYLAND_SOCKET` contains its file descriptor.
Connect once; do not give that descriptor to a second toolkit display connection.

The compositor offers a `frame` object when a window needs a frame. It sends
dimensions and metadata in a `configure` event:

```text
configure(serial, width, height, top, right, bottom, left,
          state, title, app_id, style)
```

### Configure event fields

| Field                            | Meaning                                                                                                 |
| -------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `serial`                         | Identifier for this configuration; acknowledge it before committing its buffer.                         |
| `width`, `height`                | Required dimensions for the matching frame buffer.                                                      |
| `top`, `right`, `bottom`, `left` | Frame extents for this window, in that order.                                                           |
| `state`                          | Bit field for focus, maximize state, and available window actions. See the [state flags](#state-flags). |
| `title`, `app_id`                | Window metadata supplied by Gnoblin.                                                                    |
| `style`                          | Style label for the renderer; interpret it according to your renderer's theme support.                  |

Bind `gnoblin_window_frame_manager_v1`, `wl_compositor` and a buffer facility.
For each `frame` event, create a fresh roleless `wl_surface` and attach it with
`attach_surface(surface)`. Never reuse an app's `xdg_toplevel` surface: a
Wayland surface can have only one role.

### JS-like lifecycle pseudocode

This illustrates the request order in language-neutral pseudocode. Replace
`on`, `createSurface`, `createArgb8888Buffer` and drawing calls with your
Wayland binding's APIs. The names in backticks are the protocol requests.

```javascript
const ACTION_CLOSE = 2;

manager.on("frame", (frame) => {
    const surface = compositor.createSurface(); // no role assigned yet
    let config = null;
    let hover = 0;
    let pressed = false;

    frame.attach_surface(surface);

    function drawAndCommit() {
        if (!config) return;

        const buffer = shm.createArgb8888Buffer(config.width, config.height);
        drawFrame(buffer, {
            title: config.title,
            appId: config.app_id,
            style: config.style,
            extents: [config.top, config.right, config.bottom, config.left],
            state: config.state,
            hover,
            pressed,
        });

        frame.ack_configure(config.serial);
        frame.clear_regions();
        const close = closeButtonBounds(config); // use your renderer's layout
        frame.region(ACTION_CLOSE, close.x, close.y, close.width, close.height);
        surface.attach(buffer, 0, 0);
        surface.damage(0, 0, config.width, config.height);
        surface.commit();
    }

    frame.on("configure", (next) => {
        config = next;
        drawAndCommit();
    });

    frame.on("interaction", (event) => {
        hover = event.action; // 0 means no region is hovered
        pressed = event.pressed !== 0;
        drawAndCommit();
    });

    frame.on("closed", () => {
        frame.destroy();
        surface.destroy();
    });
});
```

The buffer must match the latest configure's width and height. Keep it alive
until `wl_buffer.release`. Acknowledge the latest serial before committing its
buffer. Gnoblin presents only buffers for the current configuration; it ignores
stale generations.

### State flags {#state-flags}

The `state` value is a bit field. Test each flag with a bitwise AND:

| Flag         | Value |
| ------------ | ----: |
| Focused      |   `1` |
| Maximized    |   `2` |
| Can close    |   `4` |
| Can maximize |   `8` |
| Can minimize |  `16` |
| Can resize   |  `32` |

## Input regions

`region(action, x, y, width, height)` declares a hit area in logical frame
coordinates. Clear and redeclare regions for each commit; the compositor applies
them atomically with the pixels. At most 64 regions are allowed, and the last
registered region wins when areas overlap. Use actual button bounds.

Gnoblin excludes application content, clips the outer radius, checks permissions
and owns move/resize grabs.

The native resize perimeter takes priority, even with zero painted side/bottom
extents. It supplies directional cursors and respects non-resizable windows.

| Action                     | Number |
| -------------------------- | ------ |
| Drag                       | 1      |
| Close                      | 2      |
| Maximise/restore           | 3      |
| Minimise                   | 4      |
| Resize N/NE/E/SE/S/SW/W/NW | 5–12   |

The protocol action values appear below. Keyboard focus stays with the app.
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
