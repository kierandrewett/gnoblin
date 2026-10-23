# Lua Events

Subscribe to Mutter and Gnoblin events from the supervised Lua runtime or a
`gnoblinctl lua` script.

Mutter event availability follows the running Mutter build. Gnoblin event names
and payloads are defined by the running Gnoblin API.

Use the [shell runtime API overview](/shell-api/) to find methods by task.
This page describes callback registration and event payloads.

Register repeating callbacks with `gnoblin.events.on`; use
`gnoblin.events.once` for one-time callbacks. Each returns a `Subscription`.

Remove a repeating callback with `subscription:unsubscribe()`. A `once`
subscription removes itself before invocation. `gnoblin.on` remains an alias
for `gnoblin.events.on`.

Subscribe to compositor signals with `gnoblin.events.mutter.on` or
`gnoblin.events.mutter.once`. Pass a full `mutter.*` event name. Available
signals depend on the running Mutter build and are not stable across versions.

Event names identify their source: `mutter.*` comes from Mutter and
`gnoblin.*` comes from Gnoblin. The supervised Lua runtime stays alive for the
session; reloading the config replaces its callbacks.

`gnoblinctl lua` can also subscribe while a script runs. It stays open until
every subscription has ended. A one-time subscription ends after its first
event; keep a repeating subscription in a variable so the callback can call
`subscription:unsubscribe()`. Press Ctrl+C to stop listeners manually.

For desktop color-scheme change notifications, subscribe to
`gnoblin.appearance.color-scheme-changed`. The [appearance guide](/guides/theming)
explains how applications use the preference. Use
`gnoblin.appearance.color_scheme()` to read the current value.

```lua
gnoblin.on("mutter.wayland.pointer-window-changed", function(event)
    local speed = event.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The Mutter pointer-window event changes when the pointer enters a different
Wayland surface, before later scroll events reach that surface. It does not
depend on keyboard focus or clicking the window. The event also fires when the
pointer leaves a client surface; then the window fields are empty strings.

## Event sources

## Event domains

- [Mutter](mutter)
- [Gnoblin lifecycle](lifecycle)
- [Input and shortcuts](input-shortcuts)
- [Session events](session)
- [Launch events](launch-portal)
- [Window, workspace, and monitor records](desktop-state)
- [Listen to all forwarded events](listen-all)

See the [Lua runtime API](/shell-api/) for subscriptions and event-bound operations.
