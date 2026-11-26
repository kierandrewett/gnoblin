# Workspaces

A workspace is a `Workspace` object that holds windows and lays them out. `gnoblin.workspaces` is the collection of them.
Add workspaces with `gnoblin.workspaces.add`, then read them, show one and change their windows from a script.

```ts
import { GridLayout, MasterStackLayout, Workspace } from "gnoblin-std";

gnoblin.workspaces.add(new Workspace("web", { layout: new MasterStackLayout() }));
gnoblin.workspaces.add(new Workspace("chat", { layout: new GridLayout() }));

gnoblin.input.primary.bind("Super+Tab", () => {
    const list = gnoblin.workspaces.list;
    const current = list.find((workspace) => workspace.active);
    const next = list[(current?.index ?? -1) + 1] ?? list[0];
    if (next) {
        gnoblin.workspaces.show(next, { duration: 250 });
    }
});
```

Put the file in `~/.config/gnoblin/shell.ts`. `Super+Tab` slides to the next workspace, and back to the first after the
last. A `Workspace` takes its `id` first, then an object with `name`, `layout` and `gaps`.

## Read workspaces

`gnoblin.workspaces` holds the workspaces in order. Every read of one workspace returns the same object. To select
workspaces by their fields, use [`gnoblin.find("Workspace", filter)`](find).

| Member of the collection   | Meaning                                                                                 |
| -------------------------- | --------------------------------------------------------------------------------------- |
| `list`                     | The workspaces as an array, in order. Use `find`, `filter`, `map` and `for...of` on it. |
| `length`                   | How many workspaces there are.                                                          |
| `byId(id)`                 | The workspace with that id, or `undefined`.                                             |
| `ofClient(client)`         | The workspace a window is on, or `undefined`.                                           |
| `ofOutput(output)`         | The workspace an output shows, or `undefined`.                                          |
| `add(workspace)`           | Adds a `new Workspace(id)`. A second workspace with the same id throws.                 |
| `remove(workspace)`        | Removes a workspace. Returns `false` for the last one, or one that is not there.        |
| `clear()`                  | Removes every workspace. Add one afterwards: windows need a workspace.                  |
| `show(workspace, options)` | Shows a workspace.                                                                      |
| `showById(id, options)`    | Shows the workspace with that id.                                                       |
| `area`                     | The area windows are laid out in on the primary output, or `undefined`.                 |

Removing a workspace moves its windows to the workspace before it (the one after, for the first) and shows that
workspace where the removed one was. No window closes.

## The `Workspace` object

| Member          | Type               | Meaning                                                                                |
| --------------- | ------------------ | -------------------------------------------------------------------------------------- |
| `id`            | string             | The workspace's identity. Unique in the collection.                                    |
| `name`          | string             | What the workspace is called where a name is shown. Writable.                          |
| `index`         | number             | The position in `gnoblin.workspaces`, or -1 outside it.                                |
| `active`        | boolean            | Whether an output shows it. With several outputs several are active.                   |
| `output`        | number             | The number of the output that shows it, or 0 while none does.                          |
| `layout`        | `Layout`           | The layout that places its windows. Assign a layout object.                            |
| `gaps`          | `{ inner, outer }` | Pixels between windows and between windows and the edge. Assign an object.             |
| `clients`       | collection         | The windows on this workspace. `add(client)` moves a window here.                      |
| `attached`      | boolean            | Whether the workspace is in `gnoblin.workspaces`.                                      |
| `show(options)` | method             | Shows the workspace on the output under the pointer.                                   |
| `set(values)`   | method             | Writes properties in one transition. See [Tweens and easing](tweens#animate-a-window). |
| `invalidate()`  | method             | Lays the workspace out again.                                                          |
| `walk(step)`    | method             | Moves the focus `step` windows along the workspace: 1 is the next, -1 the previous.    |

`walk` does nothing when no window of the workspace has the focus, and a step past either end leaves the focus where it is.
A workspace is the script's own object: it stays usable after it leaves the collection.

## Show a workspace

`gnoblin.workspaces.show(workspace, options?)` shows a workspace, and `showById(id, options?)` does the same for an id.
`gnoblin.show(workspace, options?)` is the same call. It acts on the output under the pointer, or the primary output when
no pointer says. `Workspace.show` and `Output.show` are the same call with a different target: see
[Outputs](outputs#one-workspace-on-one-output).

| Option     | Values                  | Default                      | Meaning                                                  |
| ---------- | ----------------------- | ---------------------------- | -------------------------------------------------------- |
| `duration` | milliseconds, 0 or more | 0, an instant switch         | How long the windows take to slide to the new workspace. |
| `progress` | 0 to 1                  | 0                            | How much of the slide a touchpad gesture already made.   |
| `output`   | an output's number      | the output under the pointer | The output to show the workspace on.                     |

A slide takes no time while the seat asks for reduced motion. See [Tweens and easing](tweens#reduced-motion).

## Layouts

`workspace.layout` decides where the windows of a workspace go. Give it a layout object, and its options in the
constructor:

| Layout              | Places windows                                                                                      | Options                                                                                                 |
| ------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `FloatingLayout`    | Where they are put.                                                                                 | `placement` (`smart`, `center`, `cascade`), `cascadeStep`                                               |
| `MasterStackLayout` | A master column and a stack.                                                                        | `masterCount`, `ratio`, `gap`, `orientation` (`left`, `right`, `top`, `bottom`)                         |
| `GridLayout`        | In equal cells.                                                                                     | `columns` (0 chooses from the window count), `fill` (`row` or `column`), `gap`                          |
| `MonocleLayout`     | One at a time, filling the area. `KioskLayout` is another name for it.                              | none                                                                                                    |
| `DwindleLayout`     | By halving the cell of the focused window for each new one, as Hyprland's dwindle does.             | `split`, `ratio`, `gap` and Hyprland's options: [Dwindle like Hyprland](/recipes/dwindle-like-hyprland) |
| `StripLayout`       | In a scrolling strip of columns, the view following the focus.                                      | `column`                                                                                                |
| `ArcLayout`         | Along an arc round the viewer.                                                                      | `radius`, `spacing`                                                                                     |
| `CarouselLayout`    | On a rotating carousel, with `rotate(steps)` to turn it. `Carousel3dLayout` is another name for it. | `fov`, `cardWidth`, `cardHeight`                                                                        |

Which drag moves a window or swaps two tiles is not a layout option: install a `MoveBinding`, `ResizeBinding`,
`SwapBinding` or `SplitResizeBinding`. See [Input handlers](input-handlers).

To place windows your own way, extend `Layout` and return one placement for each window from `compute`. It runs when the
workspace changes, not on every frame, and a throw leaves the last placement in place.

```ts
import { Layout, Workspace } from "gnoblin-std";
import type { LayoutContextInit, LayoutPlacementInit } from "gnoblin-std";

class Columns extends Layout {
    compute(context: LayoutContextInit): LayoutPlacementInit[] {
        const { x, y, width, height } = context.area;
        const count = context.children.length;
        return context.children.map((node, index) => ({
            node,
            tiled: true,
            bounds: {
                x: Math.floor(x + (index * width) / count),
                y,
                width: Math.floor(width / count),
                height,
            },
        }));
    }
}

gnoblin.workspaces.add(new Workspace("columns", { layout: new Columns() }));
```

`context` has these fields:

| Field       | Meaning                                                 |
| ----------- | ------------------------------------------------------- |
| `area`      | The rectangle to fill: the output's area less its bars. |
| `children`  | The windows to place, in order.                         |
| `focused`   | The focused window of the workspace, or `undefined`.    |
| `innerGap`  | The workspace's gap between windows.                    |
| `outerGap`  | The workspace's gap between windows and the edge.       |
| `workspace` | The id of the workspace.                                |

A placement has the window as `node` and any of these, which it leaves unchanged when absent:

| Field                | Meaning                                                                                |
| -------------------- | -------------------------------------------------------------------------------------- |
| `bounds`             | The window's rectangle, `{ x, y, width, height }`.                                     |
| `visible`            | `false` hides the window. A monocle hides all but one.                                 |
| `tiled`              | `true` marks the window as tiled, so it drops the border and rounding it draws itself. |
| `z`                  | Depth. Negative is in front of the camera.                                             |
| `rotation`, `scale`  | Rotation in radians and scale along each axis, as three numbers.                       |
| `offsetX`, `offsetY` | A shift of the window's picture from its place.                                        |

A layout may also define these hooks:

- `clientAdded(context, client)` and `clientRemoved(context, client)` run when a window joins or leaves the workspace.
- `areaChanged(before, now)` runs when the area changes.
- `clientsSwapped(a, b)` runs when `workspace.clients.swap(a, b)` trades the places of two windows.
- `findSplit(client, x, y)` returns the dividers a drag would move, as a list that is empty when there is none.
- `setSplit(token, ratio, crossRatio?)` and `resetSplit(token)` move a divider and put it back.
- `revision()` returns a number that changes when the layout's own state changes, so the workspace lays out again.

A layout's bindings are the drag bindings you install for the windows of its workspace, such as the drag that moves a
floating window or swaps two tiles: see [Input handlers](input-handlers).

## Workspace events

`gnoblin.workspaces.on("added", handler)` and `on("removed", handler)` hear workspaces join and leave the collection. A
`Workspace` has the events `shown`, `hidden`, `clientAdded` and `clientRemoved`. The signals `gnoblin.workspace.created`
and `gnoblin.workspace.removed` are in [Events](/events/desktop-state).
