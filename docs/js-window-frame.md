# The window frame

A window frame is the surface round one window's content. It has named places, called outlets, and you fill each one with
your own nodes. The stock titlebar is one way to fill it.

For short examples see [fill a window frame with your own parts](/recipes/window-frame).

## Make a frame

`startDecorations(options)` gives every window a `WindowFrame` and fills it. `Decorations` is the class behind it. The
options are all optional:

- `frame(client, frame)` fills the frame of each window that no rule picked. Left out, the stock titlebar fills it.
- `rules` is a list of `{ appId, title, frame }`. The first rule that matches a window decides. `appId` and `title` are a
  string or a `RegExp`, and `frame` is a callback or `false`.
- `style` is the `TitlebarStyle` of the stock titlebar.
- `resizeHandles` puts resize handles this many pixels outside the frame of every window the shell frames.

A callback that returns `false` gives that window no frame. It runs for every window, so it reads `frame.server` to tell
the two kinds of frame apart:

- `server` is a window the shell frames. It has all the outlets and takes room round its content.
- `layers` is a window that draws its own frame, as GTK and Qt windows do, and a window with no frame. It has the
  underlay and the overlay and takes no room. Reading an edge or a corner throws, because a second bar over the client's
  own is the double title bar.

Who frames a window is negotiated with the client, and `client.decoration` says the answer. See
[who draws the frame](/js-ui#who-draws-the-frame).

## Outlets

Reading an outlet makes its node under the window, so a frame that is not touched has no node.

An outlet is a `Box`. Its `fill`, `padding`, `gap`, `radius` and `grow` work, and `add(node)` puts a `Canvas` or a scene
`Node` in it.

| Outlet                                             | What it is                                             |
| -------------------------------------------------- | ------------------------------------------------------ |
| `top`, `bottom`                                    | An edge: a row as wide as the whole frame              |
| `left`, `right`                                    | An edge: a column as tall as the content               |
| `start`, `center`, `end`                           | The three zones of the top edge                        |
| `topLeft`, `topRight`, `bottomLeft`, `bottomRight` | A square over the corner, which takes no room          |
| `underlay`, `overlay`                              | A layer behind the window and one in front of it       |
| `content`                                          | The place of the client's surface, which is not filled |

Every outlet has these fields:

- `hidden` hides the outlet and gives its room back while it is true.
- `hideWhen(client)` does the same while it returns true, for example `(client) => client.maximised`.
- `clear()` destroys what was added.
- `onRefresh` runs once a frame before the outlet is placed.
- `onAdded(node)` runs when something is added.

An outlet is hidden while the window is fullscreen.

### Edges and zones

An edge takes room: the layouts leave `size` pixels between the content and the frame's outside. Left `null`, `size` is
the thickest node added to the edge, so give a node that measures itself later, such as a `Text`, an explicit `size`.

The three zones of the top edge are `start`, `center` and `end`. Reading one makes all three, in a row: `start` and `end`
are as wide as what they hold, and `center` takes the rest. Nodes added to the top edge itself sit beside the zones.

### Corners and layers

A corner is a square of `size` pixels over the edges that meet there. Left `null`, it is as large as the thicker of the two.
It takes no room.

A layer takes no room and, by default, no pointer. The underlay is painted behind the window's content and the overlay in
front of it. Both cover the whole frame, and `bleed` makes the layer reach that many pixels past the frame on every side:
a glow goes in the bleed of an underlay.

## Insets and layout

The room a frame takes is one rectangle, `FrameInset`: `{ top, right, bottom, left }`. These are the rules:

- `client.frameInset` is that rectangle. It is all zero for a window the shell does not frame, and while the window is
  fullscreen.
- `client.bounds` is the content. The frame is the content with the inset round it.
- A tile is a frame. A layout places the frame in its tile and the content is what the four edges leave.
- A snap zone is a part of the work area for the frame. The content is the zone less the inset on every side.
- Dragging a window keeps its whole frame in the work area, so no edge is lost off the screen.
- A frame keeps its place when it changes: the content moves in by what the top and left gained, and out by what they
  lost. A maximised window fills the area again, and a tiled one is placed again by its layout.
- `frame.reserved` is the inset the frame keeps for itself while fullscreen hides it.

`growRect`, `shrinkRect`, `sameInset` and `isNoInset` are the arithmetic, so your own layout can use the same rule.

## Parts

A part is an ordinary node that takes a frame and follows the window it is for. It reads the window once a frame, so it
follows a rename, a focus change or a maximise without the script running again. Parts go in any outlet, in any order.

| Part                        | What it does                                                                |
| --------------------------- | --------------------------------------------------------------------------- |
| `TitleText(frame)`          | The window's title on one line, ellipsized to its room                      |
| `WindowButton(frame, name)` | The `minimise`, `maximise` or `close` button as an icon                     |
| `WindowButtons(frame)`      | Several buttons in a row: `buttons`, `gap`, `icons` and the options of each |
| `AppIcon(frame)`            | A tile with the initial of the application id                               |
| `DragArea(frame)`           | A box that moves the window and answers the seat's click actions            |
| `ResizeHandle(frame, side)` | Resizes the window from a side or a corner when dragged                     |
| `ResizeHandles(frame)`      | The eight handles round the frame                                           |
| `FrameBorder(frame)`        | A ring on the frame's edge. `color` may be a function of the window         |
| `FrameBackdrop()`           | A fill as large as the frame, for the underlay                              |
| `FrameGlow()`               | A soft halo that fills the bleed of an underlay                             |

The behaviour every part shares is `frame.actions`. A node of your own calls it:

- `close()` fades the window out and closes it.
- `toggleMaximise()` and `minimise()` do what they say.
- `gesture(name)` runs the seat's `double_click`, `middle_click` or `right_click` action.

`AppIcon` draws a tile with a letter and a colour from the application id. The painter has no icon theme to draw the
application's own icon from.

### The stock titlebar

`Titlebar` is a `DragArea` holding a `TitleText` and the `WindowButton`s, with the look of a `TitlebarStyle`.

`titlebarFill(style)` is the frame callback that puts one in the top outlet of a window the shell frames, and returns
`false` for any other. It is the default `frame`. A callback of your own can call it, then add outlets to what it made.

### Resize handles

`frame.resizeHandles(thickness)` puts eight handles round the frame. Each reaches `thickness` pixels outside the frame
(6 by default), so it lies outside the window's geometry, and the pointer goes to the handle and not to the client.

- A drag moves the side or corner the handle holds and keeps the opposite one where it was.
- A press on a handle holds it until the button is released, so the drag goes on when the pointer leaves the thin handle.
- The window keeps the size the client states as its least and most, or the stock least.
- A handle is hidden while the window is maximised or fullscreen.

A stock frame has no handles: a window is resized with `Super+MouseRight+Drag` unless the frame adds them. The options
`inside` and `corner` set how far a handle reaches into the frame and how large a corner handle is.

A `ResizeHandle` paints nothing. Put one in an edge instead, as the [status strip recipe](/recipes/window-frame) does,
to give a handle room of its own that takes space from the window.

The compositor sets the cursor shape from clients, not from shell nodes, so a handle has no cursor of its own.

## Layers and the canvas `placement`

A canvas that is a child of a window has a native property `placement`:

- `frame` is the default. The canvas is in front of the content and part of the frame's shape: the rounded corners, the
  border ring and the shadow follow it.
- `over` is in front of the content and not part of the frame's shape.
- `under` is painted behind the content and not part of the frame's shape.

The overlay and the underlay write `over` and `under`, so a glow does not widen the rounded frame. See
[Canvas painting](/shell-api/canvas#placement).

## Mount, unmount and reload

A frame is mounted when the window is, and the compositor destroys its nodes with the window.

- A window whose `decoration` changes gets its frame made again: the callback runs again for the kind of frame it has now.
- A callback that throws, or returns `false`, leaves no frame and no room behind.
- `decorations.reframe(client)` decides again a rule that read the title.
- `frame.destroy()` takes a frame away: it destroys the outlets and ends the frame's hook.
- A reload ends the old generation's canvases and its hooks. The new generation's `startDecorations` frames every window
  again, and a window keeps its place: the frame's inset is kept across the reload and fitted to the new frame.

## What a window does not have

A window node has no `urgent` property and no `icon`, because no activation or icon protocol is wired to the shell. A part
cannot follow what is not there, and neither is invented here.
