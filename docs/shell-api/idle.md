# Idle watches

`gnoblin.idle.watch(ms, handlers)` runs your code when the session has had no input for `ms` milliseconds. Use it to dim a
panel, lock the screen or suspend the machine after a pause. It returns an `IdleWatch`.

```ts
import { defineConfig, toast } from "gnoblin";

defineConfig({
    setup: (gnoblin) => {
        gnoblin.idle.watch(300_000, {
            onIdle: () => gnoblin.session.lock(),
            onResume: () => toast("Welcome back"),
        });
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. After five minutes without input the session locks, and the first input
afterwards shows a message.

## Watch for idleness

`ms` is a number above 0 and at most 86400000, which is a day. A configuration has at most 256 live watches, and each has
its own time. Any other value throws a `RangeError`.

A watch is not idle when it starts. It becomes idle once the session has been idle for `ms` and nothing holds it awake.

- `onIdle(watch)` runs for every idle period until you cancel the watch.
- `onResume(watch)` runs on the first input after an idle period, or when a hold appears.
- `onClosed(watch, reason)` runs once when the watch ends. `reason` is `"cancelled"`.
- A watch made while the session is already idle runs `onIdle` at once.

All three handlers are optional.

| Member                    | Value                                                                 |
| ------------------------- | --------------------------------------------------------------------- |
| `watch.id`                | `"idle-"` and a number.                                               |
| `watch.ms`                | The idle time you asked for.                                          |
| `watch.idle`              | `true` while the session is idle for `ms` and nothing holds it awake. |
| `watch.alive`             | `true` until `cancel()`.                                              |
| `watch.cancel()`          | Ends the watch. `onClosed(watch, "cancelled")` runs once.             |
| `gnoblin.idle.idleMs`     | Milliseconds since the last input at the last reading, or `null`.     |
| `gnoblin.idle.inhibitors` | How many holds keep the session awake, or `null`.                     |
| `gnoblin.idle.watches`    | The live watches, oldest first.                                       |

The compositor publishes the time since the last input and the number of holds, and a watch is a threshold over that
reading. A hold from a Wayland idle-inhibit surface, such as a video player's, therefore delays `onIdle` the way it delays a
screen blank.

A watch ends with its configuration: a reload ends the old watches silently, and `onClosed` runs for `cancel()` only. The
connection to the compositor is opened when the first watch or hold is made and closed when the last one is gone, so a
configuration with none costs the compositor nothing.

## Hold the session awake

`gnoblin.idle.inhibit(holder)` records a hold under a name you choose and returns an `IdleInhibitor`. While one stands the
session is not idle, and every watch resumes. `inhibitor.release()` gives it back.

```ts
const hold = gnoblin.idle.inhibit("presentation");
// Later:
hold.release();
```

- Holding again under a name you already hold returns the same object.
- If the compositor refuses the hold, because it keeps a limited number, `inhibitor.alive` is `false` and the shell logs why.
- `inhibitor.holder` is the name, and `inhibitor.alive` is `true` until the hold is released or refused.

The shell gives every hold back when its configuration ends, which a reload and a shutdown both do. The compositor also keeps
each hold for the connection that made it:

- A shell that is killed without ending its configuration leaves no hold behind. The holds end with its connection.
- If the shell loses its connection while a script holds, those holds are already gone and `inhibitor.alive` turns `false`.
- A name belongs to your shell alone. Another program that holds under the same name makes a separate hold.
- The compositor keeps at most 256 holds across all programs, so keep your holders few and name them for what they are.
