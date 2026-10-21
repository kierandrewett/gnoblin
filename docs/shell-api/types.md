# Runtime Types

Use these shared record descriptions when reading snapshots and event payloads.

The Lua runtime exposes immutable snapshot records after native startup completes; exact availability is stated on each domain page. Records can carry a `revision` so clients can identify the state version they read.

## Shared records

Runtime APIs return records including `Window`, `Workspace`, `Monitor`, `LayerSurface`, `InputDevice`, `InputSource`, `Launch`, `Capability`, `PrivacyState`, and `SessionStatus`. Each domain page lists the fields and methods for its records:

- [Windows](/shell-api/windows)
- [Workspaces and monitors](/shell-api/workspaces-monitors)
- [Layer surfaces](/shell-api/layer-surfaces)
- [Input](/shell-api/input)
- [Launch feedback](/shell-api/launch-feedback)
- [Privacy](/shell-api/privacy)
- [Session and runtime health](/shell-api/session-runtime)

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
