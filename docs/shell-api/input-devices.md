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

## Claiming a pointer drag

A claim whose chord ends in a pointer button takes that button's drag.

- `Seat.claim` takes the chord as text: `<Super>Button1`, `<Super>Button3`, or the back button on its own, `Button8`.
- The button is a number from 1 to 9, as X11 and GTK number them: 1 is left, 2 middle, 3 right, 8 back and 9 forward.
- `Button1` to `Button7` need a modifier, so a claim cannot take the ordinary click.
- A claim for one pointer takes the pointer's name as its first line, as a keyboard chord does.
- `grab` has no effect on a pointer claim: the release always comes back.

With exactly the chord's modifiers held, a press of the button is withheld from every client and from the shell's
nodes. The owner hears the drag on the seat as the `pointer_chord` signal instead of the `chord` signal:

| Field       | Meaning                                                                                             |
| ----------- | --------------------------------------------------------------------------------------------------- |
| `phase`     | `start` at the press, `update` as the pointer moves, `end` at the release, `cancel` when it is cut. |
| `claim`     | The id the shell gave the claim.                                                                    |
| `x`, `y`    | Where the pointer is, in global logical coordinates.                                                |
| `dx`, `dy`  | How far the pointer has come since the press. Both are 0 at `start`.                                |
| `button`    | The chord's button number.                                                                          |
| `modifiers` | The chord's modifiers as it spells them, such as `<Super>`. Empty for `Button8`.                    |
| `time`      | The input event's time in milliseconds.                                                             |

- The drag belongs to the button. Letting go of the modifier first does not end it: `end` comes with the release.
- No client hears the pointer move while the drag lasts, so a window that follows the drag does not see the pointer
  cross it. The seat's `pointer_x` and `pointer_y` follow the pointer as usual. When the drag ends the pointer is given
  the surface under it.
- `update` comes at most once a frame and carries the place the pointer has reached, so a shell that is slower than the pointer is
  told where it is, not where it was. `start`, `end` and `cancel` are never merged or dropped.
- A press with other modifiers held, a press of another button, and a press while the session is locked are not the
  claim's: the client gets them as it always did, and the shell hears nothing.
- A session that locks during a drag ends it with `cancel`, once. A pointer that is unplugged during a drag does the
  same. Neither sends anything after it.
- A shell that exits, crashes or is restarted, or that gives the claim up with `Seat.unclaim`, ends the drag without
  a word to anyone. The button's release is withheld from the client as its press was, so the client sees a click it
  never started only if the shell makes one.
- Two claims of one chord for one pointer (or for every pointer) are refused: the second is dropped.

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
The compositor runs on one logind seat, so a device of another logind seat never reaches it.
