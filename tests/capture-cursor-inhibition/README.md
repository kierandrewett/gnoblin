# Embedded capture cursor regression

Run from the repository root, without a desktop or session bus:

```sh
python3 tests/capture-cursor-inhibition/run.py --baseline
python3 tests/capture-cursor-inhibition/run.py
CC=clang python3 tests/capture-cursor-inhibition/run.py --sanitize
```

The baseline deliberately exits 1: embedded area and monitor streams leave
the renderer stale on both enable and disable. The patched run must pass
all ten cases. The regression was run and failed before the patch was created.

The runner reads the Mutter revision from `src/gnome-versions.json` using
`git show`, applies the new patch in a temporary directory, and compiles
complete production functions verbatim. It does not edit the shared submodule.
It requires Python 3, a C compiler, `pkg-config`, Git and GLib development files.
It installs nothing.

To test a tree already prepared by the normal build, use:

```sh
python3 tests/capture-cursor-inhibition/run.py --source-tree /path/to/prepared/mutter
```

Use `--test /area/embedded-disable` to isolate hardware restoration. Unknown
test paths and an empty test listing are errors.

## What the test proves

The fixture executes the production backend inhibitor add/remove functions,
the aggregate inhibition predicate, the existing `update_cursors()` helper
and its sprite callback, and the area/monitor stream enable/disable functions.
GLib list ownership and assertions are real. Functions retain source paths
and line numbers in the compiled test.

The stage, renderer I/O, stream geometry and signal connections are doubles.
The renderer exposes hardware/overlay state at each synchronous refresh;
it does not paint pixels. This proves the missing refresh and its ordering,
not KMS programming, PipeWire pixels or callback scheduling in a live session.

The cases cover:

- Stationary embedded area and monitor streams refresh before enable returns.
- Disable restores hardware eligibility without another pointer event.
- Every active sprite is visited, including a second renderer and a sprite
  without a renderer.
- Hidden and metadata stream lifecycle does not change cursor rendering.
- Overlapping area/monitor streams retain inhibition until the last one ends.
- Global inhibition remains effective after a local stream ends.
- Refresh does not make an invisible cursor visible or inhibit a cursor
  outside the stream.

## Source and lifecycle evidence

The new patch is
[`66-capture-cursor/0001-refresh-cursors-on-stream-inhibition.patch`](../../patches/mutter/66-capture-cursor/0001-refresh-cursors-on-stream-inhibition.patch).
Its production change is two calls in `meta-backend.c`, after the inhibitor
list is changed. It uses Mutter's existing refresh mechanism and adds no
portal delay, protocol or setting.

Primary sources at the pinned Mutter 51.0 revision:

- [`meta-backend.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-backend.c):
  `update_cursors()` iterates stage sprites. Its callback invalidates each
  sprite and force-updates its renderer when one exists. Inhibitor add/remove
  previously only changed the list. The aggregate predicate retains other
  stream inhibitors and global inhibition.
- [`meta-stream-source-area.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-stream-source-area.c)
  and [`meta-stream-source-monitor.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-stream-source-monitor.c):
  only embedded mode adds an inhibitor. Both use the shared backend API.
  Disable removes paint watches before releasing that inhibitor. Embedded
  mode does not connect the metadata cursor-change callbacks, so refreshing
  does not re-enter that stream through such a callback. Geometry predicates
  do not depend on the stream's bookkeeping `hw_cursor_inhibited` flag.
- [`meta-cursor-renderer.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-cursor-renderer.c):
  software overlay texture and geometry are populated when a view needs
  software rendering. A hardware cursor can leave both absent until refresh.
- [`meta-cursor-renderer-native.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/native/meta-cursor-renderer-native.c):
  native renderer updates re-evaluate the backend inhibition predicate.
- [`meta-stream-source.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-stream-source.c):
  the CPU capture path already sets `FORCE_CURSORS` for embedded mode.
  That flag does not populate an empty overlay.

The separate global inhibit/uninhibit API is unchanged. This patch repairs
the shared per-stream transition used by area and monitor capture.

## Native VM confirmation

The installed candidate was checked in a fresh Gnoblin and Bingux VM session
on 2026-10-13. The compositor executable SHA256 was
`bc5c70cd5aaa8ca34b0fa90258865a4fb58c33d028419d67106a8d3bd5e79317`;
the mapped Mutter library was
`4c13c427271cb7ceb419bacc56fa4d2457848505277abc1eedeb07a4f9cdcddc`.

The standard screenshot portal returned two 1280-by-800 PNGs with a stationary
pointer at 950,520. Both cursor crops were identical. The pointer-off control
had no pointer; its only difference in that crop was the cursor footprint.
Consent was absent from the captured desktop. These captures ran without a
debugger or injected cursor refresh.

A separate passive debugger trace then recorded the real area-stream lifecycle:

- Adding the inhibitor immediately called `update_cursors()`.
- Forced paint had flags 6, a populated texture and a 24-by-24 cursor rectangle
  at 946,518. Before this patch, the same baseline trace had a null texture and
  a zero rectangle; injecting one refresh made that baseline capture work.
- Removing the inhibitor left the list empty and called the native renderer's
  update through `update_cursor_foreach_cb()` without another pointer event.

The before/after PipeWire snapshots contained the same 48 object IDs, and the
ScreenCast object tree was unchanged after completion. The debugger detached
normally. Evidence is under the VM run's `astra-cursor-*` artifacts; the original
baseline traces are in `astra-portal-cursor-diagnosis/`.

This proves pixels and refresh ordering for native area capture in this VM.
It does not prove a physical KMS cursor plane was restored. Live monitor-stream
and overlapping-stream checks remain separate from the ten fixture cases.

To repeat, activate the patched build in a disposable session and record its
executable and mapped-library identities. Request an interactive screenshot
through `org.freedesktop.portal.Screenshot`, select Include pointer, leave the
pointer stationary outside consent, and accept using the keyboard. Compare the
saved PNG with a pointer-off control. Observe inhibitor removal and native
renderer refresh separately; do not inject a refresh into the pixel check.
