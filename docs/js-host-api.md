# JavaScript session, process and timer API

`gnoblin` has more than windows. A script can ask the session to log out or suspend, start a program and hear when it
ends, run a function later, listen for the compositor's signals, search for objects and read the build identity.

Each of these is TypeScript over the shell's connection to the compositor, with the name and behavior of its Lua
counterpart in [Session and runtime health](/shell-api/session-runtime), [Processes](/shell-api/processes) and
[Operations, errors and timers](/shell-api/errors). The differences are listed at the end.

```ts
import { ProcessListener, defineConfig } from "gnoblin";

class Notify extends ProcessListener {
    onExit(process, code) {
        console.log(process.argv.join(" "), "ended with", code);
    }
}

defineConfig({
    setup: (gnoblin) => {
        gnoblin.process.addListener(new Notify());
        gnoblin.process.spawn(["date", "+%H:%M"], { stdout: "capture" });
        gnoblin.timers.after(5000, () => gnoblin.session.lock());
    },
});
```

## Operations and errors

A call that finishes later returns an `Operation`. It settles once, with a value or with a typed error, and its
callbacks run as queued calls under the same time budget as every listener: one that throws is reported, and the next
callback still runs. A call that cannot be made at all throws instead: a bad argument is a `TypeError`.

| Member             | What it does                                                                  |
| ------------------ | ----------------------------------------------------------------------------- |
| `status`           | `pending`, `succeeded` or `failed`.                                           |
| `value`            | The result of a success; `undefined` otherwise.                               |
| `error`            | `{ code, message }` of a failure; `undefined` otherwise.                      |
| `onComplete(fn)`   | Calls `fn(value, error)` once. Returns a subscription with `unsubscribe()`.   |
| `map(fn)`          | A new operation with `fn(value)`; a throw in `fn` fails it `internal`.        |
| `timeout(seconds)` | A new operation that fails `timed_out`; it does not cancel this one.          |
| `cancel()`         | Fails the operation `cancelled`. Returns `false` when it had already settled. |

The error codes are `invalid_argument`, `not_found`, `unsupported`, `denied`, `busy`, `cancelled`, `timed_out`,
`unavailable`, `internal` and, for a refused reload, `config_rejected`.

## Session

`gnoblin.session` talks to the compositor over a connection the shell holds for it. With no connection the status is
`unavailable` and every verb fails `unavailable`.

| Member        | Returns                  | Meaning                                                               |
| ------------- | ------------------------ | --------------------------------------------------------------------- |
| `status()`    | `SessionStatus`          | The session as the compositor last published it. Read on each call.   |
| `logout()`    | `Operation<Accepted>`    | Ends the session once the compositor confirms.                        |
| `poweroff()`  | `Operation<Accepted>`    | Powers the machine off once the init system accepts. Does not prompt. |
| `reboot()`    | `Operation<Accepted>`    | Reboots, with the contract of `poweroff()`.                           |
| `suspend()`   | `Operation<Accepted>`    | Suspends to RAM. The session locks first.                             |
| `hibernate()` | `Operation<Accepted>`    | Hibernates; fails `unsupported` when the machine cannot.              |
| `lock()`      | `Operation<LockRequest>` | Asks the compositor to lock. Read `status()` to see the locked state. |

`status()` has `state` (`running`, or `unavailable` with no connection), `lockAvailable`, `revision` (how many lock
changes the session published) and, only while `lockAvailable` is true, `lockState`: `unlocked`, `covering`, `locked` or
`failsafe`. Unavailable does not mean unlocked.

A refused power verb fails with one of these codes, and the message names the D-Bus error:

- `denied`: logind or polkit refused.
- `unsupported`: the machine cannot do it, or logind has no such action.
- `busy`: logind is already running another power action.
- `unavailable`: logind is not on the system bus.
- `timed_out`: logind did not answer in time.

## Reload

`gnoblin.runtime.reloadConfig()` asks for the reload `gnoblinctl config reload` asks for. It returns an
`Operation<ReloadResult>` with `ok`, `action`, `settingsRevision`, `warnings` and `notApplied`. It throws while the
configuration loads, because every reload would ask for another.

When the document is refused the operation fails `config_rejected` with the reason, and the running configuration, the
one that asked, stays to hear it. When the reload succeeds the new configuration replaces the one that asked, so the
asking generation's callbacks do not run.

## Processes

`gnoblin.process.spawn(command, options?)` starts a program and returns its `Process` at once. `gnoblin.spawn` is the
same call.

- A string is one command line for `/bin/sh`.
- An array is the program, found on `PATH`, and its arguments.
- The call throws a `GnoblinError` with the code `spawn-failed` when the program cannot start.

Each program starts in a process group of its own, so `kill` and `timeout` reach the programs it starts too. At most 64
run at once, and they end with the shell.

| Option        | Accepted values                     | Default   | Meaning                                          |
| ------------- | ----------------------------------- | --------- | ------------------------------------------------ |
| `cwd`         | absolute path                       | inherited | Working directory.                               |
| `env`         | object of strings, at most 128      | none      | Variables added to the session environment.      |
| `stdin`       | string, at most 1 MiB               | empty     | Written to standard input, which is then closed. |
| `stdout`      | `inherit`, `discard`, `capture`     | `inherit` | Where standard output goes.                      |
| `stderr`      | `inherit`, `discard`, `capture`     | `inherit` | Where standard error goes.                       |
| `outputLimit` | integer from 0 to 4194304           | `65536`   | Bytes kept from each captured stream.            |
| `timeout`     | seconds, more than 0, at most 86400 | none      | SIGTERM to the group, SIGKILL two seconds later. |

A `Process` has `id`, `pid`, `argv`, `alive`, `status` (`running` or `exited`), `exitCode` (`null` after a signal),
`signal` (`term`, `kill` and so on, `null` after a normal exit) and `timedOut`. Its methods:

- `wait()` returns the same `Operation<ProcessExit>` each time. It succeeds when the process ends, whatever its status,
  with `exitCode`, `signal`, `timedOut`, `stdout`, `stderr` and the two `Truncated` flags. A captured stream is decoded as
  UTF-8 with replacement characters.
- `kill(signal?)` sends `term`, `kill`, `int`, `hup`, `usr1`, `usr2`, `stop` or `cont` to the group. The operation fails
  `not_found` once the process has ended.

`gnoblin.process.list()` returns the running processes oldest first, `byId(id)` finds one again or returns `null`, and
`addListener(listener)` attaches a `ProcessListener` whose `onExit(process, code)` runs when any process ends, with
`code` the exit status or `null` after a signal.

## Timers

`gnoblin.timers.after(ms, fn)` runs `fn` once, `ms` milliseconds from now (0 to one day), at the first frame past the
time. It returns a `Timer` with `pending`, `at` and `cancel()`.

- At most 1024 timers wait at once.
- A timer that throws is reported, and the next one still runs.
- A reload cancels the timers of the old configuration.

`gnoblin.timers.now()` is a monotonic clock in milliseconds; only the difference between two readings means anything.
`gnoblin.after(seconds, fn)` is the older spelling in seconds and returns the same `Timer`.

## Events

`gnoblin.on(name, fn)` calls `fn` for every signal of the compositor's schema, by its `namespace.object.signal` name.
Hyphens are accepted for underscores, so `gnoblin.session.reload-requested` names `gnoblin.session.reload_requested`.
`once` runs for the first signal only. `off(subscription)` or `off(name, fn)` detaches a handler.

The handler receives `{ name, object, ...fields }`: the signal's name, the identity of the object that raised it, and
its fields in camelCase. The generated `gnoblin.d.ts` types each event by name. An unknown name throws and lists the
near matches. At most 1024 handlers attach at once, and a reload detaches them.

```ts
gnoblin.on("gnoblin.session.bell", (event) => console.log("bell", event.count));
```

## Find, focus history and version

`gnoblin.find(kind, filter?)` returns the live objects of one kind in list order. The kinds are `Window`, `Workspace`
and, where the shell has outputs, `Output`.

- A table filter keeps the objects whose fields equal every value. `app_id` and `appId` are one field.
- A function filter keeps the objects it returns a truthy value for.
- An unknown kind or field throws and names what works.

`gnoblin.focus.history(filter?)` returns the live windows in the order they were focused, the latest first. Windows never
seen focused follow in list order, and minimised windows are included.

The filter has three keys:

- `workspaceId` keeps the windows of one workspace.
- `monitorId` keeps the windows of one output.
- `limit` caps the count: 1 to 256, 50 by default.

`gnoblin.version()` is a frozen record with `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha` and
`buildId`. A field the build does not know is `"unknown"`, and a remote loses its credentials.

## Differences from Lua

- `timers.after` takes milliseconds and a function, and returns a `Timer`. In Lua it takes seconds and returns an
  operation.
- Names are camelCase: `byId`, `exitCode`, `reloadConfig`, `lockState`.
- A process's end reaches `ProcessListener.onExit` and `wait()`. There is no `changed` or `watch`.
- A captured stream is a string, not raw bytes.
- A reload ends the old generation's handles and callbacks with it, as it ends its timers and event handlers.
- `gnoblin.session.activity`, `gnoblin.capabilities`, `gnoblin.focus.policy` and `gnoblin.runtime.status` are not ported.
