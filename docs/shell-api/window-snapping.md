# Window snapping

Snapping tiles a window to an edge of the work area. Dropping a dragged window on an edge snaps it, and the stock move
binding does that. A script can snap a window by itself with `ClientState` and `Snap`, which `gnoblin-std` exports.

::: tabs

```typescript
import { ClientState } from "gnoblin-std";

const input = gnoblin.input.primary;
input.bind("Super+Shift+Left", () => {
    const client = input.focusedClient;
    const area = gnoblin.workspaces.area;
    if (client && area) {
        ClientState.of(client).snapTo("left", area, true);
    }
});
input.bind("Super+Shift+Down", () => {
    const client = input.focusedClient;
    if (client) {
        ClientState.of(client).unsnap();
    }
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import ClientState, Workspaces

gnoblin = Gnoblin.get()
workspaces = Workspaces.get()
seat = gnoblin.input.primary


def snap_left(event):
    client = seat.focused_client
    area = workspaces.area
    if client and area:
        ClientState.of(client).snap_to("left", area, True)


def unsnap(event):
    client = seat.focused_client
    if client:
        ClientState.of(client).unsnap()


seat.bind("Super+Shift+Left", snap_left)
seat.bind("Super+Shift+Down", unsnap)
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::objects::{ClientState, ModelRootStdExt, SnapZone};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?.objects()?;
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(input) = model.input()? else {
        return Ok(());
    };
    let Some(seat) = input.primary()? else {
        return Ok(());
    };
    let seat = Rc::new(seat);
    let workspaces = Rc::new(model.workspaces()?);

    let (focus, area_of, objects) = (Rc::clone(&seat), Rc::clone(&workspaces), std.clone());
    seat.bind(
        "Super+Shift+Left",
        Some(move |_| {
            if let (Some(client), Some(area)) = (focus.focused_client()?, area_of.area()?) {
                ClientState::of(&objects, &client)?.snap_to(SnapZone::Left, &area, Some(true))?;
            }
            Ok(true)
        }),
        None,
    )?;

    let (focus, objects) = (Rc::clone(&seat), std.clone());
    seat.bind(
        "Super+Shift+Down",
        Some(move |_| {
            if let Some(client) = focus.focused_client()? {
                ClientState::of(&objects, &client)?.unsnap()?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts`, the Python in `~/.config/gnoblin/shell.py`, and the Rust in a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+Shift+Left` tiles the focused window to the left
half of the work area, and `Super+Shift+Down` puts it back where it floated.

## Zones

A snap zone is the part of the work area a window tiles to:

| Zone    | The window fills                | Pointer is within 4 pixels of |
| ------- | ------------------------------- | ----------------------------- |
| `left`  | The left half of the work area. | The left edge.                |
| `right` | The right half.                 | The right edge.               |
| `top`   | The whole work area.            | The top edge.                 |

A corner counts as the left or right edge before the top. The halves are whole pixels.

Edge tiling is on. The older `edgeTiling` and `dragConstraint` settings have no object in the API, so a file cannot turn them off (see [Settings with no object](/js-settings)).

The stock move drag, `MoveBinding`, snaps a window dropped within `snapMargin` pixels of an edge. The default is 4, and
`snap: false` never snaps.

## Functions

| Function                                                 | Does                                                                                                   |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `Snap.edgeZone(x, y, area, margin?)`                     | The zone a point is within `margin` pixels of, as `{ zone }`, or `undefined`. `margin` defaults to 4.  |
| `Snap.tileRect(zone, area)`                              | The rectangle a window tiles to in a zone.                                                             |
| `ClientState.of(client).snapTo(zone, area, edgeTiling?)` | Tiles a window. Returns `false`, and moves nothing, when edge tiling is off or the zone does not tile. |
| `ClientState.of(client).unsnap()`                        | Puts a snapped window back where it floated. Returns `false` when it is not snapped.                   |
| `ClientState.of(client).snap`                            | The snap of a window: its `zone` and the rectangle it floated at, or `undefined`.                      |
| `ClientState.of(client).forgetSnap()`                    | Forgets the snap and leaves the window where it is.                                                    |

Each language spells the same members its own way:

- Python uses `snake_case`: `ClientState.of(client).snap_to(zone, area, edge_tiling)`, `.unsnap()`, `.snap` and
  `.forget_snap()`, and `Snap.edge_zone(x, y, area, margin)` and `Snap.tile_rect(zone, area)`.
- Rust passes the typed objects and every call returns a `Result`: `ClientState::of(&objects, &client)?.snap_to(zone, &area,
Some(true))?`, `.unsnap()?`, `.snap()?` and `.forget_snap()?`, and `Snap::edge_zone(&objects, x, y, &area, margin)?` and
  `Snap::tile_rect(&objects, zone, &area)?`. A zone is a `SnapZone`.

`area` of `snapTo` is the work area the window tiles in, such as `gnoblin.workspaces.area`. `edgeTiling` says whether edge
tiling is on, and it defaults to on.

A maximised window floats first, so unsnapping returns it to the rectangle it had before it was maximised. A window that is
already snapped keeps the rectangle it first floated at. The snap record is kept beside the window, never on it, so a
window the shell does not own is left unchanged.
