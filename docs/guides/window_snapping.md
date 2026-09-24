# window_snapping

[Configuration reference](/config/configure)

With Bingux installed, drag a window toward an edge or press Super+Z to choose
a layout. Bingux displays the picker; Gnoblin moves and resizes the window.
The picker controls below belong to Bingux. Other shells can provide their own
using the [integration API](#shell-integration).

This supports resizable Wayland and X11 windows. Minimum sizes still apply;
fullscreen and fixed-size windows are excluded.

## Use the picker

| Action                       | Result                                 |
| ---------------------------- | -------------------------------------- |
| Drag toward the top centre   | Show layouts                           |
| Release at the top edge      | Fill the work area                     |
| Drag to a side or corner     | Half or quarter screen                 |
| Release over a layout region | Snap to that region                    |
| Hold Ctrl while dragging     | Select regions in the active layout    |
| Super+Z                      | Choose a region for the focused window |
| Arrows/Tab, then Enter       | Select with the keyboard               |
| Escape                       | Cancel                                 |
| Drag a snapped window away   | Restore its original size              |

Bingux saves layouts in `~/.config/bingux/snapping.ini`.
It imports Tiling Shell layouts once when available; otherwise it supplies defaults.

Windows fill the usable part of the monitor, leaving room for panels that
reserve space.
See [drag boundaries](/guides/session_settings#window-drag-boundary) for overlap policy.

## Shell integration

Use the [bridge socket](/compositor-bridge) to supply a picker. Rectangles
use logical desktop pixels. Read the work area from Gnoblin; it excludes space
reserved by panels.

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
numbers above only show the message shape. At most 128 regions may be offered.
Each rectangle needs finite `x`, `y`, `width` and `height`; sizes must be 1–32768.
Optional `control: true` requires Ctrl at release. Optional `maximize: true`
maximizes the window instead of using `target` as its final size. On success the
owner receives `{"event":"snap-completed","layout":"left-half"}`.

For a keyboard picker, request `snap-context`, let the user choose a rectangle
inside its `area`, then send:

```json
{ "op": "snap-window", "window": "42", "monitor": 0, "target": { "x": 0, "y": 32, "width": 960, "height": 1048 } }
```

Use the returned window ID and monitor ID. Only one client owns a drag. Gnoblin
rejects stale serials and targets outside the work area. Escape, disconnect or
drag completion clears offers; the picker does not grab input during a drag.
