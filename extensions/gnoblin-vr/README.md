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

The compositor keeps a spatial scene store behind these calls. Anchors, free and viewport-docked surface placements, stacking ranks and pose updates are validated and applied there, and `snapshot` and `surface` return the stored records with a scene revision. A headless test covers the store. It does not yet import window textures, project quad layers, hit test controller rays, or route pointer and focus events; those consume the store in later phases.

## Headset as an outputs-style record

`gnoblin-vr.lua` can describe the headset in the shape of a `gnoblin.outputs` record. `output(callback)`
runs `gnoblin-vr --status` and passes `callback` a record with `kind = "xr"`, `enabled`, `state`,
`diagnostic` and a `views` list of per-eye `width` and `height`. The record has no logical desktop
rectangle. It matches the `xr:0` record that `gnoblin.outputs.list()` returns; use the helper when you
need a fresh read of the status instead of the published snapshot.
`parse_status` and `output_record` are the pure parts, if you already have the status text.

## Window arrangement policy

`gnoblin-vr-shell.lua` is a Lua module that decides where windows go. It moves a viewport pill, docks
windows to it, places windows in free space turned towards you, and turns controller rays into grab, move
and resize. The compositor only stores the result, so a configuration can replace any of it. Copy the
module next to `gnoblin-vr.lua` and load it with `require("gnoblin-vr-shell")`.

```lua
local shell = require("gnoblin-vr-shell").new()
shell:dock("window:1", {width = 1.2, height = 0.7}, {x = -0.7, y = 0, z = 0})
shell:float("window:2", {x = 0.8, y = 1.4, z = -3}, {width = 1.6, height = 0.9})
local grab = shell:grab({origin = {x = 0, y = 1.4, z = 0}, direction = {x = 0.8, y = 0, z = -3}})
shell:drag(grab, {origin = {x = 0, y = 1.4, z = 0}, direction = {x = -1, y = 0, z = -3}})
```

`tests/vr-shell-live.test.lua` runs the module against a live session: `gnoblinctl lua
tests/vr-shell-live.test.lua` prints one PASS or FAIL line per check.

The compositor reads the status file at `$XDG_RUNTIME_DIR/gnoblin-vr/status.ini` and adds the headset to its output list as an `xr:0` record with `kind = "xr"`, `state`, `diagnostic` and per-eye `views`. Mutter has no registration interface for an external output driver, so the record is read-only and the monitor manager still owns every display output.

Build and test the extension independently with CMake and CTest from this
directory. The build requires GLib/GIO and the OpenXR loader development files.
