# Outputs

An output is a stable connector-backed rendering target. Its `id` is the
connector name; it does not imply that the output occupies a rectangle in the
desktop's logical layout.

The current read API is `gnoblin.outputs.list()`, `primary()`, `by_id(id)`,
and `renderer()`. An output record contains `id`, `kind`, optional identity
strings (`name`, `make`, `model`, `serial`), `enabled`, `primary`, and
`revision`.

An output record also carries:

- optional `monitor_id`,
- physical mode `{width, height, refresh_rate}`,
- `scale` and `transform`,
- optional logical `x` and `y`,
- its `renderer`.

Renderer records contain the GL renderer `name`, optional `vendor`, GL `api`,
and `accelerated`. Software rasterizers such as llvmpipe, softpipe, swrast, and
Software Rasterizer report `accelerated = false`.

## Kinds

`display` describes a physical display connected through Mutter's monitor
manager. `virtual` is reserved for a future software-created output. `xr` is
a headset output with per-eye views and no logical desktop rectangle. The optional
VR extension supplies it: the compositor reads the status file that `gnoblin-vr` writes and
adds one `xr:0` record while the file exists. Mutter has no registration interface for an
external output driver, so the record is read-only and carries `state`, `diagnostic` and
`views` instead of a mode. `display` is the only kind backed by Mutter's monitor manager. Renderer identity comes from the compositor GL context and is cached once; outputs currently share the compositor renderer record.

## Display configuration

The future write API may expose operations such as
`output:set_mode{width, height, refresh_rate}` or
`gnoblin.outputs.apply(layout)`. These are typed operations with stable output
IDs, validated arguments, and operation handles. The session display
configuration interface owns validation, application, rollback, and persisted
profiles.

## Spatial output

An XR output supplies per-eye views. The Lua runtime owns spatial policy: which
surfaces join the scene and where they go. Gnoblin supplies the OpenXR session,
surface state, authorized input, and frame timing, and never calls Lua on the
render or tracking thread.

Spatial policy stays out of the logical monitor rectangle model. The scene
contract lives in design/vr.md.

## Non-goals

The initial API does not configure modes, profiles, virtual outputs, or XR
devices. It does not define a spatial scene format and does not take over the
runtime's spatial policy in compositor code.
