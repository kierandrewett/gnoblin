# UI toolkit

A shell draws its own interface with a small toolkit. It is a class library of `gnoblin-std`, so a TypeScript, a Python and
a Rust shell use the same classes with the same members, each spelled its own way.

- TypeScript imports them with `import { Box, Text } from "gnoblin-std"`.
- Python imports them with `from gnoblin.std import Box, Text`.
- Rust takes them from `gnoblin_std`, where the box is `UiBox` because `Box` is a Rust type.

Properties are `camelCase` in TypeScript, `snake_case` attributes in Python, and a getter with a `set_` method in Rust.

Every widget is an ordinary object with ordinary fields. Change a field and the next frame paints it. There is nothing to
register and no property to declare.

## Canvas

A `Canvas` is a node of the compositor's tree that shows a paint list. Give it a `behaviour`, an object of the `Widget`
interface, and have its `paint()` return the ops to paint, in node-local pixels, later ops over earlier ones.

- TypeScript and Python extend `Widget`.
- Rust fills in `WidgetHandlers` and calls `implement`.

::: tabs

```typescript
import { Canvas, Widget } from "gnoblin-std";
import type { PaintOp } from "gnoblin-std";

class Meter extends Widget {
    level = 0.5;

    constructor(readonly canvas: Canvas) {
        super();
    }

    paint(): PaintOp[] {
        const { width, height } = this.canvas;
        return [
            { op: "rect", x: 0, y: 0, width, height, radius: 4, fill: "#1e1e2e" },
            { op: "rect", x: 0, y: 0, width: width * this.level, height, radius: 4, fill: "#89b4fa" },
        ];
    }
}

const canvas = new Canvas({ width: 120, height: 8 });
const meter = new Meter(canvas);
canvas.behaviour = meter;
canvas.mountTop();

meter.level = 0.8;
canvas.invalidate();
```

```python
from gnoblin.std import Canvas, PaintKind, PaintOp, Widget


class Meter(Widget):
    def __init__(self, canvas: Canvas) -> None:
        self.canvas = canvas
        self.level = 0.5

    def paint(self) -> list[PaintOp]:
        width, height = self.canvas.width, self.canvas.height
        return [
            PaintOp(PaintKind.RECT, x=0.0, y=0.0, width=width, height=height, radius=4.0, fill="#1e1e2e"),
            PaintOp(PaintKind.RECT, x=0.0, y=0.0, width=width * self.level, height=height, radius=4.0, fill="#89b4fa"),
        ]


canvas = Canvas(width=120.0, height=8.0)
meter = Meter(canvas)
canvas.behaviour = meter
canvas.mount_top()

meter.level = 0.8
canvas.invalidate()
```

```rust
use std::cell::Cell;
use std::rc::Rc;

use gnoblin_std::{Canvas, PaintKind, PaintOp, WidgetHandlers};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    let canvas = Rc::new(Canvas::new(&objects)?);
    canvas.set_width(120.0)?;
    canvas.set_height(8.0)?;
    let level = Rc::new(Cell::new(0.5));

    let (surface, shown) = (Rc::clone(&canvas), Rc::clone(&level));
    let meter = WidgetHandlers {
        paint: Some(Box::new(move || {
            let (width, height) = (surface.width()?, surface.height()?);
            Ok(vec![
                PaintOp {
                    x: Some(0.0),
                    y: Some(0.0),
                    width: Some(width),
                    height: Some(height),
                    radius: Some(4.0),
                    fill: Some("#1e1e2e".to_owned()),
                    ..PaintOp::new(PaintKind::Rect)
                },
                PaintOp {
                    x: Some(0.0),
                    y: Some(0.0),
                    width: Some(width * shown.get()),
                    height: Some(height),
                    radius: Some(4.0),
                    fill: Some("#89b4fa".to_owned()),
                    ..PaintOp::new(PaintKind::Rect)
                },
            ])
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    canvas.set_behaviour(&meter)?;
    canvas.mount_top()?;

    level.set(0.8);
    canvas.invalidate()?;
    Ok(())
}
```

:::

`invalidate()` asks for the canvas to paint again, so a change to `level` shows on the next frame.

The ops are:

- `rect`: `x`, `y`, `width`, `height`, and optionally `radius`, `fill`, `stroke` and `strokeWidth`.
- `line`: `x1`, `y1`, `x2`, `y2`, and optionally `width` and `color`.
- `text`: `text`, and optionally `x`, `y`, `size`, `color` and `font`.
- `icon`: `name`, and optionally `x`, `y`, `size` and `color`.
- `push-clip` with `x`, `y`, `width`, `height` and optionally `radius`, closed by `pop-clip`.
- `push-opacity` with `alpha`, closed by `pop-opacity`.

An op is a `PaintOp` with its `op` kind (`PaintKind.RECT` in Python, `PaintKind::Rect` in Rust). A colour is `#rgb`,
`#rgba`, `#rrggbb` or `#rrggbbaa`.

The toolkit encodes the list into the node's `content` property, and writes it only when it changed. An op that cannot be encoded is reported as `draw op N: ...` and the node keeps its last content. The
format is described in [Canvas painting](/shell-api/canvas).

A canvas has these fields and methods:

- `x`, `y`, `width`, `height`, `visible` and `opacity`.
- `parent` and `children`, the tree of canvases.
- `add(child)` puts a canvas under this one, and `remove(child)` takes it out. `mount(node)` puts it under a node that is
  not a canvas.
- `destroy()` ends it and everything beneath it.
- `behaviour` is the `Widget` that draws and arranges the canvas. Its `refresh()` runs once a frame before the children are
  arranged, for a canvas that reads live state. Its `arrange()` places the children, and its `paint()` returns the ops.
  Each method is optional.

## Box, Text, Icon and Grid

A `Box` is a canvas that fills itself (`fill`, `radius`) and arranges its child boxes:

- `direction` is `"row"`, `"column"` or `"none"`. A row or column is a small flexbox. `"none"` leaves the children
  where they are. Python takes the name or `Direction.ROW`, and Rust takes `Direction::Row`.
- `padding` is the space around the children and `gap` the space between them.
- `grow` on a child takes its share of what is left. Every child is stretched across the other axis.

::: tabs

```typescript
import { Box, Icon, Text } from "gnoblin-std";

const bar = new Box({ width: 300, height: 32, direction: "row", gap: 8, padding: 6, fill: "#1e1e2e" });
bar.add(new Text({ text: "Files", grow: 1 }));
bar.add(new Icon({ icon: "window-close-symbolic" }));
bar.mountTop();
```

```python
from gnoblin.std import Box, Icon, Text

bar = Box(width=300.0, height=32.0, direction="row", gap=8.0, padding=6.0, fill="#1e1e2e")
bar.add(Text(text="Files", grow=1.0))
bar.add(Icon(icon="window-close-symbolic"))
bar.mount_top()
```

```rust
use gnoblin_std::{Direction, Icon, Text, UiBox};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    let bar = UiBox::new(&objects)?;
    bar.set_width(300.0)?;
    bar.set_height(32.0)?;
    bar.set_direction(Direction::Row)?;
    bar.set_gap(8.0)?;
    bar.set_padding(6.0)?;
    bar.set_fill("#1e1e2e")?;

    let label = Text::new(&objects)?;
    label.set_text("Files")?;
    label.set_grow(1.0)?;
    bar.add(&label.as_canvas()?)?;

    let close = Icon::new(&objects)?;
    close.set_icon("window-close-symbolic")?;
    bar.add(&close.as_canvas()?)?;

    bar.mount_top()?;
    Ok(())
}
```

:::

The other widgets are boxes too:

- `Text` draws one string. Its size is what the compositor measures for its font, size and text. The answer arrives a
  frame later, and until then the size is an estimate. With `ellipsize` a text wider than its node ends in an ellipsis.
- `Icon` draws a named icon, square at its `size`.
- `Grid` puts its children in a near-square grid of equal cells.

Give a node a `width` or `height` and that is what it has. Call `clearWidth()` or `clearHeight()` to go back to the
natural size.

A canvas handles pointer events with `onPointer`:

- `onPointer` takes the event. Its `kind` is `press`, `release`, `motion`, `scroll`, `enter` or `leave`, and `button` is the
  button of a press or a release.
- An event goes to the canvas the pointer is over, then up through its parents until a handler returns `true`.
- The event's `x` and `y` are in the coordinates of the canvas whose handler runs, and `dx` and `dy` are the distance of
  a motion or a scroll.
- A press on a canvas holds it until that button is released: it hears the motion and the release wherever the pointer
  goes, with `x` and `y` outside its rectangle, and nothing else hears the pointer meanwhile. A window is not held, because
  its client hears the pointer itself.

A key goes to the canvas that holds the keyboard focus, then up through its parents, in `onKey(event)`, which returns
`true` to stop it:

- `canvas.focus()` gives a canvas the keyboard. It receives keys only while it holds it.
- The event has `key` (the keysym's name: `Return`, `a`), `text` (what the key types, empty for a key that types
  nothing) and `pressed` (false for the release).
- `canvas.mountTop()` puts a canvas over every window, as a menu or a dialog needs.
- `secure: true` keeps a canvas out of screen captures.

## Titlebar

After `Decorations.get().start()` a window that takes the shell's frame gets a `WindowFrame`, and the stock fill puts a
`Titlebar` in its top outlet. The starter calls it. A file that does not call it has no titlebars. A window that draws its
own title bar gets none (see "Who draws the frame" below).

To make a different frame, fill the outlets yourself with [the window frame](/js-window-frame). The
[recipes](/recipes/window-frame) show three.

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

Its look is plain data, `TitlebarStyle`. A field you leave out keeps its default:

::: tabs

```typescript
import { Decorations } from "gnoblin-std";

const decorations = Decorations.get();
decorations.style = { height: 36, buttons: ["close"], focusedFill: "#223344" };
decorations.start();
```

```python
from gnoblin.std import Decorations, TitlebarStyle, WindowButtonName

decorations = Decorations.get()
decorations.style = TitlebarStyle(height=36.0, buttons=[WindowButtonName.CLOSE], focused_fill="#223344")
decorations.start()
```

```rust
use gnoblin_std::{Decorations, TitlebarStyle, WindowButtonName};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let decorations = Decorations::get(&objects)?;
    decorations.set_style(&TitlebarStyle {
        height: 36.0,
        buttons: Some(vec![WindowButtonName::Close]),
        focused_fill: "#223344".to_owned(),
        ..TitlebarStyle::default()
    })?;
    decorations.start()?;
    Ok(())
}
```

:::

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

Shells read it from the client:

- `client.decoration` is the answer above.
- `client.decorationWanted` (`decoration_wanted` in Python and Rust) is what the client asked for: `client`, `server`, or `unset`.
- A `ClientListener`'s `onDecorationChanged` hears the answer change (`on_decoration_changed` in Python and Rust).

`Decorations` makes a server frame, and fills it with a `Titlebar`, only on a `server` window. It makes it again for a window
whose answer changes. A `client` window never shows a second bar over its own: it has the underlay and the overlay only.

The shell declares that it draws decorations by writing the seat's `decorations_available`:

- It is true while `Decorations.get().start()` has run, and cleared when the shell retires.
- A reload commits the new generation before the old one ends: a new generation that starts decorations keeps it true, and
  one that does not clears it, so the old generation's end never undoes the new one's declaration.
- The compositor also clears it when the shell's connection drops.
- A client that asks for server-side is told so only while it is true and a shell holds the tree.
- With no shell, or without `Decorations.get().start()`, every client is told client-side, so a window is never left with no title bar
  and nothing to move it by.
- A change reaches the windows that already exist. A reload whose file does not call `Decorations.get().start()` gives each window that asked for
  the shell's bar its own back, and a reload whose file calls it gives the bar again.

### The room round the client

The stock library gives a server-decorated window's frame room of its own instead of drawing over the client:

- A frame is a `WindowFrame`: a node under the window's node for each outlet the script reads. The stock titlebar is a
  child of the `top` outlet, which lies above the window's node at `y = -height` and never over it. See
  [the window frame](/js-window-frame).
- `client.bounds` (and `x`, `y`, `width` and `height`) is the content's rectangle. The frame is the content with its
  inset round it, and `client.frameInset` (`frame_inset` in Python and Rust) is that inset: `{ top, right, bottom, left }`.
  It is all zero for a window with no frame: one that draws its own frame, or that is fullscreen. For the stock titlebar
  only `top` is more than 0.
- A layout places frames. The layout driver gives a tile to the frame and the content is what the four sides leave.
  Floating placement places the frame and puts the content inside it. Dragging keeps the frame inside the work area, so an
  edge is never lost off the screen, and an edge snap tiles the frame.
- A window the compositor maximises or makes fullscreen is sized by the compositor. A maximise through the titlebar or
  the menu goes through the shell, which leaves the frame its room.
- The frame keeps its place when it comes, goes or changes. A frame can arrive after the window was placed: the window
  mapped before the shell declared decorations, a reload turned them on, or the compositor changed the window's
  `decoration`.
    - The content moves in by what the top and left gained and out by what they lost, so no outlet is drawn off the
      work area. The frame is clamped inside the area.
    - A maximised window fills the area again, a snapped one tiles to its zone again, and a tiled one is placed again by its
      layout.
    - The rectangles a window returns to follow. A fullscreen window is left alone and fitted when it leaves.

To have no frame on any window, leave `Decorations.get().start()` out of your file.

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

::: tabs

```typescript
import { Entry } from "gnoblin-std";

const entry = new Entry({ x: 20, y: 48, width: 320, height: 36, fill: "#313244" });
entry.mountTop();
entry.onSubmit = (text) => console.log(`submitted ${text}`);
entry.focus();
```

```python
from gnoblin.std import Entry

entry = Entry(x=20.0, y=48.0, width=320.0, height=36.0, fill="#313244")
entry.mount_top()
entry.on_submit = lambda text: print(f"submitted {text}")
entry.focus()
```

```rust
use gnoblin_std::Entry;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    let entry = Entry::new(&objects)?;
    entry.set_x(20.0)?;
    entry.set_y(48.0)?;
    entry.set_width(320.0)?;
    entry.set_height(36.0)?;
    entry.set_fill("#313244")?;
    entry.mount_top()?;
    entry.set_on_submit(|text| {
        println!("submitted {text}");
        Ok(())
    })?;
    entry.focus()?;
    Ok(())
}
```

:::

A key an entry uses it stops. `secret: true` shows a `*` for each character. The editing itself is `EntryModel`, with no
drawing: `text`, `caret`, `selection`, `insert`, `backspace`, `delete`, `left`, `right`, `home`, `end` and `selectAll`.

`Clock` is a `Text` that shows the time as a strftime format, `%H:%M` by default. `formatTime` supports `%H %I %M %S
%p %d %e %m %y %Y %j %a %A %b %B %T %R %F %%`.

A clock keeps its own time. The shell runs no frame while nothing happens, so a `Clock` asks for one when its text is
due to change: at the end of the minute, or of the second for a format that shows seconds.

`Bar` is a strip along the `top` or `bottom` of an output, over the windows and as wide as the output. It sits on the
primary output unless `output` names another output's node. Bars on one edge of one output stack in the order they were
made. A bar takes its room from the windows there:

- `gnoblin.workspaces.area`, the area of an output and the layouts' work area are the output less its bars. A maximised
  window fills what the bars leave, a snapped window tiles in it, and a floating window under a new bar moves below it.
- Room follows the bar: it comes when the bar is made, and goes when the bar is destroyed, hidden or unplugged with its output.
- A bar gives way to a fullscreen window. While a window that is not minimised is fullscreen on the workspace an output
  shows, that output's bars are hidden (`shown` is false) and take no room, and the fullscreen window covers the whole output.
  They come back when it leaves or closes. Bars on other outputs stay.
- `hideWhen` hides a bar, and gives its room back, while it returns true.
- `Bar.insets(output)` is the room the bars take from the `top` and `bottom` of an output, the primary one when none is given.

::: tabs

```typescript
import { Bar } from "gnoblin-std";

const bar = new Bar({ edge: "top", height: 32 });
console.log(Bar.insets().top, bar.shown);
bar.destroy();
```

```python
from gnoblin.std import Bar

bar = Bar(edge="top", height=32.0)
print(Bar.insets().top, bar.shown)
bar.destroy()
```

```rust
use gnoblin_std::prelude::*;
use gnoblin_std::{Bar, BarEdge};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    let bar = Bar::new(&objects)?;
    bar.set_edge(BarEdge::Top)?;
    bar.set_height(32.0)?;
    let room = Bar::insets(&objects, None)?;
    println!("{} {}", room.top, bar.shown()?);
    bar.destroy()?;
    Ok(())
}
```

:::

A shell can answer the two questions itself: `Bar.onOutputs(primary)` says which output is the primary one, and
`Bar.giveWayTo(covered)` says whether a fullscreen window covers an output. Leave them out for the library's answers, which
are the output flagged primary (else the first) and a fullscreen window on the workspace that output shows.

## The stock shell UI

Each of these is started by the starter and has a function that starts it.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { Launcher, NotRespondingAlerts, Toast, WindowMenus } from "gnoblin-std";

WindowMenus.get().start();
NotRespondingAlerts.get().start();
gnoblin.input.primary.bind("Super+space", () => {
    Launcher.open();
});
Toast.show("The shell is running");
```

```python
from gnoblin import BindingEvent, Gnoblin
from gnoblin.std import Launcher, NotRespondingAlerts, Toast, WindowMenus

gnoblin = Gnoblin.get()

WindowMenus.get().start()
NotRespondingAlerts.get().start()


def launcher(event: BindingEvent) -> None:
    Launcher.open()


gnoblin.input.primary.bind("Super+space", launcher)
Toast.show("The shell is running")
```

```rust
use std::rc::Rc;
use std::time::Instant;

use gnoblin_core::Gnoblin;
use gnoblin_std::{Launcher, NotRespondingAlerts, Toast, WindowMenus};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Rc::new(Gnoblin::get(&g.objects()?)?);
    let objects = g.objects()?.std()?;

    WindowMenus::get(&objects)?.start()?;
    NotRespondingAlerts::get(&objects)?.start()?;
    let context = model.input()?.primary()?;
    let library = objects.clone();
    let shell = Rc::clone(&model);
    context.bind(
        "Super+space",
        Some(move |_| {
            let shell = Rc::clone(&shell);
            // Return starts what you typed, as `gnoblin.spawn` does.
            Launcher::open(
                &library,
                Some(move |command: String| {
                    shell.spawn(&[command], None)?;
                    Ok(())
                }),
            )?;
            Ok(true)
        }),
        None,
    )?;
    // The toast reads this clock in milliseconds, so the fade follows the shell's own time.
    let started = Instant::now();
    Toast::show(
        &objects,
        "The shell is running",
        None,
        None,
        None,
        Some(move || Ok(started.elapsed().as_secs_f64() * 1000.0)),
    )?;
    Ok(())
}
```

:::

### Window menu

Start it with `WindowMenus.get().start()`.

- The compositor asks for a window's menu as a `menu` request on the window's node: a client's `show_window_menu`, the
  window-menu key (Alt+Space) and a program's `request_menu`. The titlebar's right click asks too.
- The shell answers every request with the menu service's `show(client, x, y)`. Without `WindowMenus.get().start()` the
  requests are left unanswered: nothing opens, and the compositor waits for nothing.
- Entries: Restore or Maximise, Minimise, Move, Resize, Always on top, Move to workspace and Close. Move and Resize are
  the keyboard modes: they are off until `Repositioning.get().start()` runs or the service has `moveWithKeys` and
  `resizeWithKeys` functions of your own.
- Keys: Up, Down, Home, End, Tab and Shift+Tab move over the entries that are on; a letter jumps; Return and Space run;
  Right and Left open and close the submenu; Escape closes.
- A press outside closes the menu and is not passed on. One menu is open at a time. It closes when its window goes or
  the session locks, and the keyboard goes back to where it was.
- Give the service a `MenuProvider` to change what it offers or to draw another: see [the window
  menu](/guides/stock-window-menu#change-what-the-menu-offers).

### Not-responding alert

Start it with `NotRespondingAlerts.get().start()`.

- A window the compositor marks `unresponsive` gets a card with Wait (the default, and Escape) and Force quit.
- Force quit calls `client.kill()` and does nothing for the first 0.5 s. The compositor refuses its own process, init
  and a process that did not open the window.
- A window the user waited on gets no second alert until it has answered a ping and stopped answering again.
- No alert opens under a lock, and an alert closes an open window menu first. The state machine is `AlertTracker`.

### Launcher, toast and overview

- **`Launcher.open()`** opens an `Entry` on the screen, and the starter binds it to Super+Space. Return starts what you
  typed (`gnoblin.spawn`) and Escape closes it. `Launcher.open(fn)` calls `fn` with the command instead of starting it.
- **`Toast.show(text)`** shows a message card in the top right corner for a few seconds.
- **`Overview.get().toggle()`** dims the windows of the workspaces that are not shown, or gives them all full strength.

`Ui.estimateText` and `Ui.encodePaintList` are the toolkit's two functions that need no canvas:

- TypeScript spells them `Ui.estimateText` and `Ui.encodePaintList`.
- Python spells them `Ui.estimate_text` and `Ui.encode_paint_list`.
- Rust spells them `Ui::estimate_text` and `Ui::encode_paint_list`.

[The TypeScript shell page](typescript-shell.md) has the table of the functions the older library had and the class that
now holds each, and [the Python shell page](python-shell.md) has the same table in Python spellings.

The consent and passphrase dialogs are not here: the compositor draws them, and `gnoblin.dialogs` observes them.

## What is not done

- A right click on the titlebar and a drag of a resize handle are proved on the real compositor. A drag of the titlebar
  itself has been tested against a fake compositor only.
- A fade on close for a window the client closes itself.
