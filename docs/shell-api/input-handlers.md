# Input handlers

A key press or a click is a function. A pointer drag and a touchpad gesture take place over time, so they are binding
objects with hooks that run as the input starts, moves and ends. The stock shell moves and resizes windows, switches
workspaces and acts on gestures with these.

```ts
import { PointerDragBinding, binding, defineConfig } from "gnoblin";
import type { Client, DragEvent } from "gnoblin";

interface Grab {
    client: Client;
    x: number;
    y: number;
}

class Nudge extends PointerDragBinding<Grab> {
    onStart(event: DragEvent): Grab | false {
        const client = event.client;
        return client ? { client, x: client.x, y: client.y } : false;
    }
    onUpdate(event: DragEvent, state: Grab): void {
        state.client.x = state.x + (event.dx ?? 0);
        state.client.y = state.y + (event.dy ?? 0);
    }
}

defineConfig({
    keys: {
        "Alt+MouseMiddle+Drag": binding(() => new Nudge({ id: "nudge", trigger: "Alt+MouseMiddle+Drag" })),
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. Hold `Alt`, press the middle button on a window and drag: the window follows
the pointer.

## Drag bindings

Extend `PointerDragBinding` to handle a pointer drag. Its constructor takes `id`, `trigger`, and `threshold`, the pixels the
pointer must move from the press before the drag starts. The default is 4. A press and release before that is a click.

| Hook                     | Runs when                                                                                          |
| ------------------------ | -------------------------------------------------------------------------------------------------- |
| `onStart(event)`         | The pointer went past the threshold. Return a state object to own the drag, or `false` to decline. |
| `onUpdate(event, state)` | The pointer moved. It runs at most once a frame, with the position at that moment.                 |
| `onEnd(event, state)`    | The button was released.                                                                           |
| `onCancel(event, state)` | The compositor cut the drag short, for example because the session locked.                         |

`onStart` returns a fresh state object for each drag, and each later hook receives it, so one binding can serve several
seats at once. A binding that declines lets the next binding of the same trigger have the drag. `onStart`, `onEnd` and
`onCancel` are never dropped. When scripts fall behind, `onUpdate` reads the position at that moment and skips the samples between.

The event carries what the hooks need:

| Field                        | Meaning                                                                          |
| ---------------------------- | -------------------------------------------------------------------------------- |
| `timestamp`, `deltaTime`     | The input time, and seconds since the previous event of this drag, at most 0.1.  |
| `client`, `layout`           | The window under the pointer at the press, and the layout whose binding this is. |
| `position`, `localPosition`  | The pointer in compositor coordinates, and in the window's own coordinates.      |
| `x`, `y`                     | The pointer. The same as `position`.                                             |
| `dx`, `dy`                   | Pixels from the press.                                                           |
| `button`                     | The button, numbered as X11 does: 1 left, 2 middle, 3 right.                     |
| `modifiers`                  | The modifiers held, as a set of names.                                           |
| `workArea`, `dragConstraint` | The rectangle a dragged window stays in, and the seat's constraint.              |
| `edgeTiling`                 | Whether a drop on an edge tiles the window.                                      |
| `context`, `device`          | The seat, and the device when the claim was for one device.                      |

The stock drags are classes you can reuse or extend:

| Class                 | Trigger it takes by default | What it does                                                                                                                            |
| --------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `ClientMoveBinding`   | `Super+MouseLeft+Drag`      | The window follows the pointer and stays in the work area. `snap` is how near an edge, in pixels, a drop tiles it; `false` never tiles. |
| `ClientResizeBinding` | `Super+MouseRight+Drag`     | The corner nearest the press follows the pointer. `edges` is `quadrant`, `nearest` or one corner.                                       |
| `ClientSwapBinding`   | `Super+MouseLeft+Drag`      | In a tiling layout, drop the tile you hold on another one and the two trade places when you let go.                                     |
| `SplitResizeBinding`  | `Super+MouseRight+Drag`     | In a tiling layout, the divider nearest the press follows the pointer, and the tiles on both sides follow it.                           |

`dragConstraint` is `work_area`, which keeps a window inside the work area, or `minimum_strip`, which lets a window leave
until a strip of 75 pixels stays visible. Set it in `defineConfig`.

### Tiling drags

The tiling layouts register the last two classes themselves, under the ids `swap` and `resize`:

- `MasterStackLayout` and `DwindleLayout` register both.
- `GridLayout` registers `swap` only, since a grid has no divider to drag.
- `MonocleLayout` registers neither.

Each layout takes a `triggers` option, as `FloatingLayout` does. A string replaces a trigger and `null` leaves that binding
out.

```ts
import { MasterStackLayout, defineConfig } from "gnoblin";

defineConfig({
    workspaces: [
        {
            id: "dev",
            layout: new MasterStackLayout({
                ratio: 0.6,
                triggers: { swap: "Alt+MouseLeft+Drag", resize: null },
            }),
        },
    ],
});
```

`ClientSwapBinding` changes nothing while the pointer is down. When you let go over another tiled window, the two trade
places in `workspace.clients` and the layout computes once. A release outside every tile, over the window you hold, or a
cancel leaves the order as it was.

A floating, fullscreen or maximised window declines the swap, and so does a press with no window under it.

`SplitResizeBinding` asks the layout for the divider nearest the press, through `layout.findSplit(client, point)`. The
divider follows the pointer as far as the pointer travels, and the layout computes on each frame, so the windows move while
you drag. A release keeps the ratio, and a cancel puts it back. What a divider is depends on the layout:

- `MasterStackLayout` has one, between the master and the stack. Its `ratio` changes, held to 0.1 to 0.9.
- `DwindleLayout` has one for each split. The drag changes the ratio of the split it took and leaves the others.
  `layout.ratio` is still what every split that nobody dragged uses.
- A layout with no divider declines. That is a grid, or a master stack with a single window.

To drag a divider of your own layout, return a `LayoutSplit` from `findSplit`:

```ts
import { Layout } from "gnoblin";
import type { LayoutContext, LayoutSplit, Placement } from "gnoblin";

class Divider implements LayoutSplit {
    readonly axis = "x";
    readonly direction = 1;
    readonly min = 0.2;
    readonly max = 0.8;
    readonly span: number;
    readonly #layout: Halves;
    readonly #start: number;

    constructor(layout: Halves, span: number) {
        this.#layout = layout;
        this.#start = layout.ratio;
        this.span = span;
    }

    get ratio(): number {
        return this.#layout.ratio;
    }

    set ratio(value: number) {
        this.#layout.ratio = value;
    }

    reset(): void {
        this.#layout.ratio = this.#start;
    }
}

class Halves extends Layout {
    ratio = 0.5;
    width = 0;

    compute(context: LayoutContext): Placement[] {
        const { x, y, width, height } = context.area;
        const left = Math.floor(width * this.ratio);
        this.width = width;
        return context.children.map((node, index) => ({
            node,
            tiled: true,
            bounds: index === 0 ? { x, y, width: left, height } : { x: x + left, y, width: width - left, height },
        }));
    }

    findSplit(): LayoutSplit | null {
        return this.width > 0 ? new Divider(this, this.width) : null;
    }
}
```

`span` is the pixels of pointer travel that change the ratio by 1, and `direction` is -1 when moving the pointer right or
down shrinks the ratio. A layout that keeps its own record of who sits where can also define `onClientsSwapped(a, b)`, which
runs when a swap trades two windows. `DwindleLayout` uses it to trade two leaves of its tree.

To trade two windows from a script, call `workspace.clients.swap(a, b)`. Both must be in the workspace.

```ts
import { gnoblin } from "gnoblin";

const dev = gnoblin.workspaces.get("dev");
const [first, second] = dev?.clients.map((client) => client) ?? [];
if (dev && first && second) {
    dev.clients.swap(first, second);
}
```

## Gesture bindings

A touchpad gesture binding extends `GestureBinding` and uses the same four hooks. Its trigger is a gesture such as `Swipe3`
or `Pinch2In`. A gesture event adds these fields:

| Field               | Meaning                                                                   |
| ------------------- | ------------------------------------------------------------------------- |
| `phase`             | `begin`, `update`, `end` or `cancel`.                                     |
| `dx`, `dy`          | Movement since the begin, in touchpad units.                              |
| `scale`, `rotation` | The spread of a pinch, where 1 is unchanged, and its rotation in radians. |
| `progress`          | 0 to 1: how far the gesture has gone its own way.                         |

`WorkspaceSlideBinding` is the stock three-finger slide between workspaces: the windows follow the fingers, and the slide
commits when the fingers lift past `commit`, a fraction from 0 to 1 that defaults to 0.25. Its `travel` is the distance in
touchpad units that equals a full slide, and `seconds` the length of the slide that finishes it.

To run a function when a gesture ends, name the gesture in `gestureActions` or bind a function to it. The named actions and
`gnoblin.gestures` are in [Window policy](/js-window-policy#gesture-actions).

## Add a binding at run time

A drag or gesture binding goes to the seat with `gnoblin.input.primary.bind(trigger, binding)`. The seat moves the binding to
the trigger you give. `binding.unbind()` removes it, and the compositor's claim goes when no binding needs it.

```ts
const nudge = gnoblin.input.primary.bind(
    "Alt+MouseMiddle+Drag",
    new Nudge({ id: "nudge", trigger: "Alt+MouseMiddle+Drag" }),
);
```

A `BindingCollection`, such as `context.bindings`, `device.bindings` or `layout.bindings`, holds the bindings by `id`: `add`,
`get`, `has`, `remove`, `clear` and `size`. Adding a binding with a taken id, or one that is already in another collection,
throws.
