# Touchpad gestures

Configure touchpad swipes and pinches in `gnoblin.configure`. The standalone
compositor can run a built-in window or workspace action, or start a command
when a gesture completes. The packaged `init.lua.example` includes two
three-finger workspace swipe bindings.

An empty or omitted `touchpad_gestures` list adds no direct actions or
commands. Mutter gesture events are still available to Lua and integrations;
see [Lua events](/config/lua-events).

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
            when = "unlocked",
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
            when = "unlocked",
            threshold = 0.18,
        },
    },
}
```

Commands are argument arrays, not shell command strings. Gnoblin starts the
program after the gesture ends, using the executable name or an absolute path.
Set exactly one of `action` or `command` for each entry.

| Field       | Accepted values                                           | Default and meaning                                                                                                           |
| ----------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `name`      | Unique identifier, 1–64 letters, digits, `_`, or `-`      | Required. Used in validation errors and diagnostics.                                                                          |
| `gesture`   | `"swipe"` or `"pinch"`                                    | Required.                                                                                                                     |
| `fingers`   | Integer from 2 to 5                                       | Required.                                                                                                                     |
| `path`      | Swipe: 2–16 `{x, y}` points, each coordinate from -1 to 1 | Required for swipes. Starts at `{x = 0, y = 0}` and describes movement.                                                       |
| `direction` | Pinch: `"in"` or `"out"`                                  | Required for pinches; do not set `path`.                                                                                      |
| `tolerance` | Swipe: 0.05–0.5                                           | Defaults to 0.22. Lower values require a closer match to the path.                                                            |
| `action`    | One of the built-in actions below                         | Set this or `command`, but not both.                                                                                          |
| `command`   | Nonempty array of strings                                 | Set this or `action`, but not both.                                                                                           |
| `when`      | `"unlocked"`, `"locked"`, or `"any"`                      | Defaults to `"unlocked"`. `unlocked` applies while the session is unlocked; `locked` applies while the session is locked; `any` applies in either state. `"normal"` and `"unlock-screen"` still load as the old names for `"unlocked"` and `"locked"`. |
| `threshold` | Swipe: 16–240; pinch: 0.05–0.5                            | Defaults to 48 for swipe and 0.12 for pinch. Minimum movement before the path can match.                                      |

At gesture start, Gnoblin reserves input when a binding matches the gesture
type, finger count, and active lock context. At the end, it runs the action or
command only if the movement also matches the configured path or pinch
direction and threshold.

A reserved gesture whose movement does not match has no direct action and is
not passed to another Mutter gesture handler.

## Built-in actions

These actions run after a matching gesture ends:

| Action                     | Effect                                     |
| -------------------------- | ------------------------------------------ |
| `"workspace.next"`         | Switch to the next workspace.              |
| `"workspace.previous"`     | Switch to the previous workspace.          |
| `"window.close"`           | Close the focused window.                  |
| `"window.minimize"`        | Minimize the focused window.               |
| `"window.toggle-maximize"` | Toggle maximization of the focused window. |

Workspace and window actions do nothing while the session is locked. Commands
can be configured for either lock context; use care when starting programs
while the screen is locked.

## Handle gestures in a shell

Shells can subscribe to the raw `mutter.touchpad.gesture` signal or the stable
`gnoblin.input.gesture` event and decide how to respond. These events report
gesture phases and movement; they do not provide GNOME Shell overview,
emoji-picker, or lock-screen progress animations. Shells that need animated UI
can implement that behavior themselves. See [Lua events](/config/lua-events)
for event payloads and a listener example.
