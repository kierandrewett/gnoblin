# Shared types

The records and small types that more than one part of the API uses. `gnoblin.d.ts` in your config directory declares every
type in full: `gnoblinctl types init` writes it, and your editor reads it for completion. See
[Editor support](/guides/editor_support).

## Geometry

Coordinates are logical pixels in the compositor's space. An output further right on the desktop starts at a larger `x`.

| Type     | Fields                      | Used by                                                                     |
| -------- | --------------------------- | --------------------------------------------------------------------------- |
| `Vec2`   | `x`, `y`                    | `Output.position`, the seat's `pointer`.                                    |
| `Vec3`   | `x`, `y`, `z`               | A node's position.                                                          |
| `Size2D` | `width`, `height`           | A window's `committedSize` and `size`.                                      |
| `Rect`   | `x`, `y`, `width`, `height` | `Client.bounds`, `Output.bounds` and `area`, `Placement.bounds`.            |
| `Gaps`   | `inner`, `outer`            | `Workspace.gaps`. Pixels between windows, and between windows and the edge. |

## Values

| Type           | Meaning                                                                                             |
| -------------- | --------------------------------------------------------------------------------------------------- |
| `Color`        | A string: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.                                                |
| `Trigger`      | A binding's trigger, such as `Super+Return`. See [Keyboard shortcuts](keyboard-shortcuts#triggers). |
| `Subscription` | What `gnoblin.on` and `Operation.onComplete` return. `unsubscribe()` stops the callback.            |
| `Operation<T>` | The result of a call that finishes later. See [Operations, errors and timers](errors).              |
| `Timer`        | A pending call of a function at a time. See [Operations, errors and timers](errors#timers).         |

## Live objects

A `Client`, a `Workspace` and an `Output` are live objects: every read of one thing returns the same object, so you can keep
it. `alive` turns `false` once the thing is gone, and the object keeps its last values.

`gnoblin.clients`, `gnoblin.workspaces` and `gnoblin.outputs` are collections of them. A collection is live and read-mostly.
Besides `length`, `at`, `get`, `find`, `filter` and `map`, which [Windows](windows#read-windows) describes, it has:

- `where(query)`, a live selection: [Find objects](find#where-on-a-collection).
- `focused`, the member that holds the keyboard focus, or `null`. Only windows take the focus.
- `addListener(listener)` and `removeListener(listener)`: [Events](/events/desktop-state).

`gnoblin.workspaces` is the one collection a script may change, with `add`, `remove` and `clear`.

## Errors

A native failure is a `GnoblinError`. Its `code` is `destroyed`, `read-only`, `refused` or `spawn-failed`, and its `target`
names the object and member, such as `Client#7.x`.

A value of the wrong type is a `TypeError`, and a value out of range a `RangeError`. An operation that fails carries `{ code, message }` instead: see
[Operations, errors and timers](errors#error-codes).
