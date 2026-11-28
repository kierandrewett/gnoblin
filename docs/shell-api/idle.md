# Idle watches

`gnoblin.idle.watch(ms, listener)` runs your code when the session has had no input for `ms` milliseconds. Use it to dim a
panel, lock the screen or suspend the machine after a pause. It returns an `IdleWatch`.

::: tabs

```typescript
gnoblin.idle.watch(300_000, {
    onIdle: () => gnoblin.session.lock(),
    onResume: () => gnoblin.spawn(["notify-send", "Welcome back"]),
});
```

```python
from gnoblin.objects.model import Gnoblin, IdleListener

gnoblin = Gnoblin.get()


class LockWhenIdle(IdleListener):
    def on_idle(self, watch):
        gnoblin.session.lock()

    def on_resume(self, watch):
        gnoblin.spawn(["notify-send", "Welcome back"])


gnoblin.idle.watch(300_000, LockWhenIdle())
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{IdleListenerHandlers, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Rc::new(Gnoblin::get(&objects)?);
    let idle = model.idle()?;
    let (locker, notifier) = (Rc::clone(&model), Rc::clone(&model));
    let listener = IdleListenerHandlers {
        on_idle: Some(Box::new(move |_watch| {
            let session = locker.session()?;
            session.lock()?;
            Ok(())
        })),
        on_resume: Some(Box::new(move |_watch| {
            notifier.spawn(&["notify-send".to_owned(), "Welcome back".to_owned()], None)?;
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    idle.watch(300_000.0, Some(&listener))?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts` and the Python in `~/.config/gnoblin/shell.py`. The Rust is a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. After five minutes without input the session locks, and
the first input afterwards shows a notification.

## Watch for idleness

`ms` is a number above 0 and at most 86400000, which is a day. A configuration has at most 256 live watches, and each has
its own time. Any other value throws a `RangeError`.

A watch is idle once the session has been idle for `ms` and nothing holds it awake. A watch made while the session is quiet is idle
at once; any other becomes idle when `ms` have passed without input.

- `onIdle(watch)` runs for every idle period until you cancel the watch.
- `onResume(watch)` runs on the first input after an idle period, or when a hold appears.
- `onClosed(watch, reason)` runs once when the watch ends. `reason` is `"cancelled"`.
- A watch made while the session is already idle has `isIdle` true when `watch` returns, and runs `onIdle` once, from the next
  turn, after the call that made it has returned. A listener passed to `watch` hears it, and so does one added to the returned
  watch straight after, such as `watch.on("idle", ...)` in TypeScript.
- If the session turns active, or a hold of yours appears, before that turn, the watch is not idle by then: `onIdle` does not
  run, and `onResume` does not either, because the listeners were never told the session was idle.

An `IdleListener` is a class with optional methods, or an object that has some of them. All three methods are optional.

| Member                    | Value                                                                 |
| ------------------------- | --------------------------------------------------------------------- |
| `watch.id`                | `"idle-"` and a number.                                               |
| `watch.ms`                | The idle time you asked for.                                          |
| `watch.isIdle`            | `true` while the session is idle for `ms` and nothing holds it awake. |
| `watch.alive`             | `true` until `cancel()`.                                              |
| `watch.cancel()`          | Ends the watch. `onClosed(watch, "cancelled")` runs once.             |
| `gnoblin.idle.idleMs`     | Milliseconds since the last input at the last reading.                |
| `gnoblin.idle.inhibitors` | How many holds keep the session awake.                                |
| `gnoblin.idle.watches`    | The live watches, oldest first.                                       |

The table uses the TypeScript spellings. The other languages spell the same members their own way:

- Python uses `snake_case` and attributes: `watch.is_idle`, `watch.cancel()` and `gnoblin.idle.idle_ms`.
- Rust has a getter for each property, and every call returns a `Result`: `watch.is_idle()?`, `watch.cancel()?` and
  `idle.idle_ms()?`.

The [API reference](/reference) lists every member in each language.

The compositor publishes the time since the last input and the number of holds, and a watch is a threshold over that
reading. A hold from a Wayland idle-inhibit surface, such as a video player's, therefore delays `onIdle` the way it delays a
screen blank.

A watch ends with its configuration: a reload ends the old watches silently, and `onClosed` runs for `cancel()` only. The
connection to the compositor is opened when the first watch or hold is made and closed when the last one is gone, so a
configuration with none costs the compositor nothing.

## Hold the session awake

`gnoblin.idle.inhibit(holder)` records a hold under a name you choose and returns an `IdleInhibitor`. While one stands the
session is not idle, and every watch resumes. `inhibitor.release()` gives it back.

::: tabs

```typescript
const hold = gnoblin.idle.inhibit("presentation");
// Later:
hold.release();
```

```python
from gnoblin.objects.model import Gnoblin

hold = Gnoblin.get().idle.inhibit("presentation")
# Later:
hold.release()
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    {
        let idle = model.idle()?;
        let hold = idle.inhibit("presentation")?;
        // Later:
        hold.release()?;
    }
    Ok(())
}
```

:::

- Holding again under a name you already hold returns the same object.
- If the compositor refuses the hold, because it keeps a limited number, `inhibitor.alive` is `false` and the shell logs why.
- `inhibitor.holder` is the name, and `inhibitor.alive` is `true` until the hold is released or refused.

The shell gives every hold back when its configuration ends, which a reload and a shutdown both do. The compositor also keeps
each hold for the connection that made it:

- A shell that is killed without ending its configuration leaves no hold behind. The holds end with its connection.
- If the shell loses its connection while a script holds, those holds are already gone and `inhibitor.alive` turns `false`.
- A name belongs to your shell alone. Another program that holds under the same name makes a separate hold.
- The compositor keeps at most 256 holds across all programs, so keep your holders few and name them for what they are.
