# Workspaces

A workspace is a `Workspace` object that holds windows and lays them out. `gnoblin.workspaces` is the collection of them.
Declare workspaces in `defineConfig`, then read them, show one and change their windows from a script.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    workspaces: [
        { id: "web", layout: "master-stack" },
        { id: "chat", layout: "grid" },
    ],
    keys: {
        "Super+Tab": ({ gnoblin }) => {
            const current = gnoblin.workspaces.find((workspace) => workspace.active);
            const next = gnoblin.workspaces.at((current?.index ?? -1) + 1) ?? gnoblin.workspaces.at(0);
            if (next) {
                gnoblin.show(next, { duration: 0.25 });
            }
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+Tab` slides to the next workspace, and back to the first after the
last. The [defineConfig reference](/typescript-config#workspaces) lists every key of a declared workspace.

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
| `sets`                     | Named groups of workspaces. See [Workspace sets](#workspace-sets).               |
| `activateSet(name)`        | Makes a set live. Returns `false` for an unknown set.                            |

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

| Option     | Values             | Default                      | Meaning                                                  |
| ---------- | ------------------ | ---------------------------- | -------------------------------------------------------- |
| `duration` | seconds, 0 or more | 0, an instant switch         | How long the windows take to slide to the new workspace. |
| `progress` | 0 to 1             | 0                            | How much of the slide a touchpad gesture already made.   |
| `output`   | an `Output`        | the output under the pointer | The output to show the workspace on.                     |

A slide takes no time while the seat asks for reduced motion. See [Tweens and easing](tweens#reduced-motion).

## Layouts

`workspace.layout` decides where the windows of a workspace go. Give a stock layout by name in `defineConfig`, or a layout
object when you need its options. The names and options of the stock layouts are in the
[defineConfig reference](/typescript-config#workspaces).

To place windows your own way, extend `Layout` and return one placement for each window from `compute`. It runs when the
workspace changes, not on every frame, and a throw leaves the last placement in place.

```ts
import { Layout, defineConfig } from "gnoblin";
import type { LayoutContext, Placement } from "gnoblin";

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

defineConfig({ workspaces: [{ id: "columns", layout: new Columns() }] });
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

A layout may also define `onClientAdded(client, context)`, `onClientRemoved(client, context)` and `onAreaChanged(oldArea,
newArea)`. A layout's `bindings` collection holds the bindings that act on the windows of its workspace, such as the drag that
moves a floating window: see [Input handlers](input-handlers).

## Workspace sets

A workspace set is a named group of workspaces that are shown together, each on its own output. Declare them in
`defineConfig` and make one live with `activateSet`.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    workspaces: [{ id: "1" }, { id: "2" }, { id: "3" }],
    workspaceSets: [
        {
            name: "work",
            entries: [
                { workspace: "2", output: "DP-1" },
                { workspace: "3", output: "HDMI-1" },
            ],
        },
    ],
    keys: {
        "Super+w": ({ gnoblin }) => {
            gnoblin.workspaces.activateSet("work");
        },
    },
});
```

Each entry shows its workspace on its output. An entry with no `output`, or whose output is not connected, shows on the
primary output, and the shell reports the fallback. A list that does not check refuses the configuration: an empty or
repeated name, a set with no entries, a workspace that is not declared, or two entries for one output.

| Call                                     | Does                                                                     |
| ---------------------------------------- | ------------------------------------------------------------------------ |
| `gnoblin.workspaces.activateSet(name)`   | Makes the set live. Returns `false`, and reports, for an unknown set.    |
| `gnoblin.workspaces.sets.activate(name)` | The same, but throws a `WorkspaceSetError` instead of returning `false`. |
| `gnoblin.workspaces.sets.define(list)`   | Replaces the sets from a script. Validated as in `defineConfig`.         |
| `gnoblin.workspaces.sets.list`           | The defined sets.                                                        |
| `gnoblin.workspaces.sets.active`         | The name of the live set, or `null`.                                     |

Making the live set live again changes nothing.

## Workspace events

A workspace listener's `onAdded` and `onRemoved` hear workspaces join and leave the collection. The signals
`gnoblin.workspace.created` and `gnoblin.workspace.removed` are in [Events](/events/desktop-state).
