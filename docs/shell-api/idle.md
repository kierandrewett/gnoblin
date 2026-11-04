# Idle watches

`gnoblin.idle.watch(seconds)` runs your code when the session has had no input for a set time. Use it to dim a
panel, lock the screen or suspend the machine after a pause. It returns an `IdleWatch`.

```lua
local watch = gnoblin.idle.watch(300)

watch:on("idle", function()
    gnoblin.session.lock()
end)

watch:on("active", function()
    print("the user is back")
end)
```

`seconds` is a number above 0 and at most 86400. A fraction is allowed. You can make up to 256 watches for each
runtime, and each watch has its own time. The call raises an error for any other value.

Mutter's idle monitor counts the time. An idle inhibitor of the session, such as a video player, delays `"idle"` in the
same way that it delays the screen blank.

## The IdleWatch object

| Field     | Value                                                               |
| --------- | ------------------------------------------------------------------- |
| `id`      | Identifier of this watch, `"idle-"` and a number.                   |
| `alive`   | `true` until the watch has ended.                                   |
| `seconds` | The idle time you asked for.                                        |
| `idle`    | `true` while the session has been idle for `seconds` with no input. |

The fields are read-only and change as the session goes idle and becomes active.

A watch is not idle when it starts. Input after an idle period makes `idle` false and runs `"active"`. The watch then
starts to count again, so it runs `"idle"` for every idle period until you cancel it.

`watch:on(signal, fn)` takes one of these signals and returns a subscription with an `unsubscribe()` method:

| Signal     | Runs when                                              |
| ---------- | ------------------------------------------------------ |
| `"idle"`   | The session has been idle for `seconds`.               |
| `"active"` | The first input arrives after an idle period.          |
| `"closed"` | The watch has ended. Listening ends after this signal. |

`fn` receives the watch. A watch that the compositor refused passes the reason as a second argument to `"closed"`.

`watch:watch(field, fn)` calls `fn(watch, value, old)` when a field changes, for example `idle`.

## Stop a watch

`watch:cancel()` returns an `Operation`. The watch ends, and its `"closed"` listeners run, when the compositor has
removed it. The operation fails with `not_found` for a watch that has already ended.

```lua
local watch = gnoblin.idle.watch(60)
watch:on("idle", function()
    watch:cancel()
end)
```

A configuration reload ends every watch, and the new configuration makes its own. The same happens when the Lua
worker restarts and when the compositor stops.

## Idle watches in `gnoblinctl lua`

A `gnoblinctl lua` script ends before the session is idle, so `gnoblin.idle.watch` raises an error there. Make the
watch in a configuration file.

## Other ways to read idle state

Socket clients read the idle sample with `gnoblinctl session activity` or the `session.activity` call, and receive
`gnoblin.session.activity-changed`. That sample uses a fixed threshold of 120 seconds. See the
[compositor bridge](/bridge/session-and-windows#session-activity). The
[session idle interface](/dbus/session-idle) gives the same sample on the session bus.

`gnoblin.session.activity()` is not in the Lua API. Use `gnoblin.idle.watch` with the time that you need.
