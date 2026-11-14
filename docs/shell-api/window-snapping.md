# Window snapping

Snapping tiles a window to an edge of the work area. Dropping a dragged window on an edge snaps it, and the stock move
binding does that when `edgeTiling` is on. A script can snap a window by itself with the helpers `gnoblin` exports.

```ts
import { defineConfig, snapClient, unsnapClient } from "gnoblin";

defineConfig({
    edgeTiling: true,
    keys: {
        "Super+Shift+Left": ({ gnoblin }) => {
            const client = gnoblin.clients.focused;
            const area = gnoblin.workspaces.area;
            if (client && area) {
                snapClient(client, "left", { edgeTiling: true, area });
            }
        },
        "Super+Shift+Down": ({ gnoblin }) => {
            const client = gnoblin.clients.focused;
            if (client) {
                unsnapClient(client);
            }
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+Shift+Left` tiles the focused window to the left half of the work area, and
`Super+Shift+Down` puts it back where it floated.

## Zones

A snap zone is the part of the work area a window tiles to:

| Zone    | The window fills                | Pointer is within 4 pixels of |
| ------- | ------------------------------- | ----------------------------- |
| `left`  | The left half of the work area. | The left edge.                |
| `right` | The right half.                 | The right edge.               |
| `top`   | The whole work area.            | The top edge.                 |

A corner counts as the left or right edge before the top. The halves are whole pixels.

Edge tiling is off unless `edgeTiling` is `true` in `defineConfig`, or the seat's setting says so.

The stock move drag, `ClientMoveBinding`, snaps a window dropped within `snap` pixels of an edge. The default is 4, and
`snap: false` never snaps.

## Functions

| Function                            | Does                                                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `edgeZone(x, y, area, margin?)`     | The zone a point is within `margin` pixels of, or `null`. `margin` defaults to 4.                      |
| `tileRect(zone, area)`              | The rectangle a window tiles to in a zone, or `null` for a name that is not a zone.                    |
| `snapClient(client, zone, options)` | Tiles a window. Returns `false`, and moves nothing, when edge tiling is off or the zone does not tile. |
| `unsnapClient(client)`              | Puts a snapped window back where it floated. Returns `false` when it is not snapped.                   |
| `snappedZone(client)`               | The zone a window is snapped to, or `null`.                                                            |

`options` of `snapClient` is `{ edgeTiling, area }`: whether edge tiling is on, and the work area the window tiles in, such as
`gnoblin.workspaces.area` or `output.area`.

A maximised window floats first, so unsnapping returns it to the rectangle it had before it was maximised. A window that is
already snapped keeps the rectangle it first floated at. The snap record is kept beside the window, never on it, so a
window the shell does not own is left unchanged.
