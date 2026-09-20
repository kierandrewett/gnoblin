# Blur during client animations

A translucent panel still needs full blur while it is visible. During a fade-out,
both the panel and its blur should disappear. Pixel transparency alone cannot
tell Gnoblin which of those two cases is happening.

This page explains how shell developers mark animated regions. For normal
desktop settings, see [animations](animations.md).

## Whole-window fades

Let Gnoblin animate the whole surface using a [layer animation](animations.md#per-surface-animations).
It fades the window and the blur behind it together.
Do not also fade the client buffer for the same transition.

## Items sharing one buffer

Use `gnoblin_blur_fade_manager_v1` for independent item fades.
Commit each item's logical rectangle and animation opacity with the buffer.

The protocol does not enable blur or change foreground pixels.
It is optional and unavailable outside Gnoblin mode.

| Limit               | Value                                  |
| ------------------- | -------------------------------------- |
| Regions per surface | 32                                     |
| Coordinates         | Signed 24.8 fixed point, within ±65536 |
| Opacity             | 0–1                                    |
| Clear metadata      | Empty array                            |

Do not submit both an ancestor and descendant for the same animation.
Group overlapping foreground elements and their shadows before applying opacity.

Bingux submits metadata during Qt scene synchronisation, including client
decoration offsets. Settled windows clear it and use the ordinary blur cache.

## Foreground details

Icons and separators can be more opaque than the panel. Their alpha must not
create gaps in the backdrop.

The fallback mask only estimates adjacent material at a silhouette edge with
matching colour. Explicit [standard regions](background-effects.md) give the
client direct control of that shape.

## Tests

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-blur-fade-protocol.sh" \
bash scripts/run-gnome-shell.sh
```

Repeat with `GNOBLIN_TEST_MODE=user` for stock-session isolation.

For client fades, run `tests/test-blur-fade.py` with
`GNOBLIN_TEST_CLIENT_FADE=1` and a Qt-matched `QS_TEST_BIN`.

Bingux's `tests/panel-blur-fade.py` and `tests/shared-buffer-fades.py`
cover intermediate opacity, closing and reversal. They require its effects
plugin in the matching Quickshell import path.

`tests/test-blur-detail-coverage.py` checks foreground details.
Optional `STEAM_ICON` and `LOCALSEND_ICON` paths add real icon samples;
synthetic cases always run.
