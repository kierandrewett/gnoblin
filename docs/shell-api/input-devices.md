# Input devices

The main seat lists the input devices it has in `seat.device`, and a binding can name one of them.

## The device list

`seat.device` is a read-only list of text items, one per device, in the order the devices arrived. Each item is
`id:name:caps`:

- `id` is the compositor's number for the device. A number is never reused, so a keyboard that is unplugged and
  plugged in again has a new one.
- `name` is the name the device reports, such as `Logitech K400`.
- `caps` is what the device can do, joined by commas: `keyboard`, `pointer`, `touch`, `gesture`, `tablet-tool`,
  `tablet-pad` and `switch`.

The id is up to the first colon and the capabilities are after the last, so a name that holds a colon still parses:

```lua
for _, item in ipairs(seat.device) do
    local id, name, caps = item:match("^(%d+):(.*):([%w,-]*)$")
    print(id, name, caps)
end
```

The list changes when a device is plugged in or removed. Input that no listed device made (a virtual keyboard, a
remote-input client, a nested session) has no entry and no name, so no device binding matches it.

## Binding to one device

Put `device = "name"` in the map you give `Seat:bind`. Every binding in the map takes its events from the devices that
report that name, matched exactly:

```lua
seat:bind({
    device = "Macro Pad",
    F13 = function() launch("kgx") end,
})
```

The pad's `F13` is withheld from clients and runs the function. `F13` on any other keyboard is typed as usual. The
name, not the id, is the selector, so the binding survives the pad being unplugged and plugged in again. Two devices
that report the same name are both matched.

- A chord is bound once for each device name. `F13` for `Macro Pad`, `F13` for another pad and `F13` for every device
  are three bindings. Two for the same name are refused.
- A binding for a device wins on that device. Every other device still reaches the binding for every device.
- A binding for a device matches the modifiers that device holds, not the seat's. A Shift held on the desk keyboard does
  not turn the pad's `F13` into `Shift+F13`. A Caps Lock or Num Lock still counts: a lock is the seat's state. A binding
  for every device matches the seat's modifiers, from every keyboard, as before.
- `Swipe3` and `Pinch4` take the same `device`: it names the touchpad whose gesture is claimed.
- `seat:claim(chord, grab, device)` and `seat:claim_gesture(kind, fingers, device)` take the name as an extra argument.
  A name that is empty or spans lines is an error.

## A device that only binds

A macro pad should fire shortcuts and never type into a window. Add `only = true` beside `device`:

```lua
seat:bind({
    device = "Macro Pad",
    only = true,
    F13 = function() launch("kgx") end,
})
```

From then on the keys of the devices that report `Macro Pad` reach no client, the seat's key state or the shell's
text input. They run the bindings made for the pad and the bindings made for every device, and a key with no binding
is dropped. `seat:claim_device(name)` does the same without a binding and returns an id, and `seat:unclaim(id)` ends it.

- The pad's keys are its own. A Shift held on the pad does not change what the desk keyboard types, and a binding
  matches the symbol and modifiers the pad's own keys make. A Caps Lock or Num Lock still counts, as for any binding for
  a device.
- A key the pad held when you made it binding-only is released to the client it went to, so that client is not left
  with the key down.
- A pad that is plugged in after the claim is binding-only from its first key.
- Only the keys are withheld. The device's pointer buttons, touches and gestures behave as before.
- `only = true` needs `device`. `only = false` is an error: leave it out.
- The claim goes with the shell. A shell that exits, crashes or is restarted leaves the pad typing again until the
  next shell claims it.

## Limits

A key is down for the seat once any keyboard holds it, and up once none does. That is what a client is told.

- A second keyboard's press of a key the first still holds is not a second press to a client.
- The first keyboard's release of that key is not a release while the second holds it.
- A binding for one device sees every edge of that device. The pad's chord fires when it presses a key the desk
  keyboard still holds.
- A binding for every device is the seat's, so a press of a held key does not fire it again.

Every device attaches to the one input seat, the default seat. Its name is `seat0` unless the session names another.
A device the system tagged for a seat this session does not have (libinput's `default`, or a `WL_SEAT` set on another
machine) attaches to it too, and so does a device plugged in later.

The list names devices. It does not put them on different seats.
