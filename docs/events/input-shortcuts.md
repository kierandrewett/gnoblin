# Input and Shortcut Events

Subscribe to input gestures, tablet-pad help, shortcut activations, and
operation completion in a Lua configuration.

These events are available when the session supports the relevant input source
and API. For socket subscriptions, see the [compositor bridge](/bridge/).

## Touchpad gestures

`gnoblin.input.gesture` reports a touchpad swipe, pinch, or hold. Each event has
`gesture`, `phase`, `fingers`, `sequence`, and monotonic `time` fields. The
phase is `begin`, `update`, `end`, or `cancel`.

Swipe updates include `dx` and `dy`. Pinch updates include `scale` and
`angle_delta`. Hold events have no movement fields. Deltas are per update, so
add them in a callback when a total movement is needed.

```lua
gnoblin.on("gnoblin.input.gesture", function(event)
    if event.gesture == "swipe" and event.phase == "update" then
        print(event.fingers, event.dx, event.dy)
    end
end)
```

## Tablet-pad help

When a configured tablet-pad `help` action runs, Gnoblin sends
`gnoblin.input.pad-help-requested` to Lua listeners. The event describes
zero-based buttons, mode groups, and rings, strips, or dials. Labels are
included when available and localized by the input stack.

Subscribe in a Lua file and run it with `gnoblinctl lua pad-help.lua`:

```lua
gnoblin.events.on("gnoblin.input.pad-help-requested", function(event)
    print(event.device.name)
    for _, button in ipairs(event.buttons) do
        print(button.button, button.label or "")
    end
end)
```

The event supplies data only. The shell draws and dismisses its own overlay.
Each press sends a new event; without a listener, no overlay appears. See the
[tablet-pad help reference](/shell-api/tablet-pad-help) for all payload fields.

## Shortcut sessions

`gnoblin.shortcut.session.key` reports the key value, key code, modifiers,
phase, and time. Its phase is `press` or `release`.

Real, non-repeated input may include `focus_context`. Synthetic and
input-method events do not carry authority. The context expires after five
seconds and authorizes one focus-sensitive compositor operation.

For configured command shortcuts, `gnoblin.shortcut.activated` reports the
shortcut name and trigger. Dynamic bindings use `gnoblin.shortcut.binding-activated`;
held bindings also send session events. Event order depends on the binding's
`trigger`:

- `press` activates on key-down and deactivates on key-up.
- `release` activates on key-up and has no later deactivation event.

Lua callbacks can subscribe to events and register dynamic shortcuts. See
[Keyboard shortcuts](/shell-api/keyboard-shortcuts) for accepted Lua options.

## Operation completion

Use `gnoblin.operation.completed` to observe asynchronous operations. The event
contains `operation_id`, `method`, and boolean `ok`. Success includes `value`;
failure includes an `Error` record with `code` and `message`. In Lua,
`operation_id` matches the returned operation handle's `id`.
