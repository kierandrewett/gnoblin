# Touchpad gestures

The `touchpadGestures` key configures touchpad swipes and pinches that start a command when they complete. Acting
inside the desktop (switching workspace, closing a window) is the shell's job: see
[Run a shell action from a gesture](#run-a-shell-action-from-a-gesture).

An empty or omitted `touchpadGestures` list adds no commands. Gestures of the stock config, such as the three-finger
workspace slide, are not in this list.

## Define a gesture

Swipe paths describe the shape of a gesture relative to its start. Coordinates are normalised from -1 to 1, with x
increasing to the right and y increasing downward. Gnoblin compares the path shape, so the same binding can match
gestures of different physical lengths.

This example starts `kgx` after a three-finger swipe that goes right and then down:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    touchpadGestures: [
        {
            name: "open-terminal",
            gesture: "swipe",
            fingers: 3,
            path: [
                { x: 0, y: 0 },
                { x: 1, y: 0 },
                { x: 1, y: 1 },
            ],
            tolerance: 0.2,
            command: ["kgx"],
        },
    ],
});
```

Swipe paths start at `{ x: 0, y: 0 }` and contain 2–16 points. To configure a pinch, provide `direction` instead of
`path`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    touchpadGestures: [
        {
            name: "open-terminal",
            gesture: "pinch",
            fingers: 4,
            direction: "out",
            command: ["kgx"],
            threshold: 0.18,
        },
    ],
});
```

Commands are argument lists, not shell command strings. Gnoblin starts the program after the gesture ends, using the
executable name or an absolute path. Every entry needs a `command`. The list replaces the whole list of an earlier
config, and a reload that drops the key removes it.

| Field       | Accepted values                                      | Default and meaning                                                                      |
| ----------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `name`      | Unique identifier, 1–64 letters, digits, `_`, or `-` | Required. Used in validation errors and diagnostics.                                     |
| `gesture`   | `"swipe"` or `"pinch"`                               | Required.                                                                                |
| `fingers`   | Integer from 2 to 5                                  | Required.                                                                                |
| `path`      | Swipe: 2–16 `{ x, y }` points, each from -1 to 1     | Required for swipes. Starts at `{ x: 0, y: 0 }` and describes movement.                  |
| `direction` | Pinch: `"in"` or `"out"`                             | Required for pinches; do not set `path`.                                                 |
| `tolerance` | Swipe: 0.05–0.5                                      | Defaults to 0.22. Lower values require a closer match to the path.                       |
| `command`   | Nonempty list of strings                             | Required. The program and its arguments.                                                 |
| `threshold` | Swipe: 16–240; pinch: 0.05–0.5                       | Defaults to 48 for swipe and 0.12 for pinch. Minimum movement before the path can match. |

At gesture start, Gnoblin reserves input when a binding matches the gesture type and finger count while the session is
unlocked. Bindings never reserve gestures on the lock screen. At the end, it runs the command only if the movement
also matches the configured path or pinch direction and threshold. A reserved gesture whose movement does not match
starts nothing.

## Run a shell action from a gesture

The `gestureActions` key binds a gesture, `Swipe` or `Pinch` and a finger count, to an action for each direction. The
action runs once, when the fingers lift. It is one of these named actions, or a function:

- `workspace.next` and `workspace.previous`
- `window.close`
- `window.minimise`
- `window.toggle-maximise`

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    gestureActions: {
        Swipe4: { up: "window.toggle-maximise", left: "workspace.next", right: "workspace.previous" },
        Pinch3: { in: "window.minimise" },
    },
});
```

The stock config binds a three-finger swipe to the workspace slide, which follows the fingers. A gesture of the same
trigger in `gestureActions` replaces it. A claim shadows a command in `touchpadGestures` with the same kind and finger
count, so give a command gesture four fingers. For the event a function receives and for the lower-level
`gnoblin.input.primary.bind`, see [window policy](/js-window-policy#gesture-actions).

## Status

None of this has been run against a touchpad. The routing, cancel, lock and finger-count rules are covered by tests
that feed synthetic samples, and no hardware proof is claimed.
