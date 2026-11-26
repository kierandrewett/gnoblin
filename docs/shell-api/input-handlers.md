# Input handlers

A key press or a click is a function. A pointer drag and a touchpad gesture take place over time, so they are binding
objects with hooks that run as the input starts, moves and ends. The stock shell moves and resizes windows, switches
workspaces and acts on gestures with these.

```ts
import { PointerDragBinding } from "gnoblin-core";
import type { BindingEvent, Client } from "gnoblin-core";

class Nudge extends PointerDragBinding {
    private client: Client | undefined;
    private x = 0;
    private y = 0;

    onStart(event: BindingEvent): boolean {
        if (event.client === undefined) {
            return false;
        }
        this.client = event.client;
        this.x = event.client.x;
        this.y = event.client.y;
        return true;
    }

    onUpdate(event: BindingEvent): void {
        if (this.client !== undefined) {
            this.client.x = this.x + (event.dx ?? 0);
            this.client.y = this.y + (event.dy ?? 0);
        }
    }

    onEnd(): void {
        this.client = undefined;
    }

    onCancel(): void {
        this.client = undefined;
    }
}

gnoblin.input.primary.bindDrag("Alt+MouseMiddle+Drag", new Nudge(), { id: "nudge" });
```

Put the file in `~/.config/gnoblin/shell.ts`. Hold `Alt`, press the middle button on a window and drag: the window follows
the pointer.

## Drag bindings

Extend `PointerDragBinding` to handle a pointer drag, and give it to `bindDrag(trigger, binding, options?)`. The options
are `id`, `priority`, `enabled` and `threshold`.

`threshold` is the pixels the pointer must move from the press before the drag starts. The default is 4. A press and
release before that is a click.

| Hook              | Runs when                                                                                  |
| ----------------- | ------------------------------------------------------------------------------------------ |
| `onStart(event)`  | The pointer went past the threshold. Return `true` to own the drag, or `false` to decline. |
| `onUpdate(event)` | The pointer moved. It runs at most once a frame, with the position at that moment.         |
| `onEnd(event)`    | The button was released.                                                                   |
| `onCancel(event)` | The compositor cut the drag short, for example because the session locked.                 |

Only `onStart` is required. Keep what a drag needs in fields of the class, as `Nudge` does. A binding that declines lets
the next binding of the same trigger have the drag. `onStart`, `onEnd` and `onCancel` are never dropped. When scripts fall
behind, `onUpdate` reads the position at that moment and skips the samples between.

For a plain case, `input.bind` also takes a function for a drag trigger, which runs for each step with the event's `phase`
set.

The event carries what the hooks need:

| Field                       | Meaning                                                                          |
| --------------------------- | -------------------------------------------------------------------------------- |
| `timestamp`, `deltaTime`    | The input time, and seconds since the previous event of this drag, at most 0.1.  |
| `client`, `layout`          | The window under the pointer at the press, and the layout whose binding this is. |
| `position`, `localPosition` | The pointer in compositor coordinates, and in the window's own coordinates.      |
| `dx`, `dy`                  | Pixels from the press.                                                           |
| `button`                    | The button, numbered as X11 does: 1 left, 2 middle, 3 right.                     |
| `modifiers`                 | The modifiers held, as a list of names.                                          |
| `phase`                     | `start`, `update`, `end` or `cancel`.                                            |
| `dragConstraint`            | The seat's constraint.                                                           |
| `edgeTiling`                | Whether a drop on an edge tiles the window.                                      |
| `context`, `device`         | The seat, and the device when the claim was for one device.                      |

The stock drags are classes you install. Each takes a `trigger` option, and `install()` binds it on the first seat:

| Class                | Trigger it takes by default | What it does                                                                                                                          |
| -------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `MoveBinding`        | `Super+MouseLeft+Drag`      | The window follows the pointer and stays in the work area. `snap` turns tiling on a drop at an edge on, and `snapMargin` is how near. |
| `ResizeBinding`      | `Super+MouseRight+Drag`     | The corner nearest the press follows the pointer. `grip` is `quadrant`, `nearest` or one corner.                                      |
| `SwapBinding`        | `Super+MouseLeft+Drag`      | In a tiling layout, drop the tile you hold on another one and the two trade places when you let go.                                   |
| `SplitResizeBinding` | `Super+MouseRight+Drag`     | In a tiling layout, the divider nearest the press follows the pointer, and the tiles on both sides follow it.                         |

`uninstall()` takes a binding back out, and `installed` says whether it is in. `priority` ranks it among bindings of the same
trigger. A trigger that is already bound at the same priority is refused, so give a drag of your own another trigger or
priority.

A move keeps the window inside the work area. The older `dragConstraint` setting, which let a window leave until a strip of
75 pixels stayed visible, has no object in the API (see [Settings with no object](/js-settings)).

### Tiling drags

`SwapBinding` and `SplitResizeBinding` share a default trigger with `MoveBinding` and `ResizeBinding`, so give them triggers
of their own when you install both pairs:

```ts
import { MasterStackLayout, SplitResizeBinding, SwapBinding, Workspace } from "gnoblin-std";

gnoblin.workspaces.add(new Workspace("dev", { layout: new MasterStackLayout({ ratio: 0.6 }) }));
new SwapBinding({ trigger: "Alt+MouseLeft+Drag" }).install();
new SplitResizeBinding({ trigger: "Alt+MouseRight+Drag" }).install();
```

`SwapBinding` changes nothing while the pointer is down. When you let go over another tiled window, the two trade places in
`workspace.clients` and the layout computes once. A release outside every tile, over the window you hold, or a cancel
leaves the order as it was.

A floating, fullscreen or maximised window declines the swap, and so does a press with no window under it.

`SplitResizeBinding` asks the layout for the divider nearest the press, through `layout.findSplit(client, x, y)`. The
divider follows the pointer as far as the pointer travels, and the layout computes on each frame, so the windows move while
you drag. A release keeps the ratio, and a cancel puts it back. What a divider is depends on the layout:

- `MasterStackLayout` has one, between the master and the stack. Its `ratio` changes, held to 0.1 to 0.9.
- `DwindleLayout` has one for each split. The drag changes the ratio of the split it took and leaves the others.
  `layout.ratio` is still what every split that nobody dragged uses. With `smartResizing: false` the drag moves two
  dividers, the split above the tile and the nearest one of the other direction above that, each by the pointer's
  travel along its own axis: its `LayoutSplit` has a `crossAxis`.
- A layout with no divider declines. That is a grid, or a master stack with a single window.

To drag a divider of your own layout, return a list with one `LayoutSplit` from `findSplit`, and move the divider in
`setSplit`:

```ts
import { Layout, SplitResizeBinding, Workspace } from "gnoblin-std";
import type { LayoutContextInit, LayoutPlacementInit, LayoutSplitInit } from "gnoblin-std";

class Halves extends Layout {
    ratio = 0.5;
    width = 0;
    private start = 0.5;
    private changes = 0;

    compute(context: LayoutContextInit): LayoutPlacementInit[] {
        const { x, y, width, height } = context.area;
        const left = Math.floor(width * this.ratio);
        this.width = width;
        return context.children.map((node, index) => ({
            node,
            tiled: true,
            bounds: index === 0 ? { x, y, width: left, height } : { x: x + left, y, width: width - left, height },
        }));
    }

    findSplit(): LayoutSplitInit[] {
        this.start = this.ratio;
        if (this.width === 0) {
            return [];
        }
        return [
            { token: 1, axis: "horizontal", direction: 1, span: this.width, min: 0.2, max: 0.8, ratio: this.ratio },
        ];
    }

    setSplit(_token: number, ratio: number): void {
        this.ratio = ratio;
        this.changes += 1;
    }

    resetSplit(): void {
        this.ratio = this.start;
        this.changes += 1;
    }

    revision(): number {
        return this.changes;
    }
}

gnoblin.workspaces.add(new Workspace("halves", { layout: new Halves() }));
new SplitResizeBinding().install();
```

The fields of the divider mean this:

- `span` is the pixels of pointer travel that change the ratio by 1.
- `direction` is -1 when moving the pointer right or down shrinks the ratio.
- `token` names the divider in `setSplit` and `resetSplit`.
- `revision()` tells the workspace to compute again when the ratio changed.

Two more things are optional:

- `crossAxis` with `crossDirection`, `crossSpan`, `crossMin`, `crossMax` and `crossRatio` is a second divider on the other
  axis. The same drag moves both, each by the pointer's travel along its own axis, `setSplit` gets the second ratio as
  its third argument, and a cancel resets both.
- `clientsSwapped(a, b)` is defined on the layout, not the divider. It runs when a swap trades two windows, for a layout
  that keeps its own record of who sits where. `DwindleLayout` uses it to trade two leaves of its tree.

To trade two windows from a script, call `workspace.clients.swap(a, b)`. Both must be in the workspace.

```ts
import "gnoblin-std";

const dev = gnoblin.workspaces.byId("dev");
const [first, second] = dev?.clients.members ?? [];
if (dev && first && second) {
    dev.clients.swap(first, second);
}
```

## Gesture bindings

A touchpad gesture binding extends `GestureBinding` and gives it to `bindGesture(trigger, binding, options?)`. Its hooks
are `onBegin` (required), `onUpdate`, `onEnd` and `onCancel`. Its trigger is a gesture such as `Swipe3` or `Pinch2In`. A
gesture event adds these fields:

| Field               | Meaning                                                                   |
| ------------------- | ------------------------------------------------------------------------- |
| `phase`             | `start`, `update`, `end` or `cancel`.                                     |
| `fingers`           | How many fingers are down.                                                |
| `dx`, `dy`          | Movement since the begin, in touchpad units.                              |
| `scale`, `rotation` | The spread of a pinch, where 1 is unchanged, and its rotation in radians. |
| `progress`          | 0 to 1: how far the gesture has gone its own way.                         |

`WorkspaceSlideBinding` is the stock three-finger slide between workspaces: the windows follow the fingers, and the slide
commits when the fingers lift past `commit`, a fraction from 0 to 1 that defaults to 0.25. Its `travel` is the distance in
touchpad units that equals a full slide, and `duration` the length in milliseconds of the slide that finishes it. Install
it with `new WorkspaceSlideBinding({ trigger: "Swipe3" }).install()`.

To run a function when a gesture ends, bind a function to the gesture with `input.bind`, or use `SwipeActionBinding` and
`PinchActionBinding` for one function a direction. The named actions and `Gestures` are in
[Window policy](/js-window-policy#gesture-actions).

## Add a binding at run time

A drag or gesture binding goes to the seat with `gnoblin.input.primary.bindDrag(trigger, binding)` or `bindGesture(trigger,
binding)`. `binding.unbind()` on the `Binding` it returns removes it, and the compositor's claim goes when no binding needs
it.

```ts
const nudge = gnoblin.input.primary.bindDrag("Alt+MouseMiddle+Drag", new Nudge(), { id: "nudge" });
nudge.unbind();
```

A `BindingCollection`, such as `context.bindings` or `device.bindings`, holds the bindings by `id`: `bind`, `bindDrag`,
`bindGesture`, `find`, `has`, `remove`, `clear`, `list` and `length`. Binding with a taken id, or a trigger that is already
bound at the same priority, throws.
