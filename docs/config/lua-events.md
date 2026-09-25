# Lua events

Use `gnoblin.on(name, callback)` to run Lua code when Gnoblin forwards a
compositor or shell event. The Lua runtime stays alive for the session, so
registered callbacks can change settings as events arrive. Saving and reloading
the config replaces the runtime and registers the callbacks again.

```lua
gnoblin.on("pointer_window_changed", function(window)
    local speed = window.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

This sets a slower touchpad scroll speed while the pointer is over Chromium.
Mutter sends `pointer_window_changed` when the Wayland pointer enters a
different surface, before later scroll events reach that surface. It does not
depend on keyboard focus or clicking the window. The event also fires when the
pointer leaves a client surface; then the window fields are empty strings.

## Event names and fields

Every callback receives one table. `event.name` contains the dispatched event
name. The table also contains the fields listed here.

| Event name                                   | Fields                                                               | Dispatched when                                                                                                      |
| -------------------------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `pointer_window_changed`                     | `app_id`, `wm_class`, `title`                                        | The Wayland pointer enters a different surface. Empty strings mean there is no application window under the pointer. |
| `focus_changed`                              | `app_id`, `wm_class`, `title`                                        | Mutter's keyboard-focused window changes. This can differ from the pointer window.                                   |
| `window_created`                             | `app_id`, `wm_class`, `title`                                        | Mutter creates a window.                                                                                             |
| `window_unmanaged`                           | `app_id`, `wm_class`, `title`                                        | Mutter removes a window.                                                                                             |
| `input.motion`                               | `type`, `time`, `x`, `y`                                             | A pointer motion event reaches the Clutter stage.                                                                    |
| `input.button_press`, `input.button_release` | `type`, `time`, `x`, `y`, `button`                                   | A pointer button event reaches the Clutter stage.                                                                    |
| `input.scroll`                               | `type`, `time`, `x`, `y`, `scroll_x`, `scroll_y`, `scroll_direction` | A scroll event reaches the Clutter stage.                                                                            |
| `input.key_press`, `input.key_release`       | `type`, `time`, `key_symbol`                                         | A keyboard event reaches the Clutter stage.                                                                          |

Other Clutter event types use `input.<type>` and include `type` and `time`.
Input events are delivered to the shell's captured-event handler. Events
consumed before reaching that handler are not included.

## Listen to every forwarded event

Pass `"*"` to receive each event listed above and any other named event that
Gnoblin dispatches:

```lua
gnoblin.on("*", function(event)
    print(event.name)
end)
```

`gnoblin.on` accepts any nonempty event name up to 128 bytes. Registering a
name does not create an event source: Mutter or the shell must dispatch that
name. The wildcard receives all names those sources dispatch.

Event callbacks run synchronously in the compositor's main thread, so keep
handlers short, especially for `input.*` events. `gnoblin.configure` changes are subject to the
usual live-setting rules; startup-only settings such as protocol registration
still need a new session.

If a callback fails, Gnoblin logs the error and keeps the settings from before
that event. For settings that are applied by the shell, Gnoblin applies the
updated document after the callback returns.
