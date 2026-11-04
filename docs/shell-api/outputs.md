# Outputs

An output is a connected display. Use outputs to read the layout, display
modes, work areas and privacy screens, to follow display changes, and to read
the GPU path for this session.

`gnoblinctl output list` prints the same records from the command line, with
`--json` for machine-readable output.

## Read outputs

| Method                       | Arguments           | Result                       |
| ---------------------------- | ------------------- | ---------------------------- |
| `gnoblin.outputs.list()`     | None                | All current `Output` objects |
| `gnoblin.outputs.primary()`  | None                | Primary output or `nil`      |
| `gnoblin.outputs.by_id(id)`  | String connector ID | Matching output or `nil`     |
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

During login configuration the output list may not exist yet: `list()`
returns an empty list, and `primary()`, `by_id()` and `renderer()` return
`nil` until Gnoblin publishes the first list.

## Live output objects

Like [window objects](/shell-api/windows#live-window-objects), every read of
one output returns the same object, updated with each change:

- `output.alive` turns `false` when the display is disconnected or removed.
- `output:on(signal, fn)` takes `"changed"` or `"closed"`. A `"changed"`
  callback receives the output and the list of changed fields. Listening ends
  after `"closed"`.
- `output:watch(field, fn)` calls `fn(output, value, old)` when a field of the
  `Output` record changes, such as `scale`, `mode` or `privacy_screen`.

Both return a subscription with an `unsubscribe()` method.

```lua
local output = gnoblin.outputs.primary()
if output then
    output:watch("scale", function(o, scale, old)
        print(o.id, "scale", old, "->", scale)
    end)
    output:on("closed", function(o)
        print(o.id, "was removed")
    end)
end
```

## Output records

| Field             | Type             | Meaning                                                              |
| ----------------- | ---------------- | -------------------------------------------------------------------- |
| `id`              | String           | Connector name, such as `"DP-1"`. Stable per output.                 |
| `alive`           | Boolean          | `false` once the output has been removed.                            |
| `kind`            | String enum      | `"display"`. `"virtual"` is reserved.                                |
| `name`            | String or `nil`  | Display name, when the hardware provides one.                        |
| `make`            | String or `nil`  | Manufacturer string, when available.                                 |
| `model`           | String or `nil`  | Model string, when available.                                        |
| `serial`          | String or `nil`  | Serial string, when available.                                       |
| `enabled`         | Boolean          | Whether the output is active in the current layout.                  |
| `primary`         | Boolean          | Whether this is the primary output.                                  |
| `monitor_id`      | String or `nil`  | ID of the logical monitor that shows this output.                    |
| `index`           | Integer or `nil` | Zero-based logical monitor index. It can change when outputs change. |
| `mode`            | Record           | `width` and `height` in physical pixels; `refresh_rate` in Hz.       |
| `scale`           | Number           | Compositing scale of the logical monitor.                            |
| `transform`       | String           | Rotation or reflection, such as `"normal"` or `"90"`.                |
| `x`, `y`          | Integer or `nil` | Logical position of the output.                                      |
| `width`, `height` | Integer or `nil` | Logical size of the output.                                          |
| `work_area`       | Record or `nil`  | Area that windows can use. See below.                                |
| `privacy_screen`  | Record           | Privacy screen state. See [Privacy screens](#privacy-screens).       |
| `renderer`        | Record           | `Renderer` record for the GPU path that renders this output.         |
| `revision`        | Integer          | State revision of this record.                                       |

A disabled output has no `monitor_id`, `index`, position, size or work area.

`monitor_id` is the lowest connector name in the logical monitor. Windows and
layer surfaces name the same ID in their `monitor_id` field, and
`window:move_to_monitor(id)` takes it. When two outputs show one cloned
logical monitor, both have the same `monitor_id`.

`work_area` has `x`, `y`, `width` and `height` in logical pixels. It is the
logical monitor minus the space that panels reserve with exclusive zones, on
the active workspace.

The `transform` values describe the output orientation:

- `normal`: no rotation or reflection.
- `90`, `180`, and `270`: rotate clockwise by that amount.
- `flipped`: mirror the output.
- `flipped-90`, `flipped-180`, and `flipped-270`: mirror the output, then apply
  the named clockwise rotation.

## Privacy screens

`output.privacy_screen` has these fields:

| Field       | Type             | Meaning                                                               |
| ----------- | ---------------- | --------------------------------------------------------------------- |
| `available` | Boolean          | Whether this output has a privacy screen that Gnoblin can control.    |
| `enabled`   | Boolean or `nil` | Whether the privacy screen is on. Present when `available` is true.   |
| `locked`    | Boolean or `nil` | Whether a hardware switch locks it. Present when `available` is true. |
| `requested` | Boolean          | Whether the session asks for privacy screens to be on.                |
| `source`    | String enum      | What sets the request.                                                |

`source` has one of these values:

- `runtime`: `output:set_privacy_screen` set a runtime override.
- `config`: `monitors.privacy_screen` is set in the Lua configuration.
- `system`: neither a runtime override nor the configuration sets a value.

`output:set_privacy_screen(enabled)` returns an `Operation`. It accepts:

- `true`: turn privacy screens on for this session.
- `false`: turn them off for this session.
- `"inherit"`: remove the runtime override and use the configured value, or
  the system preference when the configuration has none.

The compositor keeps one privacy screen request for all outputs. The method
changes that request, so every output with a privacy screen follows it. It
fails on an output whose `privacy_screen.available` is false. A runtime
override does not change the system preference.

```lua
for _, output in ipairs(gnoblin.outputs.list()) do
    if output.privacy_screen.available then
        output:set_privacy_screen(true)
        break
    end
end
```

When the state changes, each affected output raises `"changed"` with
`privacy_screen` in its changed list.

## Output events

| Event                    | Fields                           | Raised when                   |
| ------------------------ | -------------------------------- | ----------------------------- |
| `gnoblin.output.added`   | `output_id`, `output`            | A display is connected.       |
| `gnoblin.output.changed` | `output_id`, `output`, `changed` | A field of an output changes. |
| `gnoblin.output.removed` | `output_id`, `last`              | A display is disconnected.    |

`changed` lists the changed `Output` fields. The live object updates and runs
its own signals and watches before `gnoblin.on` callbacks run.

## Renderer records

| Field         | Type            | Meaning                                   |
| ------------- | --------------- | ----------------------------------------- |
| `name`        | String          | GL renderer string, such as `"llvmpipe"`. |
| `vendor`      | String or `nil` | GL vendor string, when available.         |
| `api`         | String          | GL API in use, such as `"gles2"`.         |
| `accelerated` | Boolean         | `false` for software rasterizers.         |

Gnoblin reports `accelerated = false` for the software rasterizers llvmpipe,
softpipe, swrast, and Software Rasterizer, and when Mutter reports software
rendering. All outputs share one compositor renderer record.

Display configuration such as modes, scale and layout stays with the session
display-configuration interface.
