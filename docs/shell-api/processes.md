# Processes

`gnoblin.process.spawn(argv, options?)` starts a program from an event handler or an operation callback and returns a
`Process`. Use it to start an application, run a helper and read its output, or ask a prompt program for an answer.

```lua
gnoblin.on("gnoblin.shortcut.activated", function(event)
    local date = gnoblin.process.spawn({"date", "+%H:%M"}, {stdout = "capture"})
    date:wait():on_complete(function(exit, err)
        if err then
            print("could not start date:", err.message)
        elseif exit.exit_code == 0 then
            print("time:", exit.stdout)
        end
    end)
end)
```

`argv` holds 1 to 64 strings, at most 64 KiB in total. The first string is the program, found on `PATH`. The call
raises an error while the config file loads, and the process cannot start while the session is locked.

## Options

| Option         | Accepted values                       | Default     | Meaning                                                  |
| -------------- | ------------------------------------- | ----------- | -------------------------------------------------------- |
| `cwd`          | absolute path                         | inherited   | Working directory.                                       |
| `env`          | table of `NAME = "value"` strings     | none        | Variables added to the session environment, at most 128. |
| `stdin`        | string, at most 1 MiB                 | empty       | Bytes written to standard input, which is then closed.   |
| `stdout`       | `"inherit"`, `"discard"`, `"capture"` | `"inherit"` | Where standard output goes.                              |
| `stderr`       | `"inherit"`, `"discard"`, `"capture"` | `"inherit"` | Where standard error goes.                               |
| `output_limit` | integer from 0 to 4194304             | `65536`     | Bytes kept from each captured stream.                    |
| `timeout`      | seconds, more than 0 up to 86400      | none        | Time before Gnoblin ends the process.                    |

For `stdout` and `stderr`:

- `"inherit"` writes to Gnoblin's own output, which the session log records.
- `"discard"` drops the output.
- `"capture"` keeps the output and returns it from `wait()` as raw bytes, which need not be UTF-8. Output past
  `output_limit` is read and dropped, so the program never blocks on a full pipe, and the matching `_truncated` field
  is `true`.

When `timeout` passes, Gnoblin sends SIGTERM to the process group and SIGKILL two seconds later.

## The Process object

| Field       | Value                                                                         |
| ----------- | ----------------------------------------------------------------------------- |
| `id`        | Identifier for this process in the session.                                   |
| `alive`     | True from spawn until the process has exited or failed to start.              |
| `argv`      | The command line it was started with.                                         |
| `pid`       | The process ID once it has started, otherwise `nil`.                          |
| `status`    | `"starting"`, `"running"`, `"exited"`, or `"failed"` when it could not start. |
| `exit_code` | The exit status after a normal exit.                                          |
| `signal`    | The signal that ended it, such as `"term"` or `"kill"`, after it was killed.  |

The fields are read-only and change as the process runs.

`process:on("closed", fn)` calls `fn(process)` once the process has ended, however
it ended: it exited, was killed, or could not be started.

- The end signal is called `closed`, as on `Window`, `Workspace` and `Output`, so
  code can treat every object alike.
- The fields above say how it ended, and `wait()` has the captured output.
- `on` fails once the process has ended, and listening ends after `closed`.

```lua
local job = gnoblin.process.spawn({"sleep", "1"})
job:on("closed", function(p) print(p.status, p.exit_code) end)
```

`process:wait()` returns an Operation that succeeds when the process ends, whatever its exit status. Every call returns
the same Operation. Its value has these fields:

| Field              | Value                                                               |
| ------------------ | ------------------------------------------------------------------- |
| `exit_code`        | The exit status, after a normal exit.                               |
| `signal`           | The signal that ended the process, after it was killed.             |
| `timed_out`        | `true` when the `timeout` option ended the process.                 |
| `stdout`, `stderr` | The captured bytes, for a stream set to `"capture"`.                |
| `stdout_truncated` | `true` when standard output passed `output_limit`; also for stderr. |

The Operation fails with `not_found` when the program does not exist, and with another error code when it could not
start for another reason.

`process:kill(signal?)` sends a signal to the process group and returns an Operation. It fails with `not_found` once the
process has exited. Ending a process is allowed while the session is locked.

| Signal             | Effect                                             |
| ------------------ | -------------------------------------------------- |
| `"term"` (default) | Asks the program to exit.                          |
| `"kill"`           | Ends the program at once; it cannot clean up.      |
| `"int"`, `"hup"`   | Interrupt and hang-up, which some programs handle. |
| `"usr1"`, `"usr2"` | Program-defined signals.                           |
| `"stop"`, `"cont"` | Pause the process group, and resume it.            |

## Process groups and reloads

Each process starts in its own session and process group, so `kill` and `timeout` also reach the programs it starts.
Gnoblin resets an inherited negative CPU nice value to `0` and keeps a positive one. At most 64 processes started this
way run at once.

A process keeps running when the config reloads. The `Process` object belongs to the runtime that started it, so the
reloaded config cannot wait for it or kill it. When Gnoblin itself stops, it sends SIGTERM to every process that is
still running.

## See also

- [Operations, errors and timers](/shell-api/errors)
- [Passphrase prompts](/shell-api/passphrase-prompts)
