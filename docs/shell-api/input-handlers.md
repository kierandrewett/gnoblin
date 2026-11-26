# Input handlers

A key press or a click is a function. A pointer drag and a touchpad gesture take place over time, so they are binding
objects with hooks that run as the input starts, moves and ends. The stock shell moves and resizes windows, switches
workspaces and acts on gestures with these.

::: tabs

```typescript
import { PointerDragBinding, gnoblin } from "gnoblin-core/generated/objects";
import type { BindingEvent, Client } from "gnoblin-core/generated/objects";

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

gnoblin.input.primary?.bindDrag("Alt+MouseMiddle+Drag", new Nudge(), { id: "nudge" });
```

```python
from gnoblin.objects.model import BindingEvent, BindingOptions, Client, Gnoblin, PointerDragBinding

gnoblin = Gnoblin.get()


class Nudge(PointerDragBinding):
    def __init__(self) -> None:
        self.client: Client | None = None
        self.x = 0.0
        self.y = 0.0

    def on_start(self, event: BindingEvent) -> bool:
        if event.client is None:
            return False
        self.client = event.client
        self.x = event.client.x
        self.y = event.client.y
        return True

    def on_update(self, event: BindingEvent) -> None:
        if self.client is not None:
            self.client.x = self.x + (event.dx or 0.0)
            self.client.y = self.y + (event.dy or 0.0)

    def on_end(self, event: BindingEvent) -> None:
        self.client = None

    def on_cancel(self, event: BindingEvent) -> None:
        self.client = None


gnoblin.input.primary.bind_drag("Alt+MouseMiddle+Drag", Nudge(), BindingOptions(id="nudge"))
```

```rust
use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{BindingEvent, BindingOptions, Client, InputManager, PointerDragBindingHandlers};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let primary = InputManager::get(&objects)?.primary()?;
    let grab: Rc<RefCell<Option<(Client, f64, f64)>>> = Rc::new(RefCell::new(None));
    let (start, update, end, cancel) = (Rc::clone(&grab), Rc::clone(&grab), Rc::clone(&grab), Rc::clone(&grab));
    let handler = PointerDragBindingHandlers {
        on_start: Box::new(move |event: BindingEvent| {
            let Some(client) = event.client else {
                return Ok(false);
            };
            let (x, y) = (client.x()?, client.y()?);
            *start.borrow_mut() = Some((client, x, y));
            Ok(true)
        }),
        on_update: Some(Box::new(move |event: BindingEvent| {
            if let Some((client, x, y)) = update.borrow().as_ref() {
                client.set_x(*x + event.dx.unwrap_or(0.0))?;
                client.set_y(*y + event.dy.unwrap_or(0.0))?;
            }
            Ok(())
        })),
        on_end: Some(Box::new(move |_event: BindingEvent| {
            *end.borrow_mut() = None;
            Ok(())
        })),
        on_cancel: Some(Box::new(move |_event: BindingEvent| {
            *cancel.borrow_mut() = None;
            Ok(())
        })),
    }
    .implement(&objects)?;
    let options = BindingOptions {
        id: Some("nudge".to_owned()),
        priority: None,
        enabled: None,
        only: None,
        threshold: None,
    };
    primary.bind_drag("Alt+MouseMiddle+Drag", &handler, Some(&options))?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts`, the Python in `~/.config/gnoblin/shell.py`, and the Rust in a shell crate. Hold `Alt`, press the middle button on a window and drag: the window follows
the pointer.

## Drag bindings

Extend `PointerDragBinding` (a class in Python, a type in Rust) to handle a pointer drag. Give it to the drag call with
a trigger and options:

- TypeScript: `bindDrag`.
- Python and Rust: `bind_drag`.

The options are `id`, `priority`, `enabled` and `threshold`.

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

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import { MasterStackLayout, SplitResizeBinding, SwapBinding, Workspace } from "gnoblin-std/generated/objects";

gnoblin.workspaces.add(new Workspace("dev", { layout: new MasterStackLayout({ ratio: 0.6 }) }));
new SwapBinding({ trigger: "Alt+MouseLeft+Drag" }).install();
new SplitResizeBinding({ trigger: "Alt+MouseRight+Drag" }).install();
```

```python
from gnoblin.objects.std import MasterStackLayout, SplitResizeBinding, SwapBinding, Workspace, Workspaces

Workspaces.get().add(Workspace("dev", layout=MasterStackLayout(ratio=0.6)))
SwapBinding(trigger="Alt+MouseLeft+Drag").install()
SplitResizeBinding(trigger="Alt+MouseRight+Drag").install()
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::objects::{
    MasterStackLayout, MasterStackLayoutOptions, ModelRootStdExt, SplitResizeBinding, SplitResizeBindingOptions,
    SwapBinding, SwapBindingOptions, Workspace, WorkspaceOptions,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?.objects()?;
    let layout = MasterStackLayout::new_with(
        &std,
        MasterStackLayoutOptions { ratio: Some(0.6), master_count: None, gap: None, orientation: None },
    )?
    .as_layout()?;
    let dev = Workspace::new_with(&std, "dev", WorkspaceOptions { layout: Some(layout), ..WorkspaceOptions::default() })?;
    ModelRoot::get(&g.objects()?)?.workspaces()?.add(&dev)?;
    SwapBinding::new_with(
        &std,
        SwapBindingOptions { priority: None, trigger: Some("Alt+MouseLeft+Drag".to_owned()) },
    )?
    .install(None)?;
    SplitResizeBinding::new_with(
        &std,
        SplitResizeBindingOptions { priority: None, trigger: Some("Alt+MouseRight+Drag".to_owned()) },
    )?
    .install(None)?;
    Ok(())
}
```

:::

`SwapBinding` changes nothing while the pointer is down. When you let go over another tiled window, the two trade places in
`workspace.clients` and the layout computes once. A release outside every tile, over the window you hold, or a cancel
leaves the order as it was.

A floating, fullscreen or maximised window declines the swap, and so does a press with no window under it.

`SplitResizeBinding` asks the layout for the divider nearest the press, through `findSplit` (`find_split` in Python and Rust). The divider follows the pointer as far as the pointer travels, and the layout computes on each frame, so the windows move while
you drag. A release keeps the ratio, and a cancel puts it back. What a divider is depends on the layout:

- `MasterStackLayout` has one, between the master and the stack. Its `ratio` changes, held to 0.1 to 0.9.
- `DwindleLayout` has one for each split. The drag changes the ratio of the split it took and leaves the others.
  The layout's `ratio` is still what every split that nobody dragged uses. With smart resizing off the drag moves two
  dividers, the split above the tile and the nearest one of the other direction above that, each by the pointer's
  travel along its own axis: that split has a cross axis (`crossAxis` in TypeScript, `cross_axis` in Python and Rust).
- A layout with no divider declines. That is a grid, or a master stack with a single window.

To drag a divider of your own layout, return a list with one split from `findSplit` (`find_split`), and move the divider in
`setSplit` (`set_split`):

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import { Layout, SplitResizeBinding, Workspace } from "gnoblin-std/generated/objects";
import type { LayoutContextInit, LayoutPlacementInit, LayoutSplitInit } from "gnoblin-std/generated/objects";

class Halves extends Layout {
    ratio = 0.5;
    width = 0;
    private start = 0.5;
    private changes = 0;

    compute(context: LayoutContextInit): LayoutPlacementInit[] {
        const { x = 0, y = 0, width = 0, height = 0 } = context.area;
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

```python
from gnoblin.objects.model import Client
from gnoblin.objects.std import (
    Layout,
    LayoutContext,
    LayoutPlacement,
    LayoutRect,
    LayoutSplit,
    SplitAxis,
    SplitResizeBinding,
    Workspace,
    Workspaces,
)


class Halves(Layout):
    def __init__(self) -> None:
        self.ratio = 0.5
        self.width = 0.0
        self.start = 0.5
        self.changes = 0

    def compute(self, context: LayoutContext) -> list[LayoutPlacement]:
        area = context.area
        left = int(area.width * self.ratio)
        self.width = area.width
        placements = []
        for index, node in enumerate(context.children):
            if index == 0:
                bounds = LayoutRect(x=area.x, y=area.y, width=left, height=area.height)
            else:
                bounds = LayoutRect(x=area.x + left, y=area.y, width=area.width - left, height=area.height)
            placements.append(LayoutPlacement(node=node, bounds=bounds, tiled=True))
        return placements

    def find_split(self, client: Client, x: float, y: float) -> list[LayoutSplit]:
        self.start = self.ratio
        if self.width == 0:
            return []
        return [
            LayoutSplit(token=1, axis=SplitAxis.HORIZONTAL, direction=1, span=self.width, min=0.2, max=0.8, ratio=self.ratio)
        ]

    def set_split(self, token: int, ratio: float, cross_ratio: float | None = None) -> None:
        self.ratio = ratio
        self.changes += 1

    def reset_split(self, token: int) -> None:
        self.ratio = self.start
        self.changes += 1

    def revision(self) -> int:
        return self.changes


Workspaces.get().add(Workspace("halves", layout=Halves()))
SplitResizeBinding().install()
```

```rust
use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::objects::{Client, ModelRoot};
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::objects::{
    LayoutContext, LayoutHandlers, LayoutPlacement, LayoutRect, LayoutSplit, ModelRootStdExt, SplitAxis,
    SplitResizeBinding, SplitResizeBindingOptions, Workspace,
};

struct Halves {
    ratio: f64,
    width: f64,
    start: f64,
    changes: u64,
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?.objects()?;
    let halves = Rc::new(RefCell::new(Halves { ratio: 0.5, width: 0.0, start: 0.5, changes: 0 }));
    let compute = {
        let halves = Rc::clone(&halves);
        move |context: LayoutContext| {
            let mut state = halves.borrow_mut();
            let area = context.area;
            let left = (area.width * state.ratio).floor();
            state.width = area.width;
            Ok(context
                .children
                .into_iter()
                .enumerate()
                .map(|(index, node)| {
                    let bounds = if index == 0 {
                        LayoutRect { x: area.x, y: area.y, width: left, height: area.height }
                    } else {
                        LayoutRect { x: area.x + left, y: area.y, width: area.width - left, height: area.height }
                    };
                    let mut placement = LayoutPlacement::new(node);
                    placement.bounds = Some(bounds);
                    placement.tiled = true;
                    placement
                })
                .collect())
        }
    };
    let find_split = {
        let halves = Rc::clone(&halves);
        move |_client: Client, _x: f64, _y: f64| {
            let mut state = halves.borrow_mut();
            state.start = state.ratio;
            if state.width == 0.0 {
                return Ok(Vec::new());
            }
            Ok(vec![LayoutSplit {
                token: 1,
                axis: SplitAxis::Horizontal,
                direction: 1,
                span: state.width,
                min: 0.2,
                max: 0.8,
                ratio: state.ratio,
                cross_axis: None,
                cross_direction: 0,
                cross_span: 0.0,
                cross_min: 0.0,
                cross_max: 0.0,
                cross_ratio: 0.0,
            }])
        }
    };
    let move_split = {
        let halves = Rc::clone(&halves);
        move |_token: u64, ratio: f64, _cross_ratio: Option<f64>| {
            let mut state = halves.borrow_mut();
            state.ratio = ratio;
            state.changes += 1;
            Ok(())
        }
    };
    let reset_split = {
        let halves = Rc::clone(&halves);
        move |_token: u64| {
            let mut state = halves.borrow_mut();
            state.ratio = state.start;
            state.changes += 1;
            Ok(())
        }
    };
    let revision = {
        let halves = Rc::clone(&halves);
        move || Ok(halves.borrow().changes)
    };
    let layout = LayoutHandlers {
        compute: Box::new(compute),
        client_added: None,
        client_removed: None,
        area_changed: None,
        clients_swapped: None,
        revision: Some(Box::new(revision)),
        find_split: Some(Box::new(find_split)),
        move_split: Some(Box::new(move_split)),
        reset_split: Some(Box::new(reset_split)),
    }
    .implement(&std)?;
    let workspace = Workspace::new(&std, "halves")?;
    workspace.set_layout(&layout)?;
    ModelRoot::get(&g.objects()?)?.workspaces()?.add(&workspace)?;
    SplitResizeBinding::new_with(&std, SplitResizeBindingOptions { priority: None, trigger: None })?.install(None)?;
    Ok(())
}
```

:::

The fields of the divider mean this:

- `span` is the pixels of pointer travel that change the ratio by 1.
- `direction` is -1 when moving the pointer right or down shrinks the ratio.
- `token` names the divider in `setSplit` and `resetSplit`.
- `revision()` tells the workspace to compute again when the ratio changed.

Two more things are optional:

- A cross axis (`crossAxis` in TypeScript, `cross_axis` in Python and Rust) with its direction, span, min, max and ratio is a second divider on the other
  axis. The same drag moves both, each by the pointer's travel along its own axis, `setSplit` gets the second ratio as its third argument, and a cancel resets both.
- The layout's clients-swapped hook (`clientsSwapped` in TypeScript, `clients_swapped` in Python and Rust) runs when a swap trades two windows, for a layout
  that keeps its own record of who sits where. `DwindleLayout` uses it to trade two leaves of its tree.

To trade two windows from a script, swap them in the workspace's client collection. Both must be in the workspace.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";
import "gnoblin-std/generated/objects";

const dev = gnoblin.workspaces.byId("dev");
const [first, second] = dev?.clients?.members ?? [];
if (dev?.clients && first && second) {
    dev.clients.swap(first, second);
}
```

```python
from gnoblin.objects.std import Workspaces

dev = Workspaces.get().by_id("dev")
if dev is not None:
    members = dev.clients.members
    if len(members) >= 2:
        dev.clients.swap(members[0], members[1])
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::objects::ModelRootStdExt;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    if let Some(dev) = ModelRoot::get(&g.objects()?)?.workspaces()?.by_id("dev")? {
        if let Some(clients) = dev.clients()? {
            let members = clients.members()?;
            if let [first, second, ..] = members.as_slice() {
                clients.swap(first, second)?;
            }
        }
    }
    Ok(())
}
```

:::

## Gesture bindings

A touchpad gesture binding extends `GestureBinding`. Give it to the gesture call (`bindGesture` in TypeScript,
`bind_gesture` in Python and Rust) with a trigger and options. Its trigger is a gesture such as `Swipe3` or `Pinch2In`.
Its hooks:

- `onBegin` (`on_begin`), which is required.
- `onUpdate` (`on_update`), `onEnd` (`on_end`) and `onCancel` (`on_cancel`).

A gesture event adds these fields:

| Field               | Meaning                                                                   |
| ------------------- | ------------------------------------------------------------------------- |
| `phase`             | `start`, `update`, `end` or `cancel`.                                     |
| `fingers`           | How many fingers are down.                                                |
| `dx`, `dy`          | Movement since the begin, in touchpad units.                              |
| `scale`, `rotation` | The spread of a pinch, where 1 is unchanged, and its rotation in radians. |
| `progress`          | 0 to 1: how far the gesture has gone its own way.                         |

`WorkspaceSlideBinding` is the stock three-finger slide between workspaces: the windows follow the fingers, and the slide commits when the fingers lift past `commit`, a fraction from 0 to 1 that defaults to 0.25.

Its `travel` is the distance in touchpad units that equals a full slide, and `duration` the length in milliseconds of the slide that finishes it. Install it with `trigger` set to `Swipe3`.

To run a function when a gesture ends, bind a plain function to the gesture on the input context, or use `SwipeActionBinding` and
`PinchActionBinding` for one function a direction. The named actions and `Gestures` are in
[Window policy](/js-window-policy#gesture-actions).

## Add a binding at run time

A drag or gesture binding goes to the seat with `bindDrag` or `bindGesture` (`bind_drag` and `bind_gesture` in Python and Rust) on the primary input context. Unbinding the `Binding` it returns removes it, and the compositor's claim goes when no binding needs
it.

::: tabs

```typescript
import { PointerDragBinding, gnoblin } from "gnoblin-core/generated/objects";

class Decline extends PointerDragBinding {
    onStart(): boolean {
        return false;
    }
}

const nudge = gnoblin.input.primary?.bindDrag("Alt+MouseMiddle+Drag", new Decline(), { id: "nudge" });
nudge?.unbind();
```

```python
from gnoblin.objects.model import BindingEvent, BindingOptions, InputManager, PointerDragBinding


class Decline(PointerDragBinding):
    def on_start(self, event: BindingEvent) -> bool:
        return False


nudge = InputManager.get().primary.bind_drag("Alt+MouseMiddle+Drag", Decline(), BindingOptions(id="nudge"))
nudge.unbind()
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{BindingEvent, InputManager, PointerDragBindingHandlers};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let primary = InputManager::get(&objects)?.primary()?;
    let handler = PointerDragBindingHandlers {
        on_start: Box::new(|_event: BindingEvent| Ok(false)),
        on_update: None,
        on_end: None,
        on_cancel: None,
    }
    .implement(&objects)?;
    let nudge = primary.bind_drag("Alt+MouseMiddle+Drag", &handler, None)?;
    nudge.unbind()?;
    Ok(())
}
```

:::

A `BindingCollection` holds the bindings by `id`. Its members are `context.bindings` and `device.bindings`.

The methods are `bind`, `bindDrag`, `bindGesture` (`bind_drag` and `bind_gesture` in Python and Rust), `find`, `has`, `remove`, `clear`, `list` and `length`. Binding with a taken id, or a trigger that is already bound at the same priority, throws.
