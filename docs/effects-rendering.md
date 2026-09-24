# Effect rendering

Developer notes for the implementation behind [window effects](/config/window_effects).

## Blur cache

Blur retains GPU textures and framebuffers until dimensions or radius change.
Damage outside its sampling rectangle does not invalidate the backdrop.

Before painting, scene damage expands through overlapping blur regions.
A surface crossing monitors refreshes for each view; there is no refresh timer.

Clients can supply [standard blur regions](background-effects.md).
Bingux's bridge hints are scoped to layer surfaces owned by the socket peer;
disconnecting removes the hints. Rules still decide blur strength.

## Shape and shadows

Reported client geometry excludes shadow margins. If the client reports its
whole buffer instead, three alpha scan lines per axis look for a stable edge.
Uncertain results keep the reported frame.

Detected geometry is shared by clipping, borders and shadows. It is cached
until geometry, state or scale changes. Explicit padding bypasses detection.

Forced rounding removes the original outside shadow, even with
`shadow = false`. Automatic mode preserves it unless a replacement is configured.

CSD corner reconstruction samples neighbouring colour/alpha patches in the
shader. Two patches must agree. It does not cache colour or read pixels back
to the CPU every frame.

## Fades

Actor opacity fades the composed client and blur together.
Independent client items use [blur-fade metadata](blur-fades.md);
buffer alpha alone cannot distinguish animation from glass transparency.

## Tests

Run against a rebuilt private prefix:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-blur-regions.py" \
bash scripts/run-gnome-shell.sh
```

| Test                                   | Covers                                              |
| -------------------------------------- | --------------------------------------------------- |
| `tests/test-window-effects.py`         | Blur pixels and removal                             |
| `tests/test-window-shaders.py`         | Shaders, transparency, uniforms and reload failures |
| `tests/test-blur-regions.py`           | Region bounds, movement and damage                  |
| `tests/test-blur-performance.py`       | Cache work and CPU samples                          |
| `tests/test-window-shadow-clipping.py` | Native and replacement shadows                      |
| `tests/test-blur-fade.py`              | Intermediate fades and reversals                    |

Set `GNOBLIN_BLUR_REQUIRE_CACHE=1` for the performance cache assertion.
Private pixel checks do not measure desktop latency, GPU memory or power.
