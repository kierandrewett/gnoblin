# Operations, errors and timers

A call that finishes later returns an `Operation`. A call that cannot be made at all throws. A function you hand to the
shell, whether a listener, a handler or a timer, runs under a time budget and may fail without stopping the others.

```ts
gnoblin.input.primary.bind("Super+Shift+q", () => {
    gnoblin.session.logout().catch((error, code) => {
        gnoblin.spawn(["notify-send", `Logout refused: ${code}: ${error}`]);
    });
});
```

Put the file in `~/.config/gnoblin/shell.ts`. `Super+Shift+q` logs out, and shows a notification when the compositor
refuses.

## Operations

An `Operation` settles once, with success or with a typed error. Its callbacks run as queued calls under the same time
budget as every listener: one that throws is reported, and the next callback still runs.

| Member              | Meaning                                                                       |
| ------------------- | ----------------------------------------------------------------------------- |
| `status`            | `pending`, `succeeded` or `failed`.                                           |
| `error`             | The words of a failure. Empty otherwise.                                      |
| `errorCode`         | The code of a failure, from the table below. Empty otherwise.                 |
| `then(fn)`          | Calls `fn()` once, when the operation succeeds.                               |
| `catch(fn)`         | Calls `fn(error, code)` once, when the operation fails.                       |
| `on("settled", fn)` | Calls `fn(operation)` once, when it settles either way. Returns a `Listener`. |

A call that has a result, such as `Process.wait()`, leaves it on the object that made the operation: read `process.exit`
inside `then`. To stop waiting for an operation after a time, start a timer:

```ts
const sleep = gnoblin.process.spawn(["sleep", "30"]);
sleep.wait().then(() => gnoblin.spawn(["notify-send", "sleep ended"]));
gnoblin.timers.after(5000, () => {
    if (sleep.alive) {
        gnoblin.spawn(["notify-send", "sleep took too long"]);
        sleep.kill();
    }
});
```

A bad argument is not an operation failure: the call throws a `TypeError` or a `RangeError` at once.

## Error codes

One set of codes covers every failure of the API, and the codes are snake_case.

- An `Operation` that fails carries an `error` and an `errorCode`.
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

A `TypeError` or a `RangeError` is the script's own mistake in a value, and it has no code. A call the library refuses is a
`GnoblinError`, which `gnoblin-core` exports, so a script can branch on `error.code`:

```ts
import { GnoblinError } from "gnoblin-core";
import { Workspace } from "gnoblin-std";

try {
    gnoblin.workspaces.add(new Workspace("main"));
} catch (error) {
    if (error instanceof GnoblinError && error.code === "already_exists") {
        gnoblin.spawn(["notify-send", "There is a workspace with that id already"]);
    }
}
```

Put the file in `~/.config/gnoblin/shell.ts`. A second workspace with the id `main` shows a notification instead of
stopping the file.

### Which failure carries which code

A call on an object of the library, such as `Client`, `InputContext` or a std widget, fails with the code that says why. The shell
sets it where it refuses the call.

| Failure                                                                 | Code                            |
| ----------------------------------------------------------------------- | ------------------------------- |
| A window or object that has closed, or a name the object does not have  | `not_found`                     |
| A second object with an id, name or trigger that is taken               | `already_exists`                |
| An object not added yet, or a transition that is not being staged       | `invalid_state`                 |
| A property or a collection that cannot be written                       | `read_only`                     |
| A member the library names and does not implement, or cannot animate    | `unsupported`                   |
| A request the compositor or the host refuses with no more specific word | `denied`                        |
| A single holder, or an object in use by a call that is running          | `busy`                          |
| No seat, or an owner that does not answer                               | `unavailable`                   |
| A fault of the shell itself, or a panic it caught                       | `internal`                      |
| A generation a reload has replaced                                      | `invalid_state`                 |
| A value of the wrong kind or out of range                               | none: `TypeError`, `RangeError` |

An object that holds state of its own is in use while a call about it runs. What a callback of that call may do depends on the
call:

- A call that changes the state, such as setting `Workspace.layout`: the callback cannot call the object back. It gets `busy`, and
  the call goes on.
- A call that only reads the state, or a property read: the callback may read the object's properties and call its other members
  that only read it.

For example, a layout's `findSplit` or `setSplit` may read the `Workspace` that runs it, and an entry's `run` may read the
`menu.client` of the `WindowMenu` that runs it.

The members the std library adds to `gnoblin` hold no state, so they are never in use. The callback of `gnoblin.requestTransition` may
read `gnoblin.workspaces` or `gnoblin.displays`.

`cancelled` and `config_rejected` come from `Operation` and the configuration, and `timed_out` from logind. A call on an object
does not raise them.

The `target` is `Class#id.member`, such as `Client#7.close`, where `id` is the object's `id` property. A class function, a
constructor or an object with no readable `id` gives `Class.member`, such as `InputContext.new`. The message begins with the
target.

### Member names in a refusal

A refusal names the member it refused, in the spelling of the language that made the call. The library words it with the schema's
name (`compositor_settings.user_shaders`), and each generated SDK writes the member as its own class spells it:

- TypeScript: `` `CompositorSettings.userShaders` must be a boolean, got text ``, with the camelCase member and the parameter or the
  record field the refusal names (`` `DwindleLayout.splitRatio`: `amount` ``).
- Python: `` `CompositorSettings.user_shaders` must be a boolean, got text ``, with the attribute the Python class has, so a member
  that is a keyword reads `from_`.
- Rust: the schema's name stays, as `Fail` carries it, and the compositor's log words it the same way.

The target of a failure (`Keyboard#1.repeatRate` in TypeScript, `Keyboard#1.repeat_rate` in Python and Rust) and the words after it
follow the same spelling. A name the generated class does not know is left as the library wrote it.

### A shell that read the old failure

Five things about a failed call of an object of the library read differently from before the codes. A shell written for the old reading changes as follows.

- `error.code` is the word, such as `"not_found"`. It was the ABI's integer, such as `-6`, on a plain `Error`. Compare `error.code` with a word, and with `instanceof GnoblinError` for a failure that has one.
- The message does not start with `E_TYPE: ` or another ABI symbol, as it did. A `TypeError` says what was wrong in the words of the library. A `GnoblinError` begins with its `target`.
- A write to a property or a collection that cannot be written is a `GnoblinError` with the code `read_only`, in TypeScript and in Python. It was a `TypeError`. A bad argument is still a `TypeError`.
- A timed write to a property that cannot animate is a `GnoblinError` with the code `unsupported`. It was a `TypeError`. A generated class has a timed setter only for a property that can animate, so only a direct call of the native module meets this.
- `Display.viewport` on a display that already shows a viewport is a `GnoblinError` with the code `busy`. It was a `TypeError`.

The ABI integer is not part of the script API. A shell that needs it reads `abi` on the `Error` a call of the native module throws, which a generated class never lets out.

## Timers

`gnoblin.timers.after(ms, fn)` runs `fn` once, `ms` milliseconds from now, at the first frame after that time. `ms` is from
0 to a day. It returns a `Timer`.

| Member                 | Meaning                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| `timer.pending`        | `true` until the timer ran or was cancelled.                         |
| `timer.cancel()`       | Stops the timer. Nothing happens when it had already run.            |
| `gnoblin.timers.now()` | The monotonic clock in milliseconds. Only differences mean anything. |

- A timer that throws is reported, and the next one still runs.
- A reload cancels the timers of the old configuration, and they do not delay it.

```ts
const timer = gnoblin.timers.after(2000, () => gnoblin.workspaces.showById("2"));
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
it was in at spin (shell.ts:12:5), called from at onFocus (shell.ts:20:3)
```

## See also

- [Events](/events/) for the listeners.
- [Compositor bridge](/bridge/connection) for the socket's own error codes.
