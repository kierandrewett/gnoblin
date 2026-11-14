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

`dragConstraint` is `work_area`, which keeps a window inside the work area, or `minimum_strip`, which lets a window leave
until a strip of 75 pixels stays visible. Set it in `defineConfig`.

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
