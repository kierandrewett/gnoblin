# Operations, errors and timers

A call that finishes later returns an `Operation`. A call that cannot be made at all throws. A function you hand to the
shell, whether a listener, a handler or a timer, runs under a time budget and may fail without stopping the others.

```ts
gnoblin.input.primary.bind("Super+Shift+q", () => {
    gnoblin.session.logout().onComplete((value, error) => {
        if (error) {
            gnoblin.spawn(["notify-send", `Logout refused: ${error.code}`]);
        }
    });
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+Shift+q` logs out, and shows a notification when the compositor
refuses.

## Operations

An `Operation` settles once, with a value or with a typed error. Its callbacks run as queued calls under the same time
budget as every listener: one that throws is reported, and the next callback still runs.

| Member           | Meaning                                                                                                               |
| ---------------- | --------------------------------------------------------------------------------------------------------------------- |
| `id`, `method`   | The operation's number and the name of the call that made it.                                                         |
| `status`         | `pending`, `succeeded` or `failed`.                                                                                   |
| `value`          | The result of a success. `undefined` otherwise.                                                                       |
| `error`          | `{ code, message }` of a failure. `undefined` otherwise.                                                              |
| `onComplete(fn)` | Calls `fn(value, error)` once, when the operation settles. Returns a subscription.                                    |
| `map(fn)`        | A new operation with `fn(value)` as its value. A throw in `fn` fails it `internal`.                                   |
| `timeout(ms)`    | A new operation that fails `timed_out` if this one has not settled in `ms` milliseconds. It does not cancel this one. |
| `cancel()`       | Fails the operation with `cancelled`. Returns `false` when it had already settled.                                    |

`onComplete` returns a subscription with `unsubscribe()`, which stops the callback before it runs.

```ts
const done = gnoblin.process.spawn(["sleep", "30"]).wait().timeout(5000);
done.onComplete((exit, error) => {
    if (error?.code === "timed_out") {
        gnoblin.spawn(["notify-send", "sleep took too long"]);
    }
});
```

A bad argument is not an operation failure: the call throws a `TypeError` or a `RangeError` at once.

## Error codes

One set of codes covers every failure of the API, and the codes are snake_case.

- An `Operation` that fails carries `{ code, message }`.
- A call that cannot be made throws a `GnoblinError` with a code from the same set.
- Its `target` names the object and member, such as `Client#7.x`, and its message begins with the target.

| Code               | Meaning                                                                    |
| ------------------ | -------------------------------------------------------------------------- |
| `invalid_argument` | An argument is invalid.                                                    |
| `not_found`        | The object or resource does not exist, or has ended.                       |
| `already_exists`   | The id, name or trigger is taken.                                          |
| `invalid_state`    | The object is not in a state that allows the call, such as a detached one. |
| `read_only`        | The property or collection cannot be written.                              |
| `unsupported`      | The compositor or the machine cannot perform the operation.                |
| `denied`           | Session policy, logind, polkit or the compositor rejected the operation.   |
| `busy`             | Something exclusive is already running or in use.                          |
| `cancelled`        | `cancel()` or the caller stopped the operation.                            |
| `timed_out`        | The operation reached its deadline.                                        |
| `unavailable`      | The compositor or a session service is not ready, or there is no seat.     |
| `internal`         | An unexpected error occurred, such as a throw inside `map`.                |
| `config_rejected`  | The configuration document refused a value, or a reload was refused.       |

A `TypeError` or a `RangeError` is the script's own mistake in a value, and it has no code. Three classes carry one so
a script can branch on `error.code`:

- `TriggerError` is a `TypeError` with the code `invalid_argument`.
- `BindingConflictError` has the code `already_exists`.
- `SceneError` is a `GnoblinError`.

```ts
try {
    gnoblin.workspaces.add(new Workspace({ id: "main" }));
} catch (error) {
    if (error instanceof GnoblinError && error.code === "already_exists") {
        gnoblin.spawn(["notify-send", "There is a workspace with that id already"]);
    }
}
```

Put the file in `~/.config/gnoblin/init.ts`. A second workspace with the id `main` shows a notification instead of
stopping the file.

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

### Time budgets

The budget counts the CPU time your code uses, not the time it takes on the clock, so a busy machine does not make a
well-behaved handler fail. The limits are:

- 1000 ms for a listener, handler or timer call.
- 5000 ms for each of the shell's own calls in a frame.
- 30000 ms for loading a config.

A call that is held up without computing is stopped when it has waited eight times its limit on the wall clock.

The message says which clock stopped the call, the limit, how much of each clock the call used and where in your code it
was:

```text
the script used 1003 ms of CPU time (1010 ms of wall time) against its 1000 ms CPU budget and was interrupted;
it was in at spin (init.ts:12:5), called from at onFocus (init.ts:20:3)
```

## See also

- [Events](/events/) for the listeners.
- [Compositor bridge](/bridge/connection) for the socket's own error codes.
