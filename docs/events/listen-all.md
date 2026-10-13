# Listen to Forwarded Events

Subscribe to all forwarded events when inspecting an integration or building a generic event viewer.

The event set depends on the running Mutter build and supported Gnoblin API version.

## Listen to every forwarded event

Pass `"*"` to receive every event source currently forwarded by Gnoblin:

```lua
gnoblin.on("*", function(event)
    print(event.name)
end)
```

Registering a name does not create an event source; Mutter or Gnoblin must
dispatch it. Signal availability follows the Mutter version used to build
Gnoblin.

`gnoblin.listeners` maps each registered event name to its callback list.
Inspect it to see which handlers earlier config files have added. The list is
for inspection; remove handlers through their `Subscription`:

```lua
for name, callbacks in pairs(gnoblin.listeners) do
    print(name, #callbacks)
end
```

Callbacks run synchronously on the supervised Lua runtime event loop, separate
from Mutter's compositor main thread. Keep handlers short, especially for
high-frequency `*.input.*` events.

Operations requested by a callback are dispatched after it returns.

`gnoblin.configure` uses the same live-apply transaction as
`gnoblinctl config reload`. Xwayland startup options restart the Xwayland child
and disconnect running X11 apps.

If a listener fails, Gnoblin logs the error and continues with the remaining
listeners. It rolls back configuration changes from that event while still
dispatching operations queued by its callbacks.

If the resulting config fails validation, Gnoblin restores the last valid
config and fails those queued operations. The compositor applies accepted
settings after the callback returns.
