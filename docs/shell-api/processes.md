# Processes

`gnoblin.process.spawn(command, options?)` starts a program from the shell and returns its `Process` at once. Use it to
start an application, run a helper and read its output, or hear when something ends. `gnoblin.spawn` is the same call.

```ts
import { ProcessListener, defineConfig } from "gnoblin";
import type { Process } from "gnoblin";

class Notify extends ProcessListener {
    onExit(process: Process, code: number | null) {
        // Every program the shell starts ends here, including notify-send: pick the one you mean.
        if (process.argv[0] === "date") {
            gnoblin.spawn(["notify-send", `date ended with ${code}`]);
        }
    }
}

defineConfig({
    setup: (gnoblin) => gnoblin.process.addListener(new Notify()),
    keys: {
        "Super+d": ({ gnoblin }) => {
            const date = gnoblin.process.spawn(["date", "+%H:%M"], { stdout: "capture" });
            date.wait().onComplete((exit) => gnoblin.spawn(["notify-send", exit?.stdout ?? ""]));
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+d` runs `date`, shows the time in a notification, and the listener
reports that the program ended.

## Start a program

The first argument is the command:

- A string is one command line for `/bin/sh`.
- An array is the program, found on `PATH`, and its arguments. Nothing splits or quotes it.

The call throws a `GnoblinError` with the code `spawn-failed` when the program cannot start. Each program starts in a
process group of its own, so `kill` and `timeout` reach the programs it starts too. At most 64 run at once, and they end
with the shell.

| Option        | Accepted values                     | Default   | Meaning                                          |
| ------------- | ----------------------------------- | --------- | ------------------------------------------------ |
| `cwd`         | absolute path                       | inherited | The working directory.                           |
| `env`         | object of strings, at most 128      | none      | Variables added to the session environment.      |
| `stdin`       | string, at most 1 MiB               | empty     | Written to standard input, which is then closed. |
| `stdout`      | `inherit`, `discard`, `capture`     | `inherit` | Where standard output goes.                      |
| `stderr`      | `inherit`, `discard`, `capture`     | `inherit` | Where standard error goes.                       |
| `outputLimit` | integer from 0 to 4194304           | `65536`   | Bytes kept from each captured stream.            |
| `timeout`     | seconds, more than 0, at most 86400 | none      | SIGTERM to the group, SIGKILL two seconds later. |

The stream modes:

- `inherit` leaves the stream where the shell's own is.
- `discard` throws the output away.
- `capture` keeps it, up to `outputLimit` bytes, for `wait()` to return.

## The `Process` object

| Member     | Meaning                                                              |
| ---------- | -------------------------------------------------------------------- |
| `id`       | The shell's number for the process. What `byId` takes.               |
| `pid`      | The operating system's process id.                                   |
| `argv`     | The program and its arguments.                                       |
| `alive`    | `true` while it runs.                                                |
| `status`   | `running` or `exited`.                                               |
| `exitCode` | The exit status, or `null` after a signal or while it runs.          |
| `signal`   | `term`, `kill` and so on after a signal, `null` after a normal exit. |
| `timedOut` | `true` when `timeout` ended it.                                      |
| `exit`     | The `ProcessExit` record once it ended, `null` before.               |

### `wait()`

`wait()` returns the same `Operation<ProcessExit>` each time. It succeeds when the process ends, whatever its status, with:

- `exitCode`, `signal` and `timedOut`, as above.
- `stdout` and `stderr`: the captured stream as a string, decoded as UTF-8 with replacement characters, or `null` when the
  stream was not captured.
- `stdoutTruncated` and `stderrTruncated`: `true` when the output passed `outputLimit`.

### `kill(signal?)`

`kill` sends a signal to the process group and returns an `Operation`. The signal is `term` (the default), `kill`, `int`,
`hup`, `usr1`, `usr2`, `stop` or `cont`. The operation fails with `not_found` once the process has ended.

## Find and watch processes

- `gnoblin.process.list()` returns the running processes, oldest first.
- `gnoblin.process.byId(id)` returns the process with that `id`, or `null`.
- `gnoblin.process.addListener(listener)` attaches a `ProcessListener` whose `onExit(process, code)` runs when any
  process ends. `code` is the exit status, or `null` after a signal. `removeListener(listener)` detaches it.

A window's process is not linked to the `Process` that started it: `Client` has no `process` property. A reload ends the
old configuration's handles and callbacks with it.
