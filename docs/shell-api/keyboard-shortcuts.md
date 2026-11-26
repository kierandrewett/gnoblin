# Keyboard shortcuts

A shortcut is a binding: a trigger, such as `Super+Return`, and an action. Make one with `bind(trigger, fn)` on the primary input context. The compositor withholds a claimed chord from
applications and tells the shell, so a shortcut never types into the focused window.

::: tabs

```typescript
const input = gnoblin.input.primary;
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
input.bind("Super+Shift+q", () => {
    input.focusedClient?.close();
});
input.bind("Super+x", () => {
    gnoblin.spawn(["notify-send", "x"]);
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
context = gnoblin.input.primary


def start_terminal(event):
    gnoblin.spawn(["foot"])


def close_focused(event):
    client = context.focused_client
    if client is not None:
        client.close()


def notify(event):
    gnoblin.spawn(["notify-send", "x"])


context.bind("Super+Return", start_terminal)
context.bind("Super+Shift+q", close_focused)
context.bind("Super+x", notify)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = Rc::new(ModelRoot::get(&objects)?);
    let input = model.input()?;
    let context = input.primary()?;
    let input = Rc::new(input);
    let (launcher, closer, notifier) = (Rc::clone(&model), Rc::clone(&input), Rc::clone(&model));
    context.bind(
        "Super+Return",
        Some(move |_event| {
            launcher.spawn(&["foot".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    context.bind(
        "Super+Shift+q",
        Some(move |_event| {
            if let Some(client) = closer.focused_client()? {
                client.close()?;
            }
            Ok(true)
        }),
        None,
    )?;
    context.bind(
        "Super+x",
        Some(move |_event| {
            notifier.spawn(&["notify-send".to_owned(), "x".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put a TypeScript shell in `~/.config/gnoblin/shell.ts`, a Python shell in `~/.config/gnoblin/shell.py`, or a Rust shell in its crate. `Super+Return` starts a terminal, `Super+Shift+q` closes the focused window and
`Super+x` shows a notification. A trigger that is already bound at the same priority is refused, so a file that starts from the
starter deletes the line it replaces.

## Triggers

A trigger is modifiers joined to one key or button with `+`. Two spellings of one trigger are the same trigger: its
`Triggers.parse(trigger).text` (`Triggers::parse` in Rust) is the canonical spelling.

| Kind    | Form                                                                | Examples                                         |
| ------- | ------------------------------------------------------------------- | ------------------------------------------------ |
| Key     | Modifiers, then a keysym name.                                      | `Super+Return`, `Alt+Shift+Tab`, `XF86AudioMute` |
| Pointer | Modifiers, a button, and optionally `Drag` or `Click`.              | `Super+MouseLeft+Drag`, `Alt+MouseRight`         |
| Wheel   | Modifiers, `Wheel`, and optionally `Up`, `Down`, `Left` or `Right`. | `Super+WheelUp`                                  |
| Gesture | `Swipe` or `Pinch`, 2 to 5 fingers, and optionally a direction.     | `Swipe3`, `Swipe4Left`, `Pinch2In`               |

- The modifiers are `Shift`, `Ctrl`, `Alt`, `Meta`, `Super` and `Hyper`. `Control`, `Primary`, `Mod1` and `Logo` are
  accepted for `Ctrl`, `Ctrl`, `Alt` and `Super`.
- A key is its keysym name, never the character it types. `Super+1` and `Super+Shift+1` are two triggers, and `Super+!` is
  refused. A letter is the same trigger in either case.
- The pointer buttons are `MouseLeft`, `MouseMiddle` and `MouseRight`. `Drag` starts after the pointer has moved a few
  pixels, and a press and release without movement is a `Click`.
- The directions of a gesture are `Left`, `Right`, `Up` and `Down` for a swipe, and `In` and `Out` for a pinch. A gesture
  takes no modifiers.
- A wheel trigger parses, and binding one throws, because the compositor has no wheel claim.

A bad trigger throws an error that names the part that is wrong. `Triggers.parse(trigger)` (Python, TypeScript) or `Triggers::parse` (Rust) returns the trigger as data, and
`Triggers.check(trigger)` (Python, TypeScript) or `Triggers::check` (Rust) returns its canonical text, if you want to check one yourself.

## Bind from a script

`bind(trigger, action)` on the primary input context binds a trigger on the first seat and returns the `Binding`. The
action is a function or nothing.

A class that extends one of the binding types goes to its own call:

| Binding type         | TypeScript    | Python and Rust |
| -------------------- | ------------- | --------------- |
| `KeyBinding`         | `bindBinding` | `bind_binding`  |
| `PointerDragBinding` | `bindDrag`    | `bind_drag`     |
| `GestureBinding`     | `bindGesture` | `bind_gesture`  |

::: tabs

```typescript
const binding = gnoblin.input.primary.bind("Super+x", (event) => {
    gnoblin.spawn(["foot"]);
});
binding.unbind();
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
context = gnoblin.input.primary


def open_terminal(event):
    gnoblin.spawn(["foot"])


binding = context.bind("Super+x", open_terminal)
binding.unbind()
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = Rc::new(ModelRoot::get(&objects)?);
    let input = model.input()?;
    let context = input.primary()?;
    let launcher = Rc::clone(&model);
    let binding = context.bind(
        "Super+x",
        Some(move |_event| {
            launcher.spawn(&["foot".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    binding.unbind()?;
    Ok(())
}
```

:::

- A function runs when the chord is pressed, or the button is clicked. It receives the event, which has `timestamp`. For a
  gesture trigger it runs once, when the gesture ends having gone far enough its way.
- A function that returns `false` declines the chord, and the next binding of the same trigger has it.
- A binding made with no action claims the trigger and does nothing with it: the chord is swallowed.
- A pointer drag trigger, such as `Super+MouseLeft+Drag`, needs a drag binding object, not a function: see
  [Input handlers](input-handlers).

`bind` raises a `GnoblinError` with code `already_exists` when another binding of the same context matches the same
trigger at the same priority. Its options argument is an object of these fields:

| Option      | Meaning                                             |
| ----------- | --------------------------------------------------- |
| `id`        | A name for the binding in its collection.           |
| `priority`  | A number. A higher priority wins inside one tier.   |
| `enabled`   | `false` makes the binding with the switch off.      |
| `only`      | On a device, `true` makes the device binding-only.  |
| `threshold` | For a gesture, how far it must go before it counts. |

A `Binding` has these members:

| Member      | Meaning                                                                                |
| ----------- | -------------------------------------------------------------------------------------- |
| `id`        | The binding's identity in its collection.                                              |
| `trigger`   | The trigger. Assigning a new one moves the binding and cancels a drag it runs.         |
| `enabled`   | `false` switches the binding off without removing it.                                  |
| `priority`  | A number. A higher priority wins inside one tier.                                      |
| `parsed`    | The trigger as data: `kind`, `modifiers` and the fields of its kind.                   |
| `threshold` | For a gesture, how far it must go before it counts.                                    |
| `tier`      | `context`, `layout` or `device`: where the binding sits.                               |
| `unbind()`  | Takes the binding away and releases the compositor's claim when nothing else needs it. |

## Which binding runs

Several bindings may name one trigger. A press goes to the highest-ranked one that does not decline:

1. Bindings made for one device, with `device.bind`.
2. The bindings of the layout that owns the window the press is for.
3. The context's own bindings, from its `bind` method.

Inside a tier the binding with the higher `priority` goes first. The context's `describe_bindings()` method (`describeBindings()` in TypeScript) lists every binding with its
`owner`, its `priority` and its `rank`: rank 1 is the one a press reaches first.

A reload removes the bindings the old configuration made, and the claims with them.
