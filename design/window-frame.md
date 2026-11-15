# Window frame: a surface with outlets

This is the owner's request, not part of the zip spec (`design/js-spec-native.md`). The spec has `Decorations` as a
library listener that mounts a `Titlebar`; it has no frame object. The owner asked for "an outlet sort of api" for the
window frame. It adds to the spec and changes none of the spec's names.

## What exists

- The compositor draws no frame. A window's node is its content. A server-decorated window gets canvas children made by
  the shell: today one `Titlebar` at `y = -height`. The scene already counts decoration children outside the node in
  the frame's shape (rounded corners, border ring, frame shadow) and hit-tests every child on its own, outside the
  parent's rectangle too.
- The layout driver knows one number, `client.frameInset` (the bar's height), and honours it on the top side only.
- Moving is the titlebar's own drag. Resizing is `Super+MouseRight+Drag` and the client's `resize` request: a
  server-decorated window has no edge handles.

## Decisions

1. **A `WindowFrame` per window, made by `Decorations`.** The `frame(client, frame)` option fills it. A callback that
   returns `false` gives the window no frame, and `rules` pick a callback by app id or title. `startDecorations(options)`
   passes the options through. Nothing changes for a file that passes none: the stock fill is the titlebar.
2. **Outlets are lazy `Box` nodes.** `frame.top`, `bottom`, `left` and `right` (edges, a row or a column), `topLeft`,
   `topRight`, `bottomLeft` and `bottomRight` (corners), `underlay` and `overlay` (layers). Reading one makes its node
   under the window. A frame that is not touched has no node. An outlet is a `Box`, so `fill`, `padding`, `gap`,
   `radius`, `grow` and the pointer handlers work on it, and `add` takes any `Canvas` or scene `Node` (a `View`).
3. **Titlebar sub-outlets** are `frame.start`, `frame.center` and `frame.end`: the three zones of the top edge.
   Reading one makes all three (a row: `start` and `end` take their content's size, `center` the rest).
4. **The content outlet is not fillable.** `frame.content` has the content rectangle and the inset. The client's
   surface sits in it.
5. **Insets are one rectangle.** `FrameInset` is `{ top, right, bottom, left }`. An edge outlet's thickness is its
   `size` or, left `null`, its thickest child. `frame.inset` is the four thicknesses, all zero while the window is
   fullscreen, and an outlet can be hidden by `hideWhen(client)` (for example `(c) => c.maximised`).
   `client.frameInset` is that rectangle, and the layout driver, floating placement, drags and snaps all use it.
   A tile is a frame: the content is the tile less the inset on every side. `frame.reserved` ignores fullscreen: the room
   a window keeps while fullscreen hides its frame.
6. **Outlets are placed by the frame** from the client's size and the insets, in the window's coordinates: `top` and
   `bottom` span the whole frame width, `left` and `right` the content height. An edge outlet's `outset` extends it
   outward past the frame with no inset, which is where an invisible resize handle sits. Corners sit over the edges, `size`
   square. `underlay` and `overlay` cover the frame and `bleed` pixels more (glow, badges).
7. **Layers need a native property.** A canvas has `placement`: `frame` (default: in front, part of the frame's shape),
   `over` (in front, not in the shape) and `under` (painted behind the window's content, not in the shape). The scene
   already paints a `behind` child first; the property makes it reachable and keeps a glow from widening the rounded
   frame, the border ring and the shadow. Input is not a compositor change: a child outside the geometry is hit already
   (proved by a test with a real client).
8. **A client-drawn window gets no edge outlets and no inset** (the double title bar rule is kept). `frame.kind` is
   `layers` for it, and for `none`: `underlay` and `overlay` work, edge and corner outlets throw. The `frame` callback is
   called for every window, so a shadow on a client-drawn window is a callback that fills `underlay` and returns.
9. **Parts, not knobs.** `TitleText`, `WindowButton`, `WindowButtons`, `AppIcon`, `DragArea`, `ResizeHandle`,
   `ResizeHandles`, `FrameBorder`, `FrameBackdrop` and `FrameGlow` are ordinary nodes taking a context (`frame`). They
   put anywhere, in any order. The shared window behaviour (close fade, maximise, minimise, the seat's click actions)
   is `frame.actions`, so a custom node calls `frame.actions.close()`.
10. **Reactive by the existing mechanisms.** A part reads the client in `refresh()` once a frame, as the toolkit does, so
    it follows any client, tracked or not. A canvas gets `properties` (the binding engine nodes and clients have), so
    `badge.properties.bind("visible", () => !client.focused)` updates without the script running again.
11. **The stock titlebar is built from the parts.** `Titlebar` stays: a `Box` holding `TitleText` and `WindowButton`s under
    the drag behaviour, with `TitlebarStyle`. `Decorations`' default fill puts one in `frame.top`. Its nodes, its numbers
    and its painted output are the ones it has now.
12. **`ResizeEdge` is taken** (`bindings.ts` already exports that type), so the part is `ResizeHandle`. Its drag is the
    resize arithmetic `ClientResizeBinding` uses, now a function (`resizedRect`). The stock fill adds no handles: opt in with
    `startDecorations({ resizeHandles: 6 })` or `frame.resizeHandles(6)`.

## Not in the window object model

- `urgent` and `icon` are not properties of a window node (no activation or icon protocol is wired), so there is nothing
  for a part to follow. `AppIcon` draws a letter tile from `appId`, because the painter has no icon theme. Neither is
  invented here.
- No cursor shape per handle: the compositor takes a cursor shape from clients, not from shell nodes.

## Lifetime

A frame mounts when the window is `server` (or always, as `layers`), is rebuilt when `decoration` changes, and goes
with the window (the compositor destroys its nodes; the toolkit forgets them). A reload ends the generation's canvases
(`uiReleaseAll`) and the frame hooks, and the next generation's `Decorations` frames every window again. `frame.destroy()`
is for a script that wants to take a frame away: it destroys the outlet nodes and ends the frame's hook.

## Checks

- `tests/ui.rs`, `stock_ui.rs`: the stock titlebar's nodes and painted ops are unchanged; the three worked examples; insets
  on all four sides in floating, tiled, snapped, maximised and fullscreen placement; teardown and reload leave no node.
- `decorated_window_tests.rs`: `over` and `under` canvases stay out of the frame's shape; an `under` canvas paints first.
- `tests/window_pointer.rs`: a real client, a handle canvas outside the geometry gets the pointer, the client does not.
- Software-renderer screenshots of the three examples.
