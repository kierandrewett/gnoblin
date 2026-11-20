# Window frame: a surface with outlets

This is the owner's request, not part of the zip specification (`js-spec-native.md (removed)`). The specification has
`Decorations` as a library listener that mounts a `Titlebar`, and it has no frame object. The owner asked for "an outlet
sort of api" for the window frame. The frame adds to the specification and changes none of its names.

The reference is `docs/js-window-frame.md` and the examples are `docs/recipes/window-frame.md`.

## What exists

- The compositor draws no frame. A window's node is its content, and a server-decorated window gets canvas children made
  by the shell: the stock `Titlebar`, at `y = -height`.
- The scene counts decoration children outside the node in the frame's shape: rounded corners, border ring and shadow.
- The scene hit-tests every child on its own, outside the parent's rectangle too.
- The layout driver knew one number, the bar's height, and honoured it on the top side only.
- A server-decorated window had no edge handles. It was resized with `Super+MouseRight+Drag` or by the client.

## Decisions

1. **A frame per window, made by `Decorations`.**
    - The `frame(client, frame)` option fills it. A callback that returns `false` gives the window no frame.
    - `rules` pick a callback by application id or title, and `startDecorations(options)` passes the options on.
    - The stock fill is the titlebar, so a file that passes none changes nothing.
2. **Outlets are lazy boxes.**
    - Edges: `top`, `bottom`, `left` and `right`. Corners: `topLeft`, `topRight`, `bottomLeft` and `bottomRight`.
    - Layers: `underlay` and `overlay`.
    - Reading an outlet makes its node. An outlet is a `Box`, and `add` takes a `Canvas` or a scene `Node`.
3. **The top edge has three zones**, `start`, `center` and `end`. Reading one makes all three, in a row.
4. **The content outlet is not fillable.** `frame.content` says where the client's surface is.
5. **Insets are one rectangle.**
    - `FrameInset` is `{ top, right, bottom, left }`, and `client.frameInset` is that rectangle.
    - An edge's thickness is its `size`, or its thickest child when `size` is `null`.
    - The inset is zero while the window is fullscreen. `frame.reserved` ignores fullscreen.
    - `hideWhen(client)` hides an outlet and gives its room back, as in `(c) => c.maximised`.
    - The layout driver, floating placement, drags and snaps all use the rectangle.
    - A tile is a frame: the content is the tile less the inset. A snap zone is a part of the work area for the frame.
6. **The frame places the outlets** from the client's size and the insets, in the window's coordinates.
    - `top` and `bottom` span the frame's width. `left` and `right` span the content's height.
    - A corner sits over the edges. A layer covers the frame and `bleed` pixels more.
    - A handle outside the frame is a `ResizeHandle` in the overlay: `frame.resizeHandles(thickness)`.
7. **Layers need a native property.**
    - A canvas has `placement`: `frame` (the default), `over` or `under`.
    - `frame` is in front and part of the frame's shape. `over` is in front and not part of it.
    - `under` is painted behind the content and is not part of the shape.
    - The scene already painted a `behind` child first. The property makes it reachable, and a glow no longer widens the
      rounded frame, the ring or the shadow.
    - Input needs no compositor change. A child outside the geometry is hit already, and a test with a real client proves it.
8. **A client-drawn window has no edge and no inset**, which keeps the double title bar rule.
    - `frame.kind` is `layers` for it, and the underlay and the overlay work. Edge and corner outlets throw.
    - The callback runs for every window, so a shadow on a client-drawn window fills the underlay and returns.
9. **Parts, not knobs.** `TitleText`, `WindowButton`, `WindowButtons`, `AppIcon`, `DragArea`, `ResizeHandle`,
   `ResizeHandles`, `FrameBorder`, `FrameBackdrop` and `FrameGlow` are nodes that take a frame. They go in any order.
10. **Shared window behaviour is `frame.actions`**: close with a fade, maximise, minimise and the seat's click actions.
11. **Reactive by the mechanisms that exist.**
    - A part reads the client in `refresh()` once a frame, as the toolkit does, so it follows any client.
    - A canvas has `properties`, so `badge.properties.bind("visible", () => !client.focused)` follows the focus.
12. **The stock titlebar is built from the parts.** Its nodes, numbers and painted output are what they were, one node
    deeper, in the top outlet.
13. **`ResizeEdge` is taken** by an export of `bindings/resize.ts`, so the part is `ResizeHandle`.
    - Its drag is the resize arithmetic of `ClientResizeBinding`, now the function `resizedRect`.
    - The stock fill adds no handles. A script opts in with `resizeHandles: 6` or `frame.resizeHandles(6)`.

## Not in the window object model

- `urgent` and `icon` are not properties of a window node, because no activation or icon protocol is wired. A part has
  nothing to follow, so neither is invented. `AppIcon` draws a letter tile from the application id.
- A handle has no cursor shape. The compositor takes a cursor shape from clients, not from shell nodes.

## Lifetime

- A frame mounts when the window is `server`, or always as `layers`. It is made again when `decoration` changes.
- The compositor destroys a frame's nodes with the window, and the toolkit forgets them.
- A reload ends the generation's canvases and its frame hooks. The next generation frames every window again.
- The inset a window was last fitted to is kept across the reload, so the frame keeps its place.
- `frame.destroy()` takes one frame away: it destroys the outlet nodes and ends the frame's hook.

## Checks

- `gnoblin-js/tests/window_frame.rs` covers the inset on all four sides in each kind of placement, the parts, the rules,
  a fill that throws, a `decoration` change and a reload.
- It also mounts the three examples of the recipes page. The tests read the code from the page.
- `gnoblin-js/tests/ui.rs` and `stock_ui.rs` keep the stock titlebar's nodes and painted ops.
- `gnoblin-backend` `decorated_window_tests.rs` checks that `over` and `under` stay out of the frame's shape, that an
  `under` canvas paints first, and, on the software renderer's pixels, that the backdrop shows through a rounded corner.
- `gnoblin-wayland/tests/window_pointer.rs` uses a real client: a handle outside the geometry gets the pointer and the
  client does not.
- `gnoblin/tests/js_shell.rs` runs the whole stack: a press outside the window's geometry lands on a handle, and the drag
  resizes the window and configures its client.
