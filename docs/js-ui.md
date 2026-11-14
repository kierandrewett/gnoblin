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

The toolkit encodes the list into the node's `content` property, and writes it only
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

A key goes to the canvas that holds the keyboard focus, then up through its parents, in `onKey(event)`:

- `canvas.focus()` gives a canvas the keyboard. It receives keys only while it holds it.
- The event has `key` (the keysym's name: `Return`, `a`), `text` (what the key types, empty for a key that types
  nothing), `pressed` (false for the release) and `stop()`.
- `canvas.mountTop()` puts a canvas over every window, as a menu or a dialog needs.
- `secure: true` keeps a canvas out of screen captures.

## Titlebar

With `defineConfig({ decorations: true })` a window that takes the shell's frame gets a `Titlebar`, a canvas under the
window's node. The stock `defaultConfig` turns it on. A config without the key has no titlebars. A window that draws its
own title bar gets none (see "Who draws the frame" below).

The bar is a `Box` row: a drag area that holds the title, then the minimise, maximise and close buttons. It follows its
window:

- Its width is the window's width, and it is hidden while the window is fullscreen.
- Its fill follows the focus and its title follows the window's title.
- The close button closes the window. The other buttons set the window's `minimised` and `maximised`.
- A press on the bar drags the window. A second press within half a second, a middle click and a right click do what
  the seat's `double_click`, `middle_click` and `right_click` say: `toggle-maximize`, `lower`, `menu`, `minimize` or
  `none` (the two one-axis maximise actions do nothing). `menu` opens the window menu at the pointer.
- The close button fades the window out over 0.18 s (it shrinks to 0.92 of its size) and then closes it. A window that is
  still there 1.5 s later, a client that asked "save changes?", is shown again. A window its client closes itself is gone
  from the compositor at once, so there is nothing to fade.

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

### Who draws the frame

A window has one title bar. Who draws it is negotiated with the client and published on the window as `decoration`:

- `server`: the shell draws it. The client asked for server-side decorations with `xdg-decoration` and the shell had
  declared that it draws them, or the window is an X11 one, which has no frame of its own.
- `client`: the client draws its own title bar and borders, as GTK, Qt and foot do. The client asked for client-side
  decorations, asked for no preference, or never bound the protocol.
- `none`: an X11 window while no shell declares decorations.

Scripts read it from the client:

- `client.decoration` is the answer above.
- `client.decorationWanted` is what the client asked for: `client`, `server`, or `unset`.
- `ClientListener.onDecorationChanged` hears the answer change.

`Decorations` mounts a `Titlebar` only on a `server` window, and takes it away from a window whose answer changes to
anything else. A `client` window never shows a second bar over its own.

The shell declares that it draws decorations by writing the seat's `decorations_available`:

- It is true while `decorations` is on, and cleared when the shell retires.
- The compositor also clears it when the shell's connection drops.
- A client that asks for server-side is told so only while it is true and a shell holds the tree.
- With no shell, or with `decorations` off, every client is told client-side, so a window is never left with no title bar
  and nothing to move it by.
- A change reaches the windows that already exist. A reload that turns decorations off gives each window that asked for
  the shell's bar its own back, and a reload that turns them on gives the bar again.

### The room above the client

The stock library gives a server-decorated window's bar room of its own instead of drawing over the client:

- The bar is a child of the window's node at `y = -height`: it lies above the node, never over it.
- `client.bounds` (and `x`, `y`, `width` and `height`) is the content's rectangle. The frame is the content with the bar
  on top, and `client.frameInset` is the bar's height. It is 0 for a window with no bar: one that draws its own frame, or
  that is fullscreen.
- A layout places frames. The layout driver writes a placement `y` plus the inset and a height minus the inset, so a
  tiled window's bar is on its tile. Floating placement places the frame and puts the content below the bar. Dragging
  keeps the frame inside the work area, so a bar is never lost off the top, and an edge snap tiles the frame.
- A window the compositor maximises or makes fullscreen is sized by the compositor. A maximise through the titlebar or
  the menu goes through the shell, which leaves the bar its room.

To have no bar on any window, leave `decorations` off.

## Entry, Clock and Bar

`Entry` is a line of text you type into. Give it the keyboard and it reads `onKey`:

| Key                           | Effect                                                           |
| ----------------------------- | ---------------------------------------------------------------- |
| Letters and other typing keys | Type at the caret, over the selection                            |
| BackSpace, Delete             | Remove the selection, or the character before or after the caret |
| Left, Right, Home, End        | Move the caret. With Shift held they select                      |
| Control+A                     | Select all                                                       |
| Return                        | Call `entry.onSubmit(text)`                                      |
| Escape, Tab, other keys       | Left for the caller                                              |

A key an entry uses it stops. `secret: true` shows a `*` for each character. The editing itself is `EntryModel`, with no
drawing: `text`, `caret`, `selection`, `insert`, `backspace`, `delete`, `left`, `right`, `home`, `end` and `selectAll`.

`Clock` is a `Text` that shows the time as a strftime format, `%H:%M` by default. `formatTime` supports `%H %I %M %S
%p %d %e %m %y %Y %j %a %A %b %B %T %R %F %%`.

`Bar` is a strip along the `top` or `bottom` of the primary screen, over the windows and as wide as the screen. It takes
its room from them:

- `gnoblin.workspaces.area` and the layouts' work area are the screen less the bars (`barInsets()`).
- `hideWhen` hides a bar, and gives its room back, while it returns true.

## The stock shell UI

Each of these is on in the stock config and has a switch.

### Window menu

Switch: `defineConfig({ menus: false })`. The module is `gnoblin/menus`.

- The compositor asks for a window's menu as a `menu` request on the window's node: a client's `show_window_menu`, the
  window-menu key (Alt+Space) and a program's `request_menu`. The titlebar's right click asks too.
- The shell answers every request with `windowMenu.show(client, x, y)`. With `menus: false` the requests are left
  unanswered: nothing opens, and the compositor waits for nothing.
- Entries: Restore or Maximise, Minimise, Move, Resize, Always on top, Move to workspace and Close. Move and Resize are
  the keyboard modes, which are not ported: they are off unless the client has `moveWithKeys` and `resizeWithKeys`.
- Keys: Up, Down, Home, End, Tab and Shift+Tab move over the entries that are on; a letter jumps; Return and Space run;
  Right and Left open and close the submenu; Escape closes.
- A press outside closes the menu and is not passed on. One menu is open at a time. It closes when its window goes or
  the session locks, and the keyboard goes back to where it was.
- Assign `windowMenu.items = (client) => [...]` to change what it offers and `windowMenu.show` to draw another.

### Not-responding alert

Switch: `defineConfig({ alerts: false })`. The module is `gnoblin/alerts`.

- A window the compositor marks `unresponsive` gets a card with Wait (the default, and Escape) and Force quit.
- Force quit calls `client.kill()` and does nothing for the first 0.5 s. The compositor refuses its own process, init
  and a process that did not open the window.
- A window the user waited on gets no second alert until it has answered a ping and stopped answering again.
- No alert opens under a lock, and an alert closes an open window menu first. The state machine is `AlertTracker`.

### Launcher, toast and overview

- **Launcher** (`gnoblin/launcher`). With no `launcher` command in the config, Super+Space opens an `Entry` on the
  screen. Return starts what you typed (`gnoblin.spawn`) and Escape closes it. `launcher: "wofi"` runs that program on
  the key instead.
- **`toast(text)`** shows a message card in the top right corner for a few seconds.
- **`overview.toggle()`** dims the windows of the workspaces that are not shown, or gives them all full strength, .

The consent and passphrase dialogs are not here: the compositor draws them, and `gnoblin.dialogs` observes them.

## What is not done

- Dragging a titlebar with a real pointer. It moves the window by how far the pointer is from where it was pressed, and
  has been tested against a fake compositor only. A right click on the bar is proved on the real compositor.
- A fade on close for a window the client closes itself.
