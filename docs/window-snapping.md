# Window snapping

[Configuration reference](configuration-reference.md)

Bingux provides the layout picker. Gnoblin moves and resizes the window.

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

Targets use the monitor's work area, including panel reservations.
See [drag boundaries](session-settings.md#window-drag-boundary) for overlap policy.

## Shell integration

The [compositor bridge](compositor-bridge.md) accepts:

| Request          | Purpose                                                       |
| ---------------- | ------------------------------------------------------------- |
| `window-drag`    | Subscribe to serial, window, pointer, modifiers and work area |
| `snap-offer`     | Offer hit/target rectangles for the current drag              |
| `snap-context`   | Get the focused window and work area                          |
| `snap-window`    | Apply a rectangle to a window                                 |
| `snap-completed` | Receive the committed layout                                  |

Only one client can own a drag. The compositor rechecks the pointer at release
and rejects stale serials or out-of-work-area targets.

Escape, disconnect, script reload and drag completion clear offers.
The picker does not take an input grab during a mouse drag.

## Test

From Gnoblin, with the sibling Bingux checkout:

```sh
GNOBLIN_PREFIX="$PWD/install" GNOBLIN_CONFIG='' \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-window-snapping.py" \
bash scripts/run-gnome-shell.sh
```

Set `BINGUX_SOURCE` for another checkout location and `GNOBLIN_QS` for a
specific Quickshell wrapper. Add `GNOBLIN_TEST_XWAYLAND=1 SNAP_TEST_BACKEND=x11`
for X11. Geometry tests also run with `node tests/snap-layouts.test.cjs` in Bingux.
