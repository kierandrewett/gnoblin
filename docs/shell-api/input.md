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

| Member     | Meaning                                                          |
| ---------- | ---------------------------------------------------------------- |
| `contexts` | The seats, each an `InputContext`, then the ones a script added. |
| `primary`  | The first seat. It throws when the compositor reports no seat.   |
| `devices`  | Every device of every seat.                                      |

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
| `id`, `alive`              | The name, and whether the context is still in `gnoblin.input.contexts`.       |

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
| `context`                         | The context that holds the device. Assign another to move it, see below.                  |
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

## Device classes and their settings

A device is an instance of the class its capabilities name. Test with `instanceof`:

- `Keyboard`: a keyboard, also one with a pointing stick.
- `Mouse`: any other pointer.
- `Touchpad`: a pointer that reports `gesture`.
- `Touchscreen`: a device with `touch`.
- `Tablet`: a device with `tablet-tool` or `tablet-pad`.
- A device that is none of these, such as a lid switch, is a plain `InputDevice`.

```ts
import { gnoblin, Keyboard, Mouse, Touchpad } from "gnoblin";

for (const device of gnoblin.input.devices) {
    if (device instanceof Mouse) {
        device.sensitivity = 0.5;
        device.acceleration = "flat";
    }
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
    if (device instanceof Touchpad) {
        device.naturalScroll = true;
        device.tapToClick = true;
    }
}
```

| Class      | Property                          | Values                                    | Written to                                      |
| ---------- | --------------------------------- | ----------------------------------------- | ----------------------------------------------- |
| `Keyboard` | `repeatRate`                      | repeats per second, 0.1 to 1000           | `input.keyboard.repeat-interval` (milliseconds) |
| `Keyboard` | `repeatDelay`                     | whole milliseconds, 1 to 10000            | `input.keyboard.delay`                          |
| `Mouse`    | `sensitivity`                     | -1 to 1                                   | `input.mouse.speed`                             |
| `Mouse`    | `acceleration`                    | `"default"`, `"flat"` or `"adaptive"`     | `input.mouse.accel-profile`                     |
| `Mouse`    | `naturalScroll`                   | boolean                                   | `input.mouse.natural-scroll`                    |
| `Touchpad` | the three above, and `tapToClick` | as for `Mouse`; `tapToClick` is a boolean | `input.touchpad.*`, `tap-to-click`              |

What backs them:

- The compositor reads device settings from the configuration document's `input` section, once for each kind of device. A
  write on one `Mouse` is the setting of every mouse, and another mouse reads the value back. A setting nobody wrote reads
  `undefined`, and assigning `undefined` gives it back to the system's value.
- The shell commits the document with these writes when the entry loads, and again on the frame after a later write (for
  example from a key handler). A value outside the range throws a `RangeError` at the write; one the document refuses
  throws a `GnoblinError` with code `refused` and the document's reason.
- The compositor applies `input.keyboard` (repeat and delay) when the document is committed. It accepts and checks
  `input.mouse` and `input.touchpad` and reports them as not applied: no device reads them yet, see
  [Input settings](/config/configure/input). A `Touchscreen` and a `Tablet` have no setting here.
- `device.enabled` is always `true`. Assigning `false` throws a `GnoblinError` with code `unsupported`, because nothing can
  disable one device. Use `bindingOnly` to keep a device's keys from windows.

## Your own input context

`new InputContext({ name })` makes a context, and `gnoblin.input.contexts.add(context)` puts it in the list. A device
assigned to it with `device.context = context` is held by it, and the context's bindings match only the events of the
devices it holds:

```ts
import { gnoblin, InputContext } from "gnoblin";

const other = new InputContext({ name: "secondary" });
gnoblin.input.contexts.add(other);
other.bind("Super+Return", () => gnoblin.spawn("foot"));

const keyboard = gnoblin.input.devices.find((device) => device.name === "USB Keyboard");
if (keyboard) {
    keyboard.context = other;
}
```

Be exact about what is native and what is not. The compositor has one input seat and attaches every device to it, so:

- A context you add holds devices and decides which bindings match them. It is not a second `wl_seat`: pointer and keyboard
  focus stay the seat's, and `focusedClient` is the same window as the primary context's.
- A binding of the new context claims its chord for each device it holds. Add the binding or the device first, in either
  order. A context with no device claims nothing.
- A chord only the primary context binds still fires for a device of another context, because the compositor withholds a
  chord by claim and a claim for one device exists only where a binding asked for it. Bind the chord in the new context too
  (as `false`, to swallow it) when that matters.
- A layout's bindings (move and resize drags) stay with the seat's own context.
- Moving a device cancels a drag its old bindings run, so no button is left held. A device that is unplugged and plugged in
  again is a new device, back in its seat's context. A context cannot take a device of another seat.
- `contexts.add` throws a `GnoblinError` with code `duplicate` for a name that is taken, and `no-seat` when the compositor
  reports no seat.
- `contexts.remove(context)` hands the devices back to their seat's context and returns `true`. The seats' own contexts
  cannot be removed (code `refused`).
- A context that is not in the list throws code `detached` from `pointer`, `clearFocus`, `makeBindingOnly` and
  `attachLayout`.

`gnoblin.input.contexts` and `devices` are live lists: `length`, `at(index)`, `get(id)`, `find`, `filter`, `map`, `flatMap`
and iteration read what is there when they are called.

## Extensions

The registry lists the optional extensions the shell knows:

- Each entry has an `id` and a `status`.
- The XR program runs out of process, so its entry is `ready` while it has a session and `unloaded` otherwise.
- `gnoblin.extensions.xr` is `null` while it is unloaded.

```ts
for (const extension of gnoblin.extensions) {
    console.log(extension.id, extension.status);
}
```
