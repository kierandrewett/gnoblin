# Window snapping

Bingux owns the layout picker, region preview, saved layouts, and selection UI.
Gnoblin owns window drag events and applies the selected rectangle to a native
window. This supports normal resizable Wayland and X11 windows. Application
minimum sizes still apply. Fullscreen and fixed-size windows are excluded.

## Interaction

- Drag a title bar towards the top centre of its monitor to reveal the layout picker.
- Move over a miniature region and release to snap. Move away to cancel the target.
- Hold Ctrl during a drag to select regions directly in the active layout.
- Press Super+Z to choose a region for the focused window. Use arrows or Tab,
  then Enter. Escape closes the picker.
- Drag a snapped window out to restore its original dimensions.

Bingux imports Tiling Shell layouts and gaps once, when available. Otherwise it
provides halves, thirds, quarters, a wide column, and a central focus layout.
The settings live in `~/.config/bingux/snapping.ini`. Regions use logical monitor
coordinates and the current workspace work area, including exclusive zones.

## Compositor contract

The existing user-private compositor socket accepts:

- `window-drag`: subscribe to `window-drag` state changes. An active record has a
  drag serial, stable window ID, pointer coordinates, modifier mask, monitor,
  and work area. Inactive records end the drag UI.
- `snap-offer`: supply hit rectangles and target rectangles for the current serial.
  Each region marks whether Ctrl must be held. Only one client can own a drag.
- `snap-context`: return the focused window, monitor, and work area for keyboard selection.
- `snap-window`: apply a target rectangle to a stable window ID on a named monitor.

The compositor checks the pointer again at release. It rejects stale serials,
invalid rectangles, locked sessions, dead windows, and targets outside the work
area. Escape, disconnect, script reload, and drag completion clear pending offers.
`snap-completed` reports the committed layout so hovering or cancelling does not change the active layout.
Pointer sampling runs only during a move grab and emits only changed state.
The UI has no pointer or keyboard grab during a mouse drag.

## Verification

Run the Bingux geometry tests with `node tests/snap-layouts.test.cjs` from Bingux.
The native integration test runs in a private Gnoblin session. It drives a GTK
window through actual virtual mouse and keyboard events and checks native frame
coordinates, focus, restore, cancellation, keyboard selection, and Ctrl regions.
Run from Gnoblin (set `GNOBLIN_QS` to the matching Quickshell wrapper if needed):

```sh
GNOBLIN_PREFIX="$PWD/install" GNOBLIN_CONFIG='' \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-window-snapping.py" \
bash scripts/run-gnome-shell.sh
```

Add `GNOBLIN_TEST_XWAYLAND=1 SNAP_TEST_BACKEND=x11` for the X11 run.
The test uses the default 1280x800 virtual monitor and the sibling Bingux checkout.
Set `BINGUX_SOURCE` when the checkout is elsewhere.
