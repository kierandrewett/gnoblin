# Panel blur fades

Blur must fade with the completed foreground, not with its material alpha.
Glass alpha sets the tint. Animation opacity sets how much of the completed
glass, text and blurred background remains visible.

Whole-window fades use the compositor actor opacity. Clients must not also
fade their buffers when the compositor owns that transition.

Independent items in a shared buffer use `gnoblin_blur_fade_manager_v1`.
The client commits logical surface rectangles and animation opacity with the
buffer. Mutter carries the metadata through pending-state merges and applies
it in the surface transaction. The masked blur shader then combines item
opacity, material alpha and actor opacity. The protocol does not enable blur
or change the foreground. It is optional and disabled outside Gnoblin mode.

The limit is 32 independent visible regions per surface. Values are signed
24.8 fixed point. The server rejects malformed arrays, invalid dimensions,
coordinates outside +/-65536 and opacity outside 0..1. An empty array clears
the metadata. Overlapping independent regions combine their coverage; do not
submit both an ancestor and its descendant for the same animation.

Qt clients must group overlapping foreground elements before applying opacity.
Include shadows in that group. Do not treat material transparency as animation
opacity. The Bingux plugin reads the same item state as the scene graph during
`beforeSynchronizing`, before Qt commits its buffer. It includes client-side
decoration margins when it converts scene coordinates to surface coordinates.
Settled windows clear the metadata and retain the existing blur cache path.

## Verification

Run each test in a separate private compositor. Set `GNOBLIN_PREFIX` explicitly;
the active desktop can still use the previous native libraries until login.

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-blur-fade-protocol.sh" \
bash scripts/run-gnome-shell.sh

GNOBLIN_PREFIX="$PWD/install" GNOBLIN_TEST_MODE=user \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-blur-fade-protocol.sh" \
bash scripts/run-gnome-shell.sh

GNOBLIN_PREFIX="$PWD/install" GNOBLIN_TEST_CLIENT_FADE=1 \
QS_TEST_BIN=/path/to/qt-matched-quickshell \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-blur-fade.py" \
bash scripts/run-gnome-shell.sh
```

Bingux adds `tests/panel-blur-fade.py` for actual popup, tooltip and switcher
close frames, and `tests/shared-buffer-fades.py` for independent component
fades. Supply these as `GNOBLIN_TEST_DBUS_CLIENT` with `MONITOR=1920x1080` and
a Qt-matched `QS_TEST_BIN`. Pixel comparisons include intermediate opacity,
zero opacity and reversal to fully visible. Build the Bingux effects plugin
and install it in that Quickshell runtime's QML import path first.

A native rebuild is not a live compositor reload. Install both native
libraries and start a new desktop session to activate this path on the desktop.
