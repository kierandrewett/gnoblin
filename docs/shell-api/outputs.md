# Outputs

Outputs are rendering targets behind the logical monitor layout. Use them to
read display modes and the GPU path for this session.

Read-only calls return detached snapshots. `gnoblinctl output list` prints the
same snapshot from the command line, with `--json` for machine-readable output.

## Read outputs

| Method                       | Arguments           | Result                       |
| ---------------------------- | ------------------- | ---------------------------- |
| `gnoblin.outputs.list()`     | None                | All current `Output` records |
| `gnoblin.outputs.primary()`  | None                | Primary record or `nil`      |
| `gnoblin.outputs.by_id(id)`  | String connector ID | Matching record or `nil`     |
| `gnoblin.outputs.renderer()` | None                | `Renderer` record or `nil`   |

```lua
local renderer = gnoblin.outputs.renderer()
if renderer and not renderer.accelerated then
    -- Avoid enabling expensive visual effects for this session.
end

for _, output in ipairs(gnoblin.outputs.list()) do
    print(output.id, output.mode.width, output.mode.height)
end
```

During login configuration the output snapshot may not exist yet: `list()`
returns an empty list, and `primary()`, `by_id()`, and `renderer()` return
`nil` until the first snapshot is published. Snapshots refresh on output
changes, so event handlers read current state instead of caching records.

## Output records

| Field        | Type            | Meaning                                                        |
| ------------ | --------------- | -------------------------------------------------------------- |
| `id`         | String          | Connector name, such as `"DP-1"`. Stable per output.           |
| `kind`       | String enum     | `"display"` or `"xr"`. `"virtual"` is reserved.                |
| `name`       | String or `nil` | Display name, when the hardware provides one.                  |
| `make`       | String or `nil` | Manufacturer string, when available.                           |
| `model`      | String or `nil` | Model string, when available.                                  |
| `serial`     | String or `nil` | Serial string, when available.                                 |
| `enabled`    | Boolean         | Whether the output is active in the current layout.            |
| `primary`    | Boolean         | Whether this is the primary output.                            |
| `monitor_id` | String or `nil` | Matching logical monitor ID from `gnoblin.monitors`.           |
| `mode`       | Record          | `width` and `height` in physical pixels; `refresh_rate` in Hz. |
| `scale`      | Number          | Compositing scale of the logical monitor.                      |
| `transform`  | String          | Current transformation, such as `"normal"` or `"90"`.          |
| `x`, `y`     | Number or `nil` | Logical position; present for display outputs.                 |
| `renderer`   | Record          | `Renderer` record for the GPU path that renders this output.   |
| `revision`   | Integer         | Snapshot revision of this record.                              |

An output is not guaranteed to occupy a rectangle in the logical layout.
The reserved `virtual` kind omits the logical position and rectangle shape.

## Headset output

With the optional VR extension installed, each headset that `gnoblin-vr --watch` finds is an output
with `kind = "xr"`. Both `gnoblin.outputs.list()` and `gnoblinctl output list` include it.

The ids are `xr:0`, `xr:1` and so on, in a stable order. A headset has no `mode`, position or
renderer. It carries these fields instead:

| Field        | Type            | Meaning                                                                                                     |
| ------------ | --------------- | ----------------------------------------------------------------------------------------------------------- |
| `state`      | String          | `"waiting"`, `"detected"`, `"ready"`, `"connected"` or `"running"`. See [VR headsets](/guides/vr_headsets). |
| `connection` | String          | `"tethered"` for a display cable, `"standalone"` for a headset that receives a stream.                      |
| `transport`  | String          | `"drm-lease"`, `"usb"` or `"wifi"`.                                                                         |
| `backend`    | String          | The OpenXR runtime that drives it: `"monado"` or `"wivrn"`.                                                 |
| `vendor`     | String or `nil` | The headset maker, when known.                                                                              |
| `device_id`  | String          | A stable id such as `drm:DP-2` or `usb:2833:0186:SERIAL`, used by `gnoblin-vr --connect`.                   |
| `diagnostic` | String or `nil` | What to do next when the headset is not ready.                                                              |
| `views`      | List of records | One `width` and `height` in pixels for each eye while the session runs; empty otherwise.                    |

`enabled` is true only while `state` is `"running"`. Without `gnoblin-vr --watch`, the
extension reports a single `xr:0` from the session status alone, with `state` set to
`"running"`, `"stopped"` or `"absent"`.

When a headset appears, changes state or goes away, the compositor sends the
`gnoblin.vr.headset-changed` event with the current list in `headsets`:

```lua
gnoblin.on("gnoblin.vr.headset-changed", function(event)
    for _, headset in ipairs(event.headsets) do
        print(headset.id, headset.connection, headset.state)
    end
end)
```

## Renderer records

| Field         | Type            | Meaning                                   |
| ------------- | --------------- | ----------------------------------------- |
| `name`        | String          | GL renderer string, such as `"llvmpipe"`. |
| `vendor`      | String or `nil` | GL vendor string, when available.         |
| `api`         | String          | GL API in use, such as `"gles2"`.         |
| `accelerated` | Boolean         | `false` for software rasterizers.         |

Gnoblin reports `accelerated = false` for the software rasterizers llvmpipe,
softpipe, swrast, and Software Rasterizer, and when Mutter reports software
rendering. All outputs share one compositor renderer record today; the outputs
design note covers multi-GPU limitations.

Display configuration such as modes and profiles stays with the session
display-configuration interface. `gnoblin.outputs` gains write access only
through that interface; see [workspaces and monitors](/shell-api/workspaces-monitors)
for the logical layout side.
