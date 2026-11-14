# Input and seats

`gnoblin.input` gives a script the compositor's seats and their input devices. A seat is a keyboard focus, a pointer and
the devices that drive them. Use it to read the pointer, list devices, make a device keys-only, or bind a chord for one
device.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    setup: (gnoblin) => {
        const pad = gnoblin.input.devices.find((device) => device.name === "Elgato Stream Deck");
        pad?.bind(
            "F13",
            () => {
                gnoblin.spawn("foot");
            },
            { only: true },
        );
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`, and use the name your device reports. Its key `F13` starts a terminal, and
the device types nothing into a window.

## Seats

`gnoblin.input` has these members:

| Member     | Meaning                                                        |
| ---------- | -------------------------------------------------------------- |
| `contexts` | The seats, each an `InputContext`.                             |
| `primary`  | The first seat. It throws when the compositor reports no seat. |
| `devices`  | Every device of every seat.                                    |

An `InputContext` is one seat:

| Member                     | Meaning                                                                       |
| -------------------------- | ----------------------------------------------------------------------------- |
| `name`                     | The seat's name.                                                              |
| `devices`                  | The devices on this seat.                                                     |
| `bindings`                 | The context's bindings, by id.                                                |
| `focusedClient`            | The window that holds the keyboard focus, or `null`. Read on every access.    |
| `pointer`                  | The pointer in compositor coordinates, `{ x, y }`.                            |
| `settings`                 | The window-management settings the seat publishes. See below.                 |
| `clearFocus()`             | Takes the keyboard focus from every window.                                   |
| `hitTest(position)`        | The topmost window under a point and the point in its coordinates, or `null`. |
| `bind(trigger, action)`    | Binds a chord for every device. See [Keyboard shortcuts](keyboard-shortcuts). |
| `describeBindings()`       | Every binding on the seat with its owner, priority and rank.                  |
| `makeBindingOnly(name)`    | Makes the devices called `name` binding-only.                                 |
| `releaseBindingOnly(name)` | Gives that up. Returns `false` when there was none.                           |
| `isBindingOnly(name)`      | Whether those devices are binding-only.                                       |

`settings` holds the values the compositor publishes for the seat, with the config's `edgeTiling` and `dragConstraint` laid
over them:

| Field                       | Values                           | Meaning                                        |
| --------------------------- | -------------------------------- | ---------------------------------------------- |
| `edgeTiling`                | boolean                          | Dropping a dragged window on an edge tiles it. |
| `focusMode`                 | `click`, `hover`, `hover-strict` | How pointer movement changes focus.            |
| `focusNewWindows`           | `allow`, `prevent`               | Whether a launched window takes focus.         |
| `raiseOnClick`, `autoRaise` | boolean                          | Whether a click or the focus raises a window.  |
| `autoRaiseDelay`            | milliseconds                     | The delay before an automatic raise.           |
| `focusChangeOnPointerRest`  | boolean                          | Focus follows the pointer once it stops.       |
| `reducedMotion`             | boolean                          | The user asked for reduced motion.             |

See [Focus and activation](focus-activation) for what the focus settings do.

## Devices

An `InputDevice` is one keyboard, mouse, touchpad or other device. `gnoblin.input.devices` lists them in the order they
arrived. A device that is unplugged and plugged in again has a new `id`.

| Member                            | Meaning                                                                                   |
| --------------------------------- | ----------------------------------------------------------------------------------------- |
| `id`                              | The compositor's number for the device, as a string.                                      |
| `name`                            | The name the device reports, such as `Logitech K400`. Several devices may share one name. |
| `capabilities`                    | What the device can do. See below.                                                        |
| `context`                         | The seat the device is on.                                                                |
| `bindings`                        | The bindings that match only this device, by id.                                          |
| `bind(trigger, action, options?)` | Binds a chord for this device.                                                            |
| `bindingOnly`                     | Whether the device is binding-only. Assign to change it.                                  |

`capabilities` is a list of these values: `keyboard`, `pointer`, `touch`, `gesture`, `tablet-tool`, `tablet-pad`, `switch`.

Input that no listed device made, such as a virtual keyboard or a remote-input client, has no entry and no name, so no device binding
matches it. The list changes when a device is plugged in or removed.

Find the names of your devices by listing them from the console:

```sh
gnoblinctl console --eval 'gnoblin.input.devices.map((d) => d.name)'
```

### Bind a chord for one device

`device.bind(trigger, action, options?)` is `context.bind` for the events of this one device. Its bindings run before a
layout's and the context's, so a device can give a key a meaning of its own. The action rules are those of
[Keyboard shortcuts](keyboard-shortcuts#bind-from-a-script).

- The device is matched by name, so the binding survives the pad being unplugged and plugged in again, and two devices that
  report the same name are both matched.
- Every other device still reaches the bindings made for every device.
- A device binding matches the modifiers that device holds, not the seat's: a Shift held on another keyboard does not turn the
  pad's `F13` into `Shift+F13`. A lock key such as Caps Lock still counts, because a lock is the seat's state.
- A gesture trigger takes the touchpad's name the same way.

### Binding-only devices

A binding-only device never types into a window: its keys reach no client and fire only the bindings made for it or for
every device. Use it for a macro pad.

- `device.bind(trigger, action, { only: true })` makes the device binding-only before it binds. `only: false` is refused.
- `device.bindingOnly = true` does the same without a binding, and `device.bindingOnly = false` gives the claim up.
- A chord no binding claims is dropped for a binding-only device.
- Only the keys are withheld. The device's pointer buttons, touches and gestures behave as before.
- A key the device held when you made it binding-only is released to the window it went to, so that window is not left with
  the key down.
- The claim is by device name and works before the device is plugged in: a pad plugged in later is binding-only from its
  first key.
- The claim goes with the shell. A shell that exits, crashes or restarts leaves the pad typing again until the next shell
  claims it.
