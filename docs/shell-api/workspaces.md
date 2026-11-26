# Workspaces

A workspace is a `Workspace` object that holds windows and lays them out. `gnoblin.workspaces` is the collection of them.
Add workspaces with `gnoblin.workspaces.add`, then read them, show one and change their windows from a script.

::: tabs

```typescript
import { GridLayout, MasterStackLayout, Workspace } from "gnoblin-std";

gnoblin.workspaces.add(new Workspace("web", { layout: new MasterStackLayout() }));
gnoblin.workspaces.add(new Workspace("chat", { layout: new GridLayout() }));

gnoblin.input.primary.bind("Super+Tab", () => {
    const list = gnoblin.workspaces.list;
    const current = list.find((workspace) => workspace.active);
    const next = list[(current?.index ?? -1) + 1] ?? list[0];
    if (next) {
        gnoblin.workspaces.show(next);
    }
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import GridLayout, MasterStackLayout, Workspace, Workspaces

gnoblin = Gnoblin.get()
workspaces = Workspaces.get()
workspaces.add(Workspace("web", layout=MasterStackLayout()))
workspaces.add(Workspace("chat", layout=GridLayout()))


def cycle(*_args):
    items = workspaces.list
    current = next((i for i, workspace in enumerate(items) if workspace.active), -1)
    items[(current + 1) % len(items)].show(None)


gnoblin.input.primary.bind("Super+Tab", cycle)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{GridLayout, MasterStackLayout, Workspace, WorkspaceOptions, Workspaces};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let workspaces = Workspaces::get(&objects)?;
    workspaces.add(&Workspace::new_with(
        &objects,
        "web",
        WorkspaceOptions { name: None, gaps: None, layout: Some(MasterStackLayout::new(&objects)?.as_layout()?) },
    )?)?;
    workspaces.add(&Workspace::new_with(
        &objects,
        "chat",
        WorkspaceOptions { name: None, gaps: None, layout: Some(GridLayout::new(&objects)?.as_layout()?) },
    )?)?;

    g.bind(
        "Super+Tab",
        Some(move |_event| {
            let items = workspaces.list()?;
            let current = items.iter().position(|workspace| workspace.active().unwrap_or(false));
            let next = current.map_or(0, |i| i + 1) % items.len();
            items[next].show(None)?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the script in your shell config directory, `~/.config/gnoblin/`. `Super+Tab` shows the next workspace, and the first one after the last.

A `Workspace` takes its `id` first, then its optional options:

- TypeScript: an object as the second argument, with `name`, `layout` and `gaps`.
- Python: keyword arguments of the same names.
- Rust: fields of `WorkspaceOptions`.

## Read workspaces

`gnoblin.workspaces` holds the workspaces in order. Every read of one workspace returns the same object. To select
workspaces by their fields, use [`gnoblin.find("Workspace", filter)`](find).

The tables name members as TypeScript spells them. Python and Rust spell the same members in snake_case.

| Member of the collection   | Meaning                                                                              |
| -------------------------- | ------------------------------------------------------------------------------------ |
| `list`                     | The workspaces as a list, in order. Use `find`, `filter`, `map` and iteration on it. |
| `length`                   | How many workspaces there are.                                                       |
| `byId(id)`                 | The workspace with that id, or none.                                                 |
| `ofClient(client)`         | The workspace a window is on, or none.                                               |
| `ofOutput(output)`         | The workspace an output shows, or none.                                              |
| `add(workspace)`           | Adds a workspace. A second workspace with the same id is refused.                    |
| `remove(workspace)`        | Removes a workspace. Returns `false` for the last one, or one that is not there.     |
| `clear()`                  | Removes every workspace. Add one afterwards: windows need a workspace.               |
| `show(workspace, options)` | Shows a workspace.                                                                   |
| `showById(id, options)`    | Shows the workspace with that id.                                                    |
| `area`                     | The area windows are laid out in on the primary output, or none.                     |

Removing a workspace moves its windows to the workspace before it (the one after, for the first) and shows that
workspace where the removed one was. No window closes.

## The `Workspace` object

| Member          | Type            | Meaning                                                                                |
| --------------- | --------------- | -------------------------------------------------------------------------------------- |
| `id`            | string          | The workspace's identity. Unique in the collection.                                    |
| `name`          | string          | What the workspace is called where a name is shown. Writable.                          |
| `index`         | integer         | The position in `gnoblin.workspaces`, or -1 outside it.                                |
| `active`        | boolean         | Whether an output shows it. With several outputs several are active.                   |
| `output`        | integer         | The number of the output that shows it, or 0 while none does.                          |
| `layout`        | `Layout`        | The layout that places its windows. Assign a layout object.                            |
| `gaps`          | inner and outer | Pixels between windows and between windows and the edge. Assign both together.         |
| `clients`       | collection      | The windows on this workspace. `add(client)` moves a window here.                      |
| `attached`      | boolean         | Whether the workspace is in `gnoblin.workspaces`.                                      |
| `show(options)` | method          | Shows the workspace on the output under the pointer.                                   |
| `set(values)`   | method          | Writes properties in one transition. See [Tweens and easing](tweens#animate-a-window). |
| `invalidate()`  | method          | Lays the workspace out again.                                                          |
| `walk(step)`    | method          | Moves the focus `step` windows along the workspace: 1 is the next, -1 the previous.    |

`walk` does nothing when no window of the workspace has the focus, and a step past either end leaves the focus where it is.
A workspace is the script's own object: it stays usable after it leaves the collection.

## Show a workspace

`gnoblin.workspaces.show(workspace, options?)` shows a workspace, and `showById(id, options?)` does the same for an id.
`gnoblin.show(workspace, options?)` is the same call. It acts on the output under the pointer, or the primary output when
no pointer says. `Workspace.show` and `Output.show` are the same call with a different target: see
[Outputs](outputs#one-workspace-on-one-output).

::: tabs

```typescript
import { Workspace } from "gnoblin-std";

const web = new Workspace("web");
gnoblin.workspaces.add(web);
gnoblin.workspaces.show(web, { duration: 250 });
```

```python
from gnoblin.objects.std import ShowOptions, Workspace, Workspaces

web = Workspace("web")
Workspaces.get().add(web)
web.show(ShowOptions(duration=250.0))
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{ShowOptions, Workspace, WorkspaceOptions, Workspaces};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let web = Workspace::new_with(&objects, "web", WorkspaceOptions { name: None, gaps: None, layout: None })?;
    Workspaces::get(&objects)?.add(&web)?;
    web.show(Some(&ShowOptions { duration: 250.0, progress: 0.0, output: None }))?;
    Ok(())
}
```

:::

| Option     | Values                  | Default                      | Meaning                                                  |
| ---------- | ----------------------- | ---------------------------- | -------------------------------------------------------- |
| `duration` | milliseconds, 0 or more | 0, an instant switch         | How long the windows take to slide to the new workspace. |
| `progress` | 0 to 1                  | 0                            | How much of the slide a touchpad gesture already made.   |
| `output`   | an output's number      | the output under the pointer | The output to show the workspace on.                     |

A slide takes no time while the seat asks for reduced motion. See [Tweens and easing](tweens#reduced-motion).

## Layouts

`workspace.layout` decides where the windows of a workspace go. Give it a layout object, and its options in the
constructor. Python takes the same options as keyword arguments in snake_case, such as `master_count` and `cascade_step`.

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

To place windows your own way, write a layout that returns one placement for each window from `compute`. It runs when the
workspace changes, not on every frame, and a throw leaves the last placement in place.

::: tabs

```typescript
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

```python
from gnoblin.objects.std import Layout, LayoutPlacement, LayoutRect, Workspace, Workspaces


class Columns(Layout):
    def compute(self, context):
        area = context.area
        count = len(context.children)
        return [
            LayoutPlacement(
                node=node,
                tiled=True,
                bounds=LayoutRect(
                    x=int(area.x + index * area.width / count),
                    y=area.y,
                    width=int(area.width / count),
                    height=area.height,
                ),
            )
            for index, node in enumerate(context.children)
        ]


Workspaces.get().add(Workspace("columns", layout=Columns()))
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{
    LayoutContext, LayoutHandlers, LayoutPlacement, LayoutRect, Workspace, WorkspaceOptions, Workspaces,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let columns = LayoutHandlers {
        compute: Box::new(|context: LayoutContext| -> gnoblin::Result<Vec<LayoutPlacement>> {
            let (x, y, width, height) = (context.area.x, context.area.y, context.area.width, context.area.height);
            let count = context.children.len().max(1) as f64;
            Ok(context
                .children
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let mut placement = LayoutPlacement::new(node.clone());
                    placement.tiled = true;
                    placement.bounds = Some(LayoutRect {
                        x: x + index as f64 * width / count,
                        y,
                        width: width / count,
                        height,
                    });
                    placement
                })
                .collect())
        }),
        client_added: None,
        client_removed: None,
        area_changed: None,
        clients_swapped: None,
        revision: None,
        find_split: None,
        move_split: None,
        reset_split: None,
    }
    .implement(&objects)?;

    Workspaces::get(&objects)?.add(&Workspace::new_with(
        &objects,
        "columns",
        WorkspaceOptions { name: None, gaps: None, layout: Some(columns) },
    )?)?;
    Ok(())
}
```

:::

`context` has these fields:

| Field       | Meaning                                                 |
| ----------- | ------------------------------------------------------- |
| `area`      | The rectangle to fill: the output's area less its bars. |
| `children`  | The windows to place, in order.                         |
| `focused`   | The focused window of the workspace, or none.           |
| `innerGap`  | The workspace's gap between windows.                    |
| `outerGap`  | The workspace's gap between windows and the edge.       |
| `workspace` | The id of the workspace.                                |

A placement has the window as `node` and any of these, which it leaves unchanged when absent:

| Field                | Meaning                                                                                |
| -------------------- | -------------------------------------------------------------------------------------- |
| `bounds`             | The window's rectangle: x, y, width and height.                                        |
| `visible`            | `false` hides the window. A monocle hides all but one.                                 |
| `tiled`              | `true` marks the window as tiled, so it drops the border and rounding it draws itself. |
| `z`                  | Depth. Negative is in front of the camera.                                             |
| `rotation`, `scale`  | Rotation in radians and scale along each axis, as three numbers.                       |
| `offsetX`, `offsetY` | A shift of the window's picture from its place.                                        |

A layout may also define these hooks. TypeScript names them in camelCase; Python defines the same names in snake_case as
methods of its `Layout` subclass, and Rust sets them as the fields of `LayoutHandlers`, also in snake_case.

| Hook                                  | Runs when                                                                                              |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `clientAdded(context, client)`        | A window joins the workspace.                                                                          |
| `clientRemoved(context, client)`      | A window leaves the workspace.                                                                         |
| `areaChanged(before, now)`            | The area changes.                                                                                      |
| `clientsSwapped(a, b)`                | `workspace.clients.swap(a, b)` trades the places of two windows.                                       |
| `findSplit(client, x, y)`             | A drag asks which dividers it would move. Returns a list, empty when there is none.                    |
| `setSplit(token, ratio, crossRatio?)` | Moves a divider. Python and Rust call it `set_split`, and Rust's layout handler calls it `move_split`. |
| `resetSplit(token)`                   | Puts a divider back where it was.                                                                      |
| `revision()`                          | Returns a number that changes when the layout's own state changes, so the workspace lays out again.    |

A layout's bindings are the drag bindings you install for the windows of its workspace, such as the drag that moves a
floating window or swaps two tiles: see [Input handlers](input-handlers).

## Workspace events

Each event has the same meaning in every language. Install a handler with `on` in TypeScript and Python, and with the
`on_` method in Rust, which returns a listener you keep for as long as you want the handler.

| Event           | TypeScript        | Python             | Rust                  | Runs with                  |
| --------------- | ----------------- | ------------------ | --------------------- | -------------------------- |
| Id changed      | `idChanged`       | `id_changed`       | `on_id_changed`       | the new id                 |
| Name changed    | `nameChanged`     | `name_changed`     | `on_name_changed`     | the new name               |
| Gaps changed    | `gapsChanged`     | `gaps_changed`     | `on_gaps_changed`     | the new gaps               |
| Layout changed  | `layoutChanged`   | `layout_changed`   | `on_layout_changed`   | the new layout             |
| Clients changed | `clientsChanged`  | `clients_changed`  | `on_clients_changed`  | the workspace's windows    |
| Attached        | `attachedChanged` | `attached_changed` | `on_attached_changed` | whether it is in the list  |
| Active          | `activeChanged`   | `active_changed`   | `on_active_changed`   | whether an output shows it |
| Index           | `indexChanged`    | `index_changed`    | `on_index_changed`    | the new position           |
| Output          | `outputChanged`   | `output_changed`   | `on_output_changed`   | the output's number        |
| Shown           | `shown`           | `shown`            | `on_shown`            | the output's number        |
| Hidden          | `hidden`          | `hidden`           | `on_hidden`           | nothing                    |
| Window added    | `clientAdded`     | `client_added`     | `on_client_added`     | the window                 |
| Window removed  | `clientRemoved`   | `client_removed`   | `on_client_removed`   | the window                 |

The collection `gnoblin.workspaces` has its own events: `list_changed` (`listChanged`, `on_list_changed`) when its list
changes, `length_changed` (`lengthChanged`, `on_length_changed`) when its count changes, `area_changed` (`areaChanged`,
`on_area_changed`) when its area changes, and `added` and `removed`, which hear workspaces join and leave the collection.

The signals `gnoblin.workspace.created` and `gnoblin.workspace.removed` are in [Events](/events/desktop-state).
