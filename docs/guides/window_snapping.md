# Window snapping

[Configuration reference](/config/configure)

Dropping a dragged window on a screen edge tiles it. Turn that on in `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    edgeTiling: true,
});
```

The stock config has it on. With it on, drag a window with `Super` and the left button until the pointer is within 4
pixels of an edge, then release:

| Drop on        | Result                         |
| -------------- | ------------------------------ |
| The left edge  | The left half of the work area |
| The right edge | The right half                 |
| The top edge   | The whole work area            |

Drag a snapped window away to restore its original size. Windows fill the usable part of the monitor, leaving room for
panels that reserve space. Minimum sizes still apply; fullscreen and fixed-size windows are excluded. See
[drag boundaries](/guides/session_settings#window-drag-boundary) for the overlap policy.

With Bingux installed you can also press Super+Z to choose a layout. Bingux displays the picker, saves layouts in
`~/.config/bingux/snapping.ini`, and imports Tiling Shell layouts once when available. Other shells can provide their
own using the [integration API](#shell-integration).

## Shell integration

Use the [bridge socket](/bridge/) to supply a picker. Rectangles
use logical desktop pixels. Read the work area from Gnoblin; it excludes space
reserved by panels. `hit` is where pointer release selects a region, while
`target` is the final window rectangle.

| Step                  | Message                 | What Gnoblin returns or does                                                                               |
| --------------------- | ----------------------- | ---------------------------------------------------------------------------------------------------------- |
| Watch drags           | `{"op":"window-drag"}`  | `window-drag` events with `active`, `serial`, `window`, pointer `x`/`y`, `modifiers`, `monitor` and `area` |
| Offer regions         | `snap-offer`            | Tests the pointer against `hit` rectangles on release, then applies the matching `target`                  |
| Ask about focus       | `{"op":"snap-context"}` | `snap-context` with focused `window`, `monitor` and `area`                                                 |
| Apply keyboard choice | `snap-window`           | Moves and resizes the chosen window                                                                        |

For example, after receiving an active drag with `serial: 7`, a shell can
offer the left half of a 1920 × 1048 work area starting at `(0, 32)`:

```json
{
    "op": "snap-offer",
    "serial": 7,
    "regions": [
        {
            "hit": { "x": 0, "y": 32, "width": 80, "height": 1048 },
            "target": { "x": 0, "y": 32, "width": 960, "height": 1048 },
            "layout": "left-half"
        }
    ]
}
```

Build both rectangles from the current event's `area` and `monitor`; the
numbers above show only the message shape.

An offer can contain up to 128 regions. Each `hit` and `target` rectangle
contains finite logical-pixel coordinates:

| Field             | Accepted values   | Meaning                                                    |
| ----------------- | ----------------- | ---------------------------------------------------------- |
| `x`, `y`          | −100,000–100,000  | Finite top-left position in desktop logical pixels.        |
| `width`, `height` | 1–32768           | Rectangle size in logical pixels.                          |
| `layout`          | Optional value    | Echoed in `snap-completed`; Gnoblin does not interpret it. |
| `control`         | Boolean, optional | Require Ctrl to be held at pointer release.                |
| `maximize`        | Boolean, optional | Maximize instead of applying the `target` rectangle.       |

Set `maximize: true` to maximize the window instead of applying `target` as its
final size. On success, the owner receives
`{"event":"snap-completed","layout":"left-half"}`.

For a keyboard picker, request `snap-context`, let the user choose a rectangle
inside its `area`, then send:

```json
{ "op": "snap-window", "window": "42", "monitor": 0, "target": { "x": 0, "y": 32, "width": 960, "height": 1048 } }
```

Use the returned window and monitor IDs. Only one client can own a drag, and
Gnoblin rejects stale serials or targets outside the work area.

Escape, disconnect or drag completion clears the offers. The picker does not
grab input while the drag is in progress.
