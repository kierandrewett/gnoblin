# JavaScript UI toolkit

The JavaScript shell draws its own interface with a small toolkit. Import it from `gnoblin`:

```ts
import { Box, Text, defineConfig } from "gnoblin";
```

Every widget is an ordinary class with ordinary fields. Change a field and the next frame paints it. There is nothing to
register and no property to declare.

## Canvas

A `Canvas` is a node of the compositor's tree that shows a paint list. Override `draw()` to return the ops to paint, in
node-local pixels, later ops over earlier ones:

```ts
import { Canvas } from "gnoblin";

class Meter extends Canvas {
    level = 0.5;

    draw() {
        return [
            { op: "rect", x: 0, y: 0, width: this.width, height: this.height, radius: 4, fill: "#1e1e2e" },
            { op: "rect", x: 0, y: 0, width: this.width * this.level, height: this.height, radius: 4, fill: "#89b4fa" },
        ];
    }
}

const meter = new Meter({ width: 120, height: 8 });
```

The ops are:

- `rect`: `x`, `y`, `width`, `height`, and optionally `radius`, `fill`, `stroke` and `strokeWidth`.
- `line`: `x1`, `y1`, `x2`, `y2`, and optionally `width` and `color`.
- `text`: `text`, and optionally `x`, `y`, `size`, `color` and `font`.
- `icon`: `name`, and optionally `x`, `y`, `size` and `color`.
- `pushClip` with `x`, `y`, `width`, `height` and optionally `radius`, closed by `popClip`.
- `pushOpacity` with `alpha`, closed by `popOpacity`.

A colour is `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.

The toolkit encodes the list into the node's `content` property, the same bytes the Lua shell writes, and writes it only
when it changed. An op that cannot be encoded is reported as `draw op N: ...` and the node keeps its last content. The
format is described in [Canvas painting](/shell-api/canvas).

A canvas has these fields and methods:

- `x`, `y`, `width`, `height`, `visible` and `opacity`.
- `parent` and `children`, the tree of canvases.
- `add(child)` puts a canvas under this one. `mount(node)` puts it under a node that is not a canvas.
- `destroy()` ends it and everything beneath it.
- `refresh()` runs once a frame before the children are arranged, for a canvas that reads live state.
- `arrange()` places the children.

## Box, Text, Icon and Grid

A `Box` is a canvas that fills itself (`fill`, `radius`) and arranges its child boxes:

- `direction` is `"row"`, `"column"` or `"none"`. A row or column is a small flexbox. `"none"` leaves the children
  where they are.
- `padding` is the space around the children and `gap` the space between them.
- `grow` on a child takes its share of what is left. Every child is stretched across the other axis.

```ts
const bar = new Box({ width: 300, height: 32, direction: "row", gap: 8, padding: 6, fill: "#1e1e2e" });
bar.add(new Text({ text: "Files", grow: 1 }));
bar.add(new Icon({ icon: "window-close-symbolic" }));
```

The other widgets are boxes too:

- `Text` draws one string. Its size is what the compositor measures for its font, size and text. The answer arrives a
  frame later, and until then the size is an estimate. With `ellipsize` a text wider than its node ends in an ellipsis.
- `Icon` draws a named icon, square at its `size`.
- `Grid` puts its children in a near-square grid of equal cells.

Give a node a `width` or `height` and that is what it has. Set one to `null` to go back to the natural size.

Pointer events are handled by methods on a canvas:

- `onPress`, `onRelease`, `onMotion`, `onScroll`, `onEnter` and `onLeave` take the event.
- An event goes to the canvas the pointer is over, then up through its parents until a handler calls `event.stop()`.
- The event's `x` and `y` are in the coordinates of the canvas whose handler runs.

## Titlebar

With `defineConfig({ decorations: true })` every window gets a `Titlebar`, a canvas under the window's node. The stock
`defaultConfig` turns it on. A config without the key has no titlebars.

The bar is a `Box` row: a drag area that holds the title, then the minimise, maximise and close buttons. It follows its
window:

- Its width is the window's width, and it is hidden while the window is fullscreen.
- Its fill follows the focus and its title follows the window's title.
- The close button closes the window. The other buttons set the window's `minimised` and `maximised`.
- A press on the bar drags the window. A second press within half a second maximises or restores it.

Its look is plain data, `TitlebarStyle`:

```ts
import { Titlebar, defaultTitlebarStyle } from "gnoblin";

const style = { ...defaultTitlebarStyle, height: 36, buttons: ["close"], focusedFill: "#223344" };
const bar = new Titlebar(client, style);
bar.mount(windowNode);
```

The style has these fields:

- `height`, `padding`, `gap` and `radius`.
- `focusedFill` and `unfocusedFill`.
- `titleSize`, `titleColor` and `titleFont`.
- `buttons`, in order, and `buttonSize`, `buttonColor` and `icons`.

The titlebar is drawn over the top of the window: it does not reserve room above the client.

## What is not done

- The right and middle click gestures of the Lua titlebar, and the fade on close.
- Dragging with a real pointer. It moves the window by how far the pointer is from where it was pressed, and has been
  tested against a fake compositor only.
- The `gnoblin.d.ts` declarations do not describe the toolkit yet.
