# Operations, errors and timers

A call that finishes later returns an `Operation`. A call that cannot be made at all throws. A function you hand to the
shell, whether a listener, a handler or a timer, runs under a time budget and may fail without stopping the others.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    keys: {
        "Super+Shift+q": ({ gnoblin }) => {
            gnoblin.session.logout().onComplete((value, error) => {
                if (error) {
                    gnoblin.spawn(["notify-send", `Logout refused: ${error.code}`]);
                }
            });
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+Shift+q` logs out, and shows a notification when the compositor
refuses.

## Operations

An `Operation` settles once, with a value or with a typed error. Its callbacks run as queued calls under the same time
budget as every listener: one that throws is reported, and the next callback still runs.

| Member             | Meaning                                                                                                  |
| ------------------ | -------------------------------------------------------------------------------------------------------- |
| `id`, `method`     | The operation's number and the name of the call that made it.                                            |
| `status`           | `pending`, `succeeded` or `failed`.                                                                      |
| `value`            | The result of a success. `undefined` otherwise.                                                          |
| `error`            | `{ code, message }` of a failure. `undefined` otherwise.                                                 |
| `onComplete(fn)`   | Calls `fn(value, error)` once, when the operation settles. Returns a subscription.                       |
| `map(fn)`          | A new operation with `fn(value)` as its value. A throw in `fn` fails it `internal`.                      |
| `timeout(seconds)` | A new operation that fails `timed_out` if this one has not settled in time. It does not cancel this one. |
| `cancel()`         | Fails the operation with `cancelled`. Returns `false` when it had already settled.                       |

`onComplete` returns a subscription with `unsubscribe()`, which stops the callback before it runs.

```ts
const done = gnoblin.process.spawn(["sleep", "30"]).wait().timeout(5);
done.onComplete((exit, error) => {
    if (error?.code === "timed_out") {
        gnoblin.spawn(["notify-send", "sleep took too long"]);
    }
});
```

A bad argument is not an operation failure: the call throws a `TypeError` or a `RangeError` at once.

## Error codes

| Code               | Meaning                                                                    |
| ------------------ | -------------------------------------------------------------------------- |
| `invalid_argument` | An argument is invalid.                                                    |
| `not_found`        | The object or resource does not exist, or has ended.                       |
| `unsupported`      | The compositor or the machine cannot perform the operation.                |
| `denied`           | Session policy, logind or polkit rejected the operation.                   |
| `busy`             | Something exclusive is already running.                                    |
| `cancelled`        | `cancel()` or the caller stopped the operation.                            |
| `timed_out`        | The operation reached its deadline.                                        |
| `unavailable`      | The compositor or a session service is not ready.                          |
| `internal`         | An unexpected error occurred, such as a throw inside `map`.                |
| `config_rejected`  | A reload was refused. Only `gnoblin.runtime.reloadConfig()` fails with it. |

## Timers

`gnoblin.timers.after(ms, fn)` runs `fn` once, `ms` milliseconds from now, at the first frame after that time. `ms` is from
0 to a day. It returns a `Timer`.

| Member                 | Meaning                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| `timer.pending`        | `true` until the timer ran or was cancelled.                         |
| `timer.at`             | When it runs, in milliseconds on the monotonic clock.                |
| `timer.cancel()`       | Stops the timer. Returns `false` when it had already run.            |
| `gnoblin.timers.size`  | How many timers are waiting.                                         |
| `gnoblin.timers.now()` | The monotonic clock in milliseconds. Only differences mean anything. |

- At most 1024 timers wait at once.
- A timer that throws is reported, and the next one still runs.
- A reload cancels the timers of the old configuration, and they do not delay it.

`gnoblin.after(seconds, fn)` is the same call in seconds and returns the same `Timer`.

```ts
const timer = gnoblin.timers.after(2000, () => gnoblin.show("2"));
timer.cancel();
```

## Handler errors

A listener, a key handler, an event handler or a timer that throws is reported once, with the file, line and column of the
throw in your `.ts` source. The shell keeps the handler attached and goes on to the next one.

A call that runs past its time budget is interrupted, and the shell stages your config again from the files. After several
interruptions in a short time it stops doing that until a cool-down has passed or you reload on purpose. The compositor
keeps running throughout.

## See also

- [Events](/events/) for `gnoblin.on`.
- [Compositor bridge](/bridge/connection) for the socket's own error codes.
