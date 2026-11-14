# Keyboard shortcuts

A shortcut is a binding: a trigger, such as `Super+Return`, and an action. Declare bindings in the `keys` key of
`defineConfig`, or make one from a script with `gnoblin.input.primary.bind`. The compositor withholds a claimed chord from
applications and tells the shell, so a shortcut never types into the focused window.

```ts
import { defineConfig, spawn } from "gnoblin";

defineConfig({
    keys: {
        "Super+Return": spawn("foot"),
        "Super+Shift+q": "close",
    },
    setup: (gnoblin) => {
        gnoblin.input.primary.bind("Super+x", () => {
            gnoblin.spawn(["notify-send", "x"]);
        });
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+Return` starts a terminal, `Super+Shift+q` closes the focused window and
`Super+x` shows a notification. The actions `keys` accepts, and how a config's keys combine with the stock ones, are in the
[defineConfig reference](/typescript-config#keys).

## Triggers

A trigger is modifiers joined to one key or button with `+`. Two spellings of one trigger are the same trigger: its
`parseTrigger(trigger).text` is the canonical spelling.

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

A bad trigger throws a `TriggerError` that names the part that is wrong. `parseTrigger` is exported by `gnoblin` if you want
to check one yourself.

## Bind from a script

`gnoblin.input.primary.bind(trigger, action)` binds a trigger on the first seat and returns the `Binding`. The action is a
function, a `Binding` object, or `false`.

```ts
const binding = gnoblin.input.primary.bind("Super+x", (event) => {
    gnoblin.spawn("foot");
});
binding.unbind();
```

- A function runs when the chord is pressed, or the button is clicked. It receives the event, which has `timestamp`. For a
  gesture trigger it runs once, when the gesture ends having gone far enough its way.
- A function that returns `false` declines the chord, and the next binding of the same trigger has it.
- An action of `false` claims the trigger and does nothing with it: the chord is swallowed.
- A pointer drag trigger, such as `Super+MouseLeft+Drag`, needs a binding object, not a function: see
  [Input handlers](input-handlers).

`bind` throws a `BindingConflictError` when another binding of the same context matches the same trigger at the same
priority.

A `Binding` has these members:

| Member     | Meaning                                                                                |
| ---------- | -------------------------------------------------------------------------------------- |
| `id`       | The binding's identity in its collection.                                              |
| `trigger`  | The trigger. Assigning a new one moves the binding and cancels a drag it runs.         |
| `enabled`  | `false` switches the binding off without removing it.                                  |
| `priority` | A number. A higher priority wins inside one tier.                                      |
| `parsed`   | The trigger as data: `kind`, `modifiers` and the fields of its kind.                   |
| `unbind()` | Takes the binding away and releases the compositor's claim when nothing else needs it. |

## Which binding runs

Several bindings may name one trigger. A press goes to the highest-ranked one that does not decline:

1. Bindings made for one device, with `device.bind`.
2. The bindings of the layout that owns the window the press is for.
3. The context's own bindings, from `keys` and `gnoblin.input.primary.bind`.

Inside a tier the binding with the higher `priority` goes first. `context.describeBindings()` lists every binding with its
`owner`, its `priority` and its `rank`: rank 1 is the one a press reaches first.

A reload removes the bindings the old configuration made, and the claims with them.
