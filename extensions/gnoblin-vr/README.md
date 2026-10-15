# Gnoblin VR extension

`gnoblin-vr` is an optional OpenXR presentation client. It runs outside the
compositor and does not render scene content in this phase. Install the
extension package alongside Gnoblin, then opt in for the current user with:

```sh
systemctl --user enable --now gnoblin-vr.service
```

The service is not enabled by package installation and is not pulled into the
stock session target. Disable it with `systemctl --user disable --now
gnoblin-vr.service`.

The client uses the Khronos OpenXR loader. If
`$XDG_RUNTIME_DIR/monado_comp_ipc` is a Monado service socket and
`XR_RUNTIME_JSON` is unset, it selects an installed Monado runtime manifest.
Without an available runtime or headset, it exits successfully after writing
an `absent` status and a diagnostic under
`$XDG_RUNTIME_DIR/gnoblin-vr/status.ini`.

The extension Lua module is shipped at
`/usr/share/gnoblin-vr/lua/gnoblin-vr.lua`. Copy it into the configuration's
`lua/` module directory and assign `require("gnoblin-vr")` to
`gnoblin.outputs.xr`; the extension supplies `start`, `stop`, and `status`.
The optional extension also installs `gnoblin.spatial` through Gnoblin's Lua
extension installer hook. Its `anchor`, `place`, `attach`, `move`, `raise`,
and `update_pose` methods validate extension-owned scene arguments and submit
asynchronous `vr.spatial.*` actions from the supervised Lua runtime. The
`surface` and `snapshot` methods are likewise typed extension actions.

The compositor validates and stores anchors, placements, stacking, poses, pointer input and environment mode. `snapshot` and `surface` read scene records; `hit` finds the nearest surface; `point` moves the real pointer and can press or release a button. Surface IDs for windows are their Gnoblin window IDs. The compositor owns storage and input delivery; Lua policy owns placement and gesture decisions.

## Headset as an outputs-style record

`gnoblin-vr.lua` can describe the headset in the shape of a `gnoblin.outputs` record. `output(callback)`
runs `gnoblin-vr --status` and passes `callback` a record with `kind = "xr"`, `enabled`, `state`,
`diagnostic` and a `views` list of per-eye `width` and `height`. The record has no logical desktop
rectangle. It matches the `xr:0` record that `gnoblin.outputs.list()` returns; use the helper when you
need a fresh read of the status instead of the published snapshot.
`parse_status` and `output_record` are the pure parts, if you already have the status text.

## Cursor sprite

The compositor publishes the cursor the session is using, so the client can draw it at the end of a
controller ray. `$XDG_RUNTIME_DIR/gnoblin-vr/cursor.rgba` holds the sprite as straight RGBA8 pixels, and
`cursor.ini` gives its `width`, `height`, hotspot (`hot_x`, `hot_y`), `scale` and a `serial` that
changes with the sprite. Re-import the texture only when `serial` changes. It follows the cursor theme
and the shape a client sets, for example the text beam over a terminal.

## Window arrangement policy

`gnoblin-vr-desktop.lua` is the default desktop policy. It mirrors normal windows on a two-metre arc at eye height, tracks window lifecycle and focus, and maps controller input to pointer, move, resize, workspace movement, scaling and passthrough. Install it beside the other Lua modules and enable it in the user configuration:

```lua
require("gnoblin-vr-session").auto()
require("gnoblin-vr-desktop").auto()
```

`auto(opts)` accepts `pixels_per_metre` (default 1000), `radius` (2 metres), `eye_height` (1.45 metres), `arc_step` (18 degrees), `arc_center`, `minimum_size`, `resize_debounce`, `environment` (`opaque` or `passthrough`), workspace scale limits, and replacement functions for the main policy methods. See the [VR headsets guide](../../docs/guides/vr_headsets.md) for the controls table.

`gnoblin-vr-shell.lua` remains available for a viewport pill and optional docked surfaces. It can be loaded independently with `require("gnoblin-vr-shell")`.

## Headsets

Two kinds of headset reach the computer, and each needs a different OpenXR runtime.

- **Tethered** (Index, Vive, PSVR2, Pimax, Windows MR): a display cable and USB. The cable is a DRM
  connector flagged `non-desktop`. Mutter leaves it out of the desktop and offers it through
  `wp_drm_lease_device_v1`; Monado takes the lease in direct mode and scans out to it. Backend:
  `monado`.
- **Standalone** (Quest, Pico, Vive Focus): the headset runs the WiVRn app and receives the scene
  as a video stream over a USB cable (`adb reverse`) or Wi-Fi. Backend: `wivrn`.

WiVRn is GPL-3.0-or-later. Gnoblin runs it as a separate process, never linked, and pins its source
as the submodule `subprojects/wivrn` for distributions that do not package it
(`make vr-streamer INSTALL=1`). Monado is BSL-1.0.

`gnoblin-vr --watch` (the `gnoblin-vr-watch.service` unit) follows USB, DRM and the WiVRn D-Bus
service with udev and signals, and keeps `$XDG_RUNTIME_DIR/gnoblin-vr/devices.ini` current. The
compositor turns it into `xr:N` outputs, each with `state`, `connection`, `transport`,
`backend` and `device_id`, and sends `gnoblin.vr.headset-changed` when the list changes.

`gnoblin-vr-session.lua` is the default policy. `require("gnoblin-vr-session").auto()` starts the
runtime unit for the chosen headset (`gnoblin-vr-runtime@monado` or `@wivrn`), runs
`gnoblin-vr --connect` for a standalone headset on USB, and starts the client
(`gnoblin-vr@monado` or `@wivrn`) once the headset is ready. The compositor knows none of this.
See the [VR headsets guide](../../docs/guides/vr_headsets.md) for setup.

| Command                        | What it does                                                      |
| ------------------------------ | ----------------------------------------------------------------- |
| `gnoblin-vr --devices`         | Print the headsets found right now.                               |
| `gnoblin-vr --watch`           | Keep `devices.ini` current as headsets come and go.               |
| `gnoblin-vr --connect [ID]`    | Start the USB link to a standalone headset through adb.           |
| `gnoblin-vr --runtime BACKEND` | Run the OpenXR runtime for `monado` or `wivrn` in the foreground. |
| `gnoblin-vr --backend BACKEND` | Run the presentation client against that runtime.                 |

The USB table that classifies headsets is a hint for what to offer; the OpenXR runtime has the
final say on a session. Detection reads `GNOBLIN_VR_FIXTURE`, a directory standing in for sysfs,
DRM and the streamer, which is how `tests/vr-devices.test.py` runs without hardware.

The compositor reads `$XDG_RUNTIME_DIR/gnoblin-vr/status.ini` for the session state and the per-eye `views` of the headset that hosts the session. Mutter has no registration interface for an external output driver, so the records are read-only and the monitor manager still owns every display output.

Build and test the extension independently with CMake and CTest from this
directory. The build requires GLib/GIO and the OpenXR loader development files.
