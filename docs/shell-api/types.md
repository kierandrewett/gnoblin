# Runtime Types

> **Status:** not available yet: `gnoblin.input.on`. The sections that use them describe the target, not current behaviour.

Use these shared record descriptions when reading snapshots and event payloads.

The Lua runtime exposes records after native startup completes; exact availability is stated on each domain page. Most records are [live objects](#live-objects) that stay current; the rest are immutable snapshots. Records can carry a `revision` so clients can identify the state version they read.

## Shared records

Runtime APIs return records including `Window`, `Workspace`, `Output`, `LayerSurface`, `InputDevice`, `InputSource`, `Launch`, `Capability`, `PrivacyState`, and `SessionStatus`. Each domain page lists the fields and methods for its records:

- [Windows](/shell-api/windows)
- [Workspaces](/shell-api/workspaces)
- [Layer surfaces](/shell-api/layer-surfaces)
- [Input](/shell-api/input)
- [Launch feedback](/shell-api/launch-feedback)
- [Privacy](/shell-api/privacy)
- [Session and runtime health](/shell-api/session-runtime)

## Live objects

A live object is one handle for one thing. Every read of it returns the same object, which shows the current state
the runtime has seen. All live objects share these members:

| Member                 | Meaning                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------- |
| `alive`                | `false` once the object has ended. The object keeps its last state.                         |
| `revision`             | Counts changes of the object's state, where its fields change. Read it; do not watch it.    |
| `object:on(signal,fn)` | Listen to a signal the kind declares. `"closed"` ends the object; `"changed"` lists fields. |
| `object:watch(f, fn)`  | Call `fn(object, value, old)` when the observable field `f` changes.                        |

`on` and `watch` return a subscription, and so do `gnoblin.on`, `gnoblin.once`, `Request:on` and
`gnoblin.input.on`. Call `subscription:unsubscribe()` to stop; calling it again does nothing.

Select objects of one kind with [`gnoblin.find`](/shell-api/find), by field value or by a predicate function.

`watch` accepts only a field that the class marks observable, which is one that changes while the object is alive.
`id`, `alive`, `revision` and the fields that never move are refused with an error that lists the fields that work.

An object made by the runtime ends when the runtime does. A reload runs the `"closed"` listeners once, with the reason
`"config_reloaded"`, as the old runtime goes. The reason is `"runtime_stopped"` when the worker stops. The listeners can
read, but they cannot start an operation.

- These objects end with the runtime: `Shortcut`, `IdleWatch`, `Launch`, `Override`, `Grab`, `Process`, `Tween` and
  `AnimationPreview`.
- A pending `Request` runs its `"closed"` listeners the same way.
- Windows, workspaces, outputs, layer surfaces and input sources belong to the compositor and carry on.

Events carry `name`; Gnoblin events commonly also carry `sequence` and monotonic `time`. Their event-specific records are described in the [event reference](/events/).

## Inspect a window record

This supervised runtime callback reads the shared `Window` record described on
the [Windows reference](/shell-api/windows). Save it in the shell config loaded
by Gnoblin, then open or focus a window. The callback prints its application ID
and title.

```lua
gnoblin.on("gnoblin.window.created", function(event)
    print(event.window.app_id, event.window.title)
end)
```
