# Processes

The process service starts a program from the shell and returns its `Process` at once. Use it to start an application, run
a helper and read its output, or hear when something ends.

Its `spawn` takes the program and its arguments:

- TypeScript: `gnoblin.process.spawn(argv, options?)`
- Python: `gnoblin.process.spawn(argv, options)`
- Rust: `processes.spawn(&argv, options)`, where `processes` is the `Processes` service from `Gnoblin::process()`.

The root object's `spawn` (`gnoblin.spawn` in TypeScript and Python, `model.spawn` in Rust) is the same call. `shell` runs one
command line through `/bin/sh`, with the same options.

::: tabs

```typescript
import { gnoblin, ProcessListener } from "gnoblin-core";
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

```python
from gnoblin import BindingEvent, Gnoblin, Process, ProcessListener, SpawnOptions, StreamMode

gnoblin = Gnoblin.get()


class Notify(ProcessListener):
    def on_exited(self, process: Process) -> None:
        # Every program the shell starts ends here, including notify-send: pick the one you mean.
        if process.argv[0] == "date":
            gnoblin.spawn(["notify-send", f"date ended with {process.exit_code}"])


def show_time(event: BindingEvent) -> None:
    date = gnoblin.process.spawn(["date", "+%H:%M"], SpawnOptions(stdout=StreamMode.CAPTURE))
    def notify_date() -> None:
        finished = date.exit
        gnoblin.spawn(["notify-send", (finished.stdout if finished else None) or ""])

    date.wait().then(notify_date)


gnoblin.process.add_listener(Notify())
gnoblin.input.primary.bind("Super+d", show_time)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{BindingEvent, Gnoblin, Process, ProcessListenerHandlers, SpawnOptions, StreamMode};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Rc::new(Gnoblin::get(&objects)?);
    let processes = model.process()?;
    let notifier = Rc::clone(&model);
    let listener = ProcessListenerHandlers {
        on_exited: Some(Box::new(move |process: Process| {
            if process.argv()?.first().map(String::as_str) == Some("date") {
                let code = process.exit()?.and_then(|exit| exit.exit_code);
                notifier.spawn(&["notify-send".to_owned(), format!("date ended with {code:?}")], None)?;
            }
            Ok(())
        })),
    }
    .implement(&objects)?;
    processes.add_listener(&listener)?;

    let input = model.input()?;
    let primary = input.primary()?;
    let spawner = Rc::clone(&model);
    primary.bind(
        "Super+d",
        Some(move |_event: BindingEvent| {
            let date = spawner.spawn(
                &["date".to_owned(), "+%H:%M".to_owned()],
                Some(&SpawnOptions {
                    cwd: None,
                    env: None,
                    stdin: None,
                    stdout: Some(StreamMode::Capture),
                    stderr: None,
                    output_limit: None,
                    timeout: None,
                }),
            )?;
            let inner = Rc::clone(&spawner);
            date.wait()?.then(move || {
                let output = date.exit()?.and_then(|exit| exit.stdout).unwrap_or_default();
                inner.spawn(&["notify-send".to_owned(), output], None)?;
                Ok(())
            })?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts` and the Python in `~/.config/gnoblin/shell.py`. The Rust is a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+d` runs `date`, shows the time in a
notification, and the listener reports that the program ended.

## Start a program

`spawn` takes a list: the program, found on `PATH`, and its arguments. Nothing splits or quotes it. To run one command
line, use `shell(command, options?)`, which runs `["/bin/sh", "-c", command]`. In Python and Rust the same two calls
exist as `spawn` and `shell` on the process service.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

gnoblin.input.primary.bind("Super+c", () => {
    const count = gnoblin.shell("ls ~/Downloads | wc -l", { stdout: "capture" });
    count.wait().then(() => gnoblin.spawn(["notify-send", `${count.exit?.stdout?.trim()} files`]));
});
```

```python
from gnoblin import BindingEvent, Gnoblin, SpawnOptions, StreamMode

gnoblin = Gnoblin.get()


def count_downloads(event: BindingEvent) -> None:
    count = gnoblin.shell("ls ~/Downloads | wc -l", SpawnOptions(stdout=StreamMode.CAPTURE))

    def announce() -> None:
        finished = count.exit
        gnoblin.spawn(["notify-send", f"{((finished.stdout if finished else None) or '').strip()} files"])

    count.wait().then(announce)


gnoblin.input.primary.bind("Super+c", count_downloads)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{BindingEvent, Gnoblin, SpawnOptions, StreamMode};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Rc::new(Gnoblin::get(&objects)?);
    let input = model.input()?;
    let primary = input.primary()?;
    let spawner = Rc::clone(&model);
    primary.bind(
        "Super+c",
        Some(move |_event: BindingEvent| {
            let count = spawner.shell(
                "ls ~/Downloads | wc -l",
                Some(&SpawnOptions {
                    cwd: None,
                    env: None,
                    stdin: None,
                    stdout: Some(StreamMode::Capture),
                    stderr: None,
                    output_limit: None,
                    timeout: None,
                }),
            )?;
            let inner = Rc::clone(&spawner);
            count.wait()?.then(move || {
                let text = count.exit()?.and_then(|exit| exit.stdout).unwrap_or_default();
                inner.spawn(&["notify-send".to_owned(), format!("{} files", text.trim())], None)?;
                Ok(())
            })?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

The call fails when the program cannot start. The error's code is `not_found` for a program that is not there, `busy` when
64 already run and `internal` for another refusal of the system.

A TypeScript or Python caller gets an exception with that code, and a Rust caller gets an `Err`. A bad option is a type
error: a `TypeError` in TypeScript and Python, and an `Err` in Rust.

Each program starts in a process group of its own, so `kill` and `timeout` reach the programs it starts too. At most 64
run at once, and they end with the shell.

Options are an object in TypeScript, keyword arguments or `SpawnOptions(...)` in Python, and a `SpawnOptions` struct in Rust
whose fields are `Option`s, with `None` for the default. The names are the same in all three, except that TypeScript
spells the multi-word ones in camelCase.

| Option                         | Accepted values                             | Default   | Meaning                                          |
| ------------------------------ | ------------------------------------------- | --------- | ------------------------------------------------ |
| `cwd`                          | absolute path                               | inherited | The working directory.                           |
| `env`                          | list of `NAME=value` strings, at most 128   | none      | Variables added to the session environment.      |
| `stdin`                        | string, at most 1 MiB                       | empty     | Written to standard input, which is then closed. |
| `stdout`                       | `inherit`, `discard`, `capture`             | `inherit` | Where standard output goes.                      |
| `stderr`                       | `inherit`, `discard`, `capture`             | `inherit` | Where standard error goes.                       |
| `outputLimit` / `output_limit` | integer from 0 to 4194304                   | `65536`   | Bytes kept from each captured stream.            |
| `timeout`                      | milliseconds, more than 0, at most 86400000 | none      | SIGTERM to the group, SIGKILL 2000 ms later.     |

The stream modes are `inherit`, `discard` and `capture`. In TypeScript they are strings. In Python they are the
`StreamMode.INHERIT`, `StreamMode.DISCARD` and `StreamMode.CAPTURE` members. In Rust they are the `StreamMode` variants
`Inherit`, `Discard` and `Capture`.

- `inherit` leaves the stream where the shell's own is.
- `discard` throws the output away.
- `capture` keeps it, up to `outputLimit` (`output_limit` in Python and Rust) bytes, for `wait()` to return.

## Programs the configuration starts

A spawn made at the top level of the configuration, outside any handler, starts a program for the configuration, and a
reload looks after it. At the top level means the file's own statements in TypeScript and Python, and the body of `main`
outside any closure in Rust. The configuration is `shell.ts`, `shell.py` or the shell library:

- The program starts when the new configuration takes over. A configuration that fails to load starts nothing, and the
  programs of the configuration that keeps running are left alone.
- A program the running configuration started with the same command and options is not started again. The new
  configuration's `Process` is the one that already runs, with its `id` and `pid`.
- A program the running configuration started that the new configuration does not start receives SIGTERM.
- A program a key binding or a listener started belongs to you, and a reload leaves it running.

Until the configuration has taken over, the `pid` of a `Process` made at the top level reads 0. A program that cannot start
then ends with exit code 127, so a handle never stays running for a program that does not, and the reload reports why.

## The `Process` object

Each member is listed by its TypeScript, Python and Rust spelling. Rust members are methods, so they take `()`.

| TypeScript | Python      | Rust          | Meaning                                                                             |
| ---------- | ----------- | ------------- | ----------------------------------------------------------------------------------- |
| `id`       | `id`        | `id()`        | The shell's number for the process. What `byId` (`by_id` in Python and Rust) takes. |
| `pid`      | `pid`       | `pid()`       | The operating system's process id, 0 until a top-level spawn starts.                |
| `argv`     | `argv`      | `argv()`      | The program and its arguments.                                                      |
| `alive`    | `alive`     | `alive()`     | `true` while it runs.                                                               |
| `status`   | `status`    | `status()`    | `running` or `exited`.                                                              |
| `exitCode` | `exit_code` | `exit_code()` | The exit status, or nothing after a signal or while it runs.                        |
| `signal`   | `signal`    | `signal()`    | `term`, `kill` and so on after a signal, nothing after a normal exit.               |
| `timedOut` | `timed_out` | `timed_out()` | `true` when `timeout` ended it.                                                     |
| `exit`     | `exit`      | `exit()`      | The `ProcessExit` record once it ended, nothing before.                             |

### `wait()`

`wait()` returns the same `Operation` each time. It succeeds when the process ends, whatever its status, and then `exit`
holds:

- `exitCode` (`exit_code`), `signal` and `timedOut` (`timed_out`), as above.
- `stdout` and `stderr`: the captured stream as a string, decoded as UTF-8 with replacement characters, or nothing when the
  stream was not captured.
- `stdoutTruncated` and `stderrTruncated` (`stdout_truncated` and `stderr_truncated`): `true` when the output passed
  `outputLimit`.

Call `then` on the operation to run code once it succeeds:

- TypeScript: `.then(() => ...)`.
- Python: `.then(lambda: ...)`.
- Rust: `.then(move || { ...; Ok(()) })`. Here `wait()` and `then` return a `Res`, so they take `?`.

### `kill(signal?)`

`kill` sends a signal to the process group and returns an `Operation`. The signal is `term` (the default), `kill`, `int`,
`hup`, `usr1`, `usr2`, `stop` or `cont`. The operation fails with `not_found` once the process has ended.

TypeScript and Python accept the signal as a string, such as `"term"`. Python also accepts a `ProcessSignal` member, such as
`ProcessSignal.TERM`. Rust takes only a `ProcessSignal` member, such as `ProcessSignal::Term`.

## Find and watch processes

- `list()` returns the running processes, oldest first.
- `byId(id)` (`by_id(id)` in Python and Rust) returns the process with that `id`, or nothing.
- `addListener(listener)` (`add_listener` in Python and Rust) attaches a `ProcessListener`. Its `onExited` method (`on_exited` in Python and Rust) runs when any process ends.
- `exitCode` (`exit_code`) is read there. It is nothing after a signal.
- `removeListener(listener)` (`remove_listener`) detaches it.
- In Rust, build the listener with `ProcessListenerHandlers` and `implement`, as the example above does.

A window's process is not linked to the `Process` that started it: no client has a `process` property in any language. A
reload ends the old configuration's handles and callbacks with it.
