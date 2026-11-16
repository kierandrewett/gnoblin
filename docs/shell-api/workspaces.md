# Workspaces

A workspace is a `Workspace` object that holds windows and lays them out. `gnoblin.workspaces` is the collection of them.
Add workspaces with `gnoblin.workspaces.add`, then read them, show one and change their windows from a script.

```ts
import { GridLayout, MasterStackLayout, Workspace } from "gnoblin-std";

gnoblin.workspaces.add(new Workspace({ id: "web", layout: new MasterStackLayout() }));
gnoblin.workspaces.add(new Workspace({ id: "chat", layout: new GridLayout() }));

gnoblin.input.primary.bind("Super+Tab", () => {
    const current = gnoblin.workspaces.find((workspace) => workspace.active);
    const next = gnoblin.workspaces.at((current?.index ?? -1) + 1) ?? gnoblin.workspaces.at(0);
    if (next) {
        gnoblin.show(next, { duration: 250 });
    }
});
```

Put the file in `~/.config/gnoblin/shell.ts`. `Super+Tab` slides to the next workspace, and back to the first after the
last. A `Workspace` takes `id`, `name`, `layout` and `gaps`.

## Read workspaces

`gnoblin.workspaces` is a collection like [`gnoblin.clients`](windows#read-windows), with `length`, `at(index)`,
`get(id)`, `find`, `filter`, `map`, `where` and iteration. Every read of one workspace returns the same object. To select
workspaces by their fields, use [`gnoblin.find("Workspace", filter)`](find).

| Member of the collection   | Meaning                                                                          |
| -------------------------- | -------------------------------------------------------------------------------- |
| `add(workspace)`           | Adds a `new Workspace({ id })`. A second workspace with the same id throws.      |
| `remove(workspaceOrId)`    | Removes a workspace. Returns `false` for the last one, or one that is not there. |
| `clear()`                  | Removes every workspace. Add one afterwards: windows need a workspace.           |
| `show(workspace, options)` | Shows a workspace. The same as `gnoblin.show`.                                   |
| `area`                     | The area windows are laid out in on the primary output, or `null`.               |

Removing a workspace moves its windows to the workspace before it (the one after, for the first) and shows that
workspace where the removed one was. No window closes.

## The `Workspace` object

| Member          | Type               | Meaning                                                                             |
| --------------- | ------------------ | ----------------------------------------------------------------------------------- |
| `id`            | string             | The workspace's identity. Unique in the collection.                                 |
| `name`          | string             | What the workspace is called where a name is shown. Writable.                       |
| `index`         | number             | The position in `gnoblin.workspaces`, or -1 outside it.                             |
| `active`        | boolean            | Whether an output shows it. With several outputs several are active.                |
| `output`        | `Output` or `null` | The output that shows it.                                                           |
| `layout`        | `Layout`           | The layout that places its windows. Assign a layout or a stock layout's name.       |
| `gaps`          | `{ inner, outer }` | Pixels between windows and between windows and the edge. Assign a partial object.   |
| `clients`       | collection         | The windows on this workspace. `add(client)` moves a window here.                   |
| `show(options)` | method             | Shows the workspace on the output under the pointer.                                |
| `walk(step)`    | method             | Moves the focus `step` windows along the workspace: 1 is the next, -1 the previous. |

`walk` does nothing when no window of the workspace has the focus, and a step past either end leaves the focus where it is.
A workspace is the script's own object: it stays usable after it leaves the collection.

## Show a workspace

`gnoblin.show(workspace, options?)` shows a workspace, given as the object or by its id. It acts on the output under the
pointer, or the primary output when no pointer says. `Workspace.show`, `Output.show` and `gnoblin.workspaces.show` are the
same call with a different target: see [Outputs](outputs#one-workspace-on-one-output).

| Option     | Values                  | Default                      | Meaning                                                  |
| ---------- | ----------------------- | ---------------------------- | -------------------------------------------------------- |
| `duration` | milliseconds, 0 or more | 0, an instant switch         | How long the windows take to slide to the new workspace. |
| `progress` | 0 to 1                  | 0                            | How much of the slide a touchpad gesture already made.   |
| `output`   | an `Output`             | the output under the pointer | The output to show the workspace on.                     |

A slide takes no time while the seat asks for reduced motion. See [Tweens and easing](tweens#reduced-motion).

## Layouts

`workspace.layout` decides where the windows of a workspace go. Give it a layout object, and its options in the
constructor:

| Layout              | Places windows                                                                          | Options                                                                                                             |
| ------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `FloatingLayout`    | Where they are put.                                                                     | `placement` (`smart`, `center`, `cascade`), `cascadeStep`, `triggers`                                               |
| `MasterStackLayout` | A master column and a stack.                                                            | `masterCount`, `ratio`, `gap`, `orientation`, `triggers`                                                            |
| `GridLayout`        | In equal cells.                                                                         | `columns` (a number or `"auto"`), `gap`, `triggers`                                                                 |
| `MonocleLayout`     | One at a time, filling the area. `KioskLayout` places the same.                         | none                                                                                                                |
| `DwindleLayout`     | By halving the cell of the focused window for each new one, as Hyprland's dwindle does. | `split`, `ratio`, `gap`, `triggers` and Hyprland's options: [Dwindle like Hyprland](/recipes/dwindle-like-hyprland) |
| `StripLayout`       | In a scrolling strip of columns, the view following the focus.                          | `column`                                                                                                            |

To place windows your own way, extend `Layout` and return one placement for each window from `compute`. It runs when the
workspace changes, not on every frame, and a throw leaves the last placement in place.

```ts
import { Layout, Workspace } from "gnoblin-std";
import type { LayoutContext, Placement } from "gnoblin-std";

class Columns extends Layout {
    compute(context: LayoutContext): Placement[] {
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

gnoblin.workspaces.add(new Workspace({ id: "columns", layout: new Columns() }));
```

`context` has these fields:

| Field       | Meaning                                                 |
| ----------- | ------------------------------------------------------- |
| `area`      | The rectangle to fill: the output's area less its bars. |
| `children`  | The windows to place, in order.                         |
| `focused`   | The focused window of the workspace, or `null`.         |
| `gap`       | The workspace's `{ inner, outer }` gaps.                |
| `workspace` | The workspace.                                          |

A `Placement` has the window as `node` and any of these, which it leaves unchanged when absent:

| Field               | Meaning                                                                                |
| ------------------- | -------------------------------------------------------------------------------------- |
| `bounds`            | The window's rectangle, `{ x, y, width, height }`.                                     |
| `visible`           | `false` hides the window. A monocle hides all but one.                                 |
| `tiled`             | `true` marks the window as tiled, so it drops the border and rounding it draws itself. |
| `z`                 | Depth. Negative is in front of the camera.                                             |
| `rotation`, `scale` | Rotation in radians and scale along each axis, as three numbers.                       |

A layout may also define these hooks:

- `onClientAdded(client, context)` and `onClientRemoved(client, context)` run when a window joins or leaves the workspace.
- `onAreaChanged(oldArea, newArea)` runs when the area changes.
- `onClientsSwapped(a, b)` runs when `workspace.clients.swap(a, b)` trades the places of two windows.
- `findSplit(client, point)` returns the divider a drag would move, or `null`.

A layout's `bindings` collection holds the bindings that act on the windows of its workspace, such as the drag that moves a
floating window or swaps two tiles: see [Input handlers](input-handlers).

## Workspace events

A workspace listener's `onAdded` and `onRemoved` hear workspaces join and leave the collection. The signals
`gnoblin.workspace.created` and `gnoblin.workspace.removed` are in [Events](/events/desktop-state).
