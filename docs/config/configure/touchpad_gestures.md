# Gnoblin touchpad gestures

Define the touchpad gestures Gnoblin recognizes and the action each gesture
starts. Put `touchpad_gestures` beside `input` in `gnoblin.configure`. The
gesture stream comes from Mutter and is available to Lua and shell frontends;
GNOME Shell actions use GNOME's existing interactive gesture animations.

If you omit `touchpad_gestures`, GNOME Shell keeps its built-in touchpad
gestures. If you set it, the list replaces those bindings. The default
`init.lua.example` declares GNOME's current three-finger navigation gestures in
this list, so you can edit or remove individual entries. An empty list disables
the configured gesture bindings:

```lua
gnoblin.configure {
    touchpad_gestures = gnoblin.array {},
}
```

## Define a gesture

Each swipe is defined by an ordered path, so a binding can match a turn or
other shape instead of every swipe in the same direction.

Coordinates are relative to where the gesture starts: x increases to the right
and y increases downward. Values are normalized from -1 to 1. Gnoblin scales
the path before comparing it, so it describes shape rather than physical
distance. Use `tolerance` to control how closely the fingers must follow it.

This example runs `kgx` after a three-finger swipe that goes right and then
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
            when = "normal",
        },
    },
}
```

The path starts at `{x = 0, y = 0}` and must contain 2–16 points. Straight
gestures need only two points. To define a pinch, use `direction = "in"` or
`"out"` instead of `path`:

```lua
gnoblin.configure {
    touchpad_gestures = {
        {
            name = "open-terminal",
            gesture = "pinch",
            fingers = 4,
            direction = "out",
            command = {"kgx"},
            when = "normal",
            threshold = 0.18,
        },
    },
}
```

Commands are argument arrays, not shell command strings. Gnoblin starts the
program after the gesture ends. Use the executable name or an absolute path.

| Field       | Accepted values                                                                      | Default and meaning                                                                  |
| ----------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ |
| `name`      | Unique identifier, 1–64 letters, digits, `_`, or `-`                                 | Required. Used in warnings and frontend integrations.                                |
| `gesture`   | `"swipe"` or `"pinch"`                                                               | Required.                                                                            |
| `fingers`   | Integer from 2 to 5                                                                  | Required.                                                                            |
| `path`      | Swipe: 2–16 `{x, y}` points, each coordinate from -1 to 1                            | Required for swipes. Starts at `{x = 0, y = 0}` and must describe movement.          |
| `direction` | Pinch: `"in"` or `"out"`                                                             | Required for pinches; do not set `path`.                                             |
| `tolerance` | Swipe: 0.05–0.5                                                                      | Defaults to 0.22. Lower values require a closer match to the path.                   |
| `action`    | One of the actions below                                                             | Set this or `command`, but not both.                                                 |
| `command`   | Nonempty array of strings                                                            | Set this or `action`, but not both.                                                  |
| `when`      | `"normal"`, `"overview"`, `"app-grid"`, `"emoji-picker"`, `"unlock-screen"`, `"any"` | Defaults to `"normal"`. Restricts the GNOME Shell action to that interface state.    |
| `threshold` | Swipe: 16–240; pinch: 0.05–0.5                                                       | Defaults to 48 for swipe and 0.12 for pinch. Applies to commands and direct actions. |

The threshold controls when a gesture has moved far enough to run. GNOME Shell
progress actions use Shell's own gesture-start threshold. Config loading rejects
entries that claim the same gesture and finger count in overlapping contexts.

A direct action or command reserves its gesture from the start. If its path does
not match at the end, Gnoblin performs no action and Mutter does not pass the
gesture to GNOME Shell's built-in handlers.

## Actions

The `*.progress` actions connect a gesture to GNOME Shell's existing live
animation. The user can reverse direction before lifting their fingers.

| Action                           | Path shape           | What it controls                           |
| -------------------------------- | -------------------- | ------------------------------------------ |
| `"workspace.progress"`           | Straight, horizontal | Switch workspaces from the normal desktop. |
| `"overview.progress"`            | Straight, vertical   | Show or hide the Activities overview.      |
| `"overview-workspaces.progress"` | Straight, horizontal | Move between workspaces in the overview.   |
| `"app-grid.progress"`            | Straight, horizontal | Move between app-grid pages.               |
| `"emoji-pager.progress"`         | Straight, horizontal | Move between emoji picker pages.           |
| `"unlock-screen.progress"`       | Straight, vertical   | Move through the lock screen.              |

Progress actions require a two-point straight path because GNOME Shell's live
animation tracker commits to an axis as the gesture begins. Other actions and
commands can use multi-point paths. These actions are GNOME Shell frontend
actions. Other frontends can listen to Mutter's raw touchpad gesture events and
choose their own response.

The remaining actions run after the gesture ends:

| Action                     | Effect                                     |
| -------------------------- | ------------------------------------------ |
| `"overview.toggle"`        | Toggle the overview.                       |
| `"overview.show"`          | Show the overview.                         |
| `"overview.hide"`          | Hide the overview.                         |
| `"app-grid.show"`          | Show the app grid.                         |
| `"workspace.next"`         | Switch to the next workspace.              |
| `"workspace.previous"`     | Switch to the previous workspace.          |
| `"window.close"`           | Close the focused window.                  |
| `"window.minimize"`        | Minimize the focused window.               |
| `"window.toggle-maximize"` | Toggle maximization of the focused window. |

## Listen to raw gesture events

Mutter emits `mutter.touchpad.gesture` for each swipe, pinch, and hold phase.
The [Lua event reference](/config/lua-events) has a runnable listener and lists
the fields included with each event.
