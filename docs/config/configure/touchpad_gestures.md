# Touchpad gestures

Configure touchpad swipes and pinches in `gnoblin.configure`. A configured
gesture starts a command when it completes. Acting inside the desktop (switching
workspace, closing a window) is the shell's job: see
[Handle gestures in the shell](#handle-gestures-in-the-shell).

An empty or omitted `touchpad_gestures` list adds no direct actions or
commands. Mutter gesture events are still available to Lua and integrations;
see [Lua events](/events/).

## Define a gesture

Swipe paths describe the shape of a gesture relative to its start. Coordinates
are normalized from -1 to 1, with x increasing to the right and y increasing
downward. Gnoblin compares the path shape, so the same binding can match
gestures of different physical lengths.

This example starts `kgx` after a three-finger swipe that goes right and then
down:

```lua
gnoblin.configure {
    touchpad_gestures = {
        {
            name = "open-terminal",
            gesture = "swipe",
            fingers = 3,
            path = {
                {x = 0, y = 0},
                {x = 1, y = 0},
                {x = 1, y = 1},
            },
            tolerance = 0.2,
            command = {"kgx"},
        },
    },
}
```

Swipe paths start at `{x = 0, y = 0}` and contain 2–16 points. To configure a
pinch, provide `direction` instead of `path`:

```lua
gnoblin.configure {
    touchpad_gestures = {
        {
            name = "open-terminal",
            gesture = "pinch",
            fingers = 4,
            direction = "out",
            command = {"kgx"},
            threshold = 0.18,
        },
    },
}
```

Commands are argument arrays, not shell command strings. Gnoblin starts the
program after the gesture ends, using the executable name or an absolute path.
Every entry needs a `command`.

| Field       | Accepted values                                           | Default and meaning                                                                      |
| ----------- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `name`      | Unique identifier, 1–64 letters, digits, `_`, or `-`      | Required. Used in validation errors and diagnostics.                                     |
| `gesture`   | `"swipe"` or `"pinch"`                                    | Required.                                                                                |
| `fingers`   | Integer from 2 to 5                                       | Required.                                                                                |
| `path`      | Swipe: 2–16 `{x, y}` points, each coordinate from -1 to 1 | Required for swipes. Starts at `{x = 0, y = 0}` and describes movement.                  |
| `direction` | Pinch: `"in"` or `"out"`                                  | Required for pinches; do not set `path`.                                                 |
| `tolerance` | Swipe: 0.05–0.5                                           | Defaults to 0.22. Lower values require a closer match to the path.                       |
| `command`   | Nonempty array of strings                                 | Required. The program and its arguments.                                                 |
| `threshold` | Swipe: 16–240; pinch: 0.05–0.5                            | Defaults to 48 for swipe and 0.12 for pinch. Minimum movement before the path can match. |

At gesture start, Gnoblin reserves input when a binding matches the gesture
type and finger count while the session is unlocked. Bindings never reserve
gestures on the lock screen. At the end, it runs the action or command only if
the movement also matches the configured path or pinch direction and threshold.

A reserved gesture whose movement does not match has no direct action and is
not passed to another Mutter gesture handler.

## The removed `action` key

The built-in `action` names never ran: the compositor only logged them. The key
is gone, and a document that sets it is refused with a pointer here. The stock
shell carries the five as Lua in `shell/gestures.lua`:

| Old action                 | Stock shell                               |
| -------------------------- | ----------------------------------------- |
| `"workspace.next"`         | `Gestures.actions.workspace_next`         |
| `"workspace.previous"`     | `Gestures.actions.workspace_previous`     |
| `"window.close"`           | `Gestures.actions.window_close`           |
| `"window.minimize"`        | `Gestures.actions.window_minimise`        |
| `"window.toggle-maximize"` | `Gestures.actions.window_toggle_maximise` |

The stock shell binds a three-finger swipe to the workspace slide, which follows
the fingers. Bind any other gesture in a shell file:

```lua
local Gestures = require("gnoblin.shell.gestures")
seat:bind({
    Swipe4 = Gestures.on_swipe({ up = Gestures.actions.window_toggle_maximise }),
})
```

Bindings never run while the session is locked. Gestures on the lock screen are
left to the lock screen, so no action or command starts there.

## Handle gestures in the shell

`seat:bind` takes a key spelled `Swipe3` or `Pinch4`: the gesture and its finger
count (2 to 5). Gnoblin claims every such gesture, so no client and no
configured binding sees it, and the function runs for each phase of it.

The event carries:

- `phase`: `begin`, `update`, `end` or `cancel`.
- `dx` and `dy`: movement since the begin, x right and y down.
- `scale`: 1 for a swipe, above 1 for a spread.
- `rotation`: radians, clockwise.

Nothing is delivered while the session is locked. A lock that comes up
mid-gesture delivers one `cancel`.

```lua
seat:bind({
    Swipe4 = function(e)
        if e.phase == "update" then print(e.dx, e.dy) end
    end,
})
```

One holder claims one kind and finger count. A claim shadows a configured
command binding of the same kind and finger count.

The stock shell claims `Swipe3`, so give a command binding four fingers.

`Gestures.on_swipe`, `Gestures.on_pinch` and `Gestures.direction` run one
function per direction when a gesture ends. At the schema level these are
`Seat:claim_gesture(kind, fingers)` and the `gesture` signal. Holds are not
claimable.

## Status and testing

Command bindings and claims are wired from libinput gesture events through the
touchpad router. The routing, cancel, lock, no-match and finger-count rules are
covered by unit and tree-level tests that feed synthetic samples. They have not
been run against a touchpad: no hardware proof is claimed, and the stock slide's
`Gestures.travel` is not tuned on one.
