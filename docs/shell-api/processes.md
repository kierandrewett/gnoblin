# Processes

`gnoblin.process.spawn(argv, options?)` starts a program from the shell and returns its `Process` at once. Use it to
start an application, run a helper and read its output, or hear when something ends. `gnoblin.spawn` is the same call.
`gnoblin.shell(command, options?)` runs one command line through `/bin/sh`.

```ts
import { ProcessListener } from "gnoblin-core";
import type { Process } from "gnoblin-core";

class Notify extends ProcessListener {
    onExited(process: Process) {
        // Every program the shell starts ends here, including notify-send: pick the one you mean.
        if (process.argv[0] === "date") {
            gnoblin.spawn(["notify-send", `date ended with ${process.exitCode}`]);
        }
    }
}

gnoblin.process.addListener(new Notify());
gnoblin.input.primary.bind("Super+d", () => {
    const date = gnoblin.process.spawn(["date", "+%H:%M"], { stdout: "capture" });
    date.wait().then(() => gnoblin.spawn(["notify-send", date.exit?.stdout ?? ""]));
});
```

Put the file in `~/.config/gnoblin/shell.ts`. `Super+d` runs `date`, shows the time in a notification, and the listener
reports that the program ended.

## Start a program

`spawn` takes an array: the program, found on `PATH`, and its arguments. Nothing splits or quotes it. To run one command
line, call `gnoblin.shell(command, options?)`, which runs `["/bin/sh", "-c", command]`.

The call throws a `GnoblinError` when the program cannot start. Its code is `not_found` for a program that is not there,
`busy` when 64 already run and `internal` for another refusal of the system. A bad option is a `TypeError`.

Each program starts in a process group of its own, so `kill` and `timeout` reach the programs it starts too. At most 64
run at once, and they end with the shell.

| Option        | Accepted values                             | Default   | Meaning                                          |
| ------------- | ------------------------------------------- | --------- | ------------------------------------------------ |
| `cwd`         | absolute path                               | inherited | The working directory.                           |
| `env`         | list of `NAME=value` strings, at most 128   | none      | Variables added to the session environment.      |
| `stdin`       | string, at most 1 MiB                       | empty     | Written to standard input, which is then closed. |
| `stdout`      | `inherit`, `discard`, `capture`             | `inherit` | Where standard output goes.                      |
| `stderr`      | `inherit`, `discard`, `capture`             | `inherit` | Where standard error goes.                       |
| `outputLimit` | integer from 0 to 4194304                   | `65536`   | Bytes kept from each captured stream.            |
| `timeout`     | milliseconds, more than 0, at most 86400000 | none      | SIGTERM to the group, SIGKILL 2000 ms later.     |

The stream modes:

- `inherit` leaves the stream where the shell's own is.
- `discard` throws the output away.
- `capture` keeps it, up to `outputLimit` bytes, for `wait()` to return.

## Programs the file starts

A `gnoblin.spawn` at the top level of `shell.ts`, outside any handler, starts a program for the configuration, and a reload
looks after it:

- The program starts when the new configuration takes over. A file that fails to load starts nothing, and the programs of
  the configuration that keeps running are left alone.
- A program the running configuration started with the same command and options is not started again. The new
  configuration's `Process` is the one that already runs, with its `id` and `pid`.
- A program the running configuration started that the new file does not start receives SIGTERM.
- A program a key binding or a listener started belongs to you, and a reload leaves it running.

Until the configuration has taken over, the `pid` of a `Process` made at the top level reads 0. A program that cannot start
then ends with exit code 127, so a handle never stays running for a program that does not, and the reload reports why.

## The `Process` object

| Member     | Meaning                                                                   |
| ---------- | ------------------------------------------------------------------------- |
| `id`       | The shell's number for the process. What `byId` takes.                    |
| `pid`      | The operating system's process id, 0 until a top-level spawn starts.      |
| `argv`     | The program and its arguments.                                            |
| `alive`    | `true` while it runs.                                                     |
| `status`   | `running` or `exited`.                                                    |
| `exitCode` | The exit status, or `undefined` after a signal or while it runs.          |
| `signal`   | `term`, `kill` and so on after a signal, `undefined` after a normal exit. |
| `timedOut` | `true` when `timeout` ended it.                                           |
| `exit`     | The `ProcessExit` record once it ended, `undefined` before.               |

### `wait()`

`wait()` returns the same `Operation` each time. It succeeds when the process ends, whatever its status, and then `exit` holds:

- `exitCode`, `signal` and `timedOut`, as above.
- `stdout` and `stderr`: the captured stream as a string, decoded as UTF-8 with replacement characters, or `undefined` when
  the stream was not captured.
- `stdoutTruncated` and `stderrTruncated`: `true` when the output passed `outputLimit`.

### `kill(signal?)`

`kill` sends a signal to the process group and returns an `Operation`. The signal is `term` (the default), `kill`, `int`,
`hup`, `usr1`, `usr2`, `stop` or `cont`. The operation fails with `not_found` once the process has ended.

## Find and watch processes

- `gnoblin.process.list()` returns the running processes, oldest first.
- `gnoblin.process.byId(id)` returns the process with that `id`, or `undefined`.
- `gnoblin.process.addListener(listener)` attaches a `ProcessListener` whose `onExited(process)` runs when any process
  ends. Read `process.exitCode`, which is `undefined` after a signal. `removeListener(listener)` detaches it.

A window's process is not linked to the `Process` that started it: `Client` has no `process` property. A reload ends the
old configuration's handles and callbacks with it.
