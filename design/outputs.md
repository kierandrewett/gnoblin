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
manager. `virtual` is reserved for a future software-created output. `display` is the
only kind backed by Mutter's monitor manager. Renderer identity comes from the compositor GL context and is cached once; outputs currently share the compositor renderer record.

## Display configuration

The future write API may expose operations such as
`output:set_mode{width, height, refresh_rate}` or
`gnoblin.outputs.apply(layout)`. These are typed operations with stable output
IDs, validated arguments, and operation handles. The session display
configuration interface owns validation, application, rollback, and persisted
profiles.

## Non-goals

The initial API does not configure modes, profiles, or virtual outputs.
