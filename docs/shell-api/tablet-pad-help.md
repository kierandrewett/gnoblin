# Tablet Pad Help

Handle tablet-pad help requests and present the available button and mode labels.

The event is available when Mutter reports a tablet-pad help request.

## Tablet-pad help

Subscribe to `gnoblin.input.pad-help-requested` to show help for a tablet pad
whose configured button action is `"help"`. Lua listeners receive the event
directly. See [Lua events](/events/) for callback behavior.

The event contains:

- `device`: the current `gnoblin.input.devices()` record for the pad;
- `monitor_id` and `output_names`: optional stable target-monitor identity;
- `buttons`: zero-based button numbers with optional localized action labels
  and optional `mode_group` values;
- `mode_groups`: group numbers, each group's `mode_count`, and its
  `switch_buttons`;
- `features`: zero-based rings, strips, and dials with optional labels for
  clockwise, counterclockwise, up, or down actions;
- `edition_mode`: whether the request is for editing; button help
  sends `false`.

The event provides labels when available, but no image or popup. Draw and
dismiss the overlay in your shell. Each help press sends a new event; without a
listener, no overlay appears.

To inspect the labels in a session, save this as `pad-help.lua` and run
`gnoblinctl lua pad-help.lua`:

```lua
gnoblin.events.once("gnoblin.input.pad-help-requested", function(event)
    for _, button in ipairs(event.buttons) do
        print(button.button, button.label or "")
    end

    for _, group in ipairs(event.mode_groups) do
        print("mode group", group.group, group.mode_count)
    end
end)
```

Press the configured pad help button while the listener is running. The console
prints each button label and mode count; a shell can use the same fields to draw
its own overlay.
