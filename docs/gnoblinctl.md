# gnoblinctl

`gnoblinctl` is the command-line client for a running Gnoblin session.

It speaks IPC 0.2.0. A compositor at another IPC version is refused with
exit 4, and the error names both versions. The list of commands is in the
[command index](cli/index.md). This page covers how the client connects, what
its exit codes mean, how to read logs, and how to script its output.

## Version

`gnoblinctl --version` prints the client's own version and needs no session:

```sh
gnoblinctl --version
```

```text
gnoblinctl 0.2.0-alpha.0
```

`gnoblinctl version` asks the running compositor for the Gnoblin version and the
IPC version this connection speaks:

```sh
gnoblinctl version
```

```text
gnoblin 0.2.0
ipc 0.2.0
```

## Connection problems

Session commands connect to the compositor's IPC socket. The socket is
`$GNOBLIN_IPC_SOCKET` when that is set, and otherwise
`$XDG_RUNTIME_DIR/gnoblin/ipc.sock`. There are no command-line flags for the
socket path.

When the socket cannot be reached, the client exits 3 and names the path it
tried:

```sh
GNOBLIN_IPC_SOCKET=/nonexistent/ipc.sock gnoblinctl ping
```

```text
gnoblinctl: cannot-connect: cannot connect to /nonexistent/ipc.sock: No such file or directory (os error 2)
```

To diagnose it:

1. Run `gnoblinctl ping`. Exit 0 with `pong` means the compositor answered.
2. Check the socket path the client used. It is the value of
   `GNOBLIN_IPC_SOCKET`, or `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`.
3. If the compositor answers but a command exits 4, the compositor and the
   client speak different IPC versions. Upgrade the one that is older.

`gnoblinctl config validate FILE` does not connect to a session. It works
before Gnoblin is running.

## Exit codes

Scripts can rely on these codes:

| Code | Meaning                                                                        |
| ---- | ------------------------------------------------------------------------------ |
| 0    | The command did what it was asked.                                             |
| 1    | The compositor answered with an error, or the command failed at runtime.       |
| 2    | The command line was wrong. A usage message goes to standard error.            |
| 3    | The compositor could not be reached. Start the session.                        |
| 4    | The compositor speaks an IPC version this client cannot use. Upgrade one side. |

A failed command prints one line to standard error, in this form:

```text
gnoblinctl: CODE: message
```

Asking for an object that is not the type the
command names gives `object-unknown` and exit 1:

```sh
gnoblinctl window get 1
```

```text
gnoblinctl: object-unknown: identity 1 is a gnoblin.session, not a gnoblin.window
```

A misspelt property or method is a usage error, so it exits 2 and lists the
values that are accepted:

```text
$ gnoblinctl output get 6 nosuch
```

```text
error: invalid value 'nosuch' for '[PROPERTY]'
  [possible values: name, make, model, modes, mode, scale, transform, primary, enabled, x, y, width, height, layout]
```

## Session commands

- `gnoblinctl ping` exits 0 and prints `pong` when the compositor answers the
  handshake.
- `gnoblinctl status` prints the session state, its revision and the lock.
- `gnoblinctl version` prints the Gnoblin and IPC versions.
- `gnoblinctl capabilities` lists every capability and whether this connection
  holds it.

```sh
gnoblinctl status
```

```text
state     running
revision  1880
lock      unlocked
```

With `--json`, `status` prints one object:

```sh
gnoblinctl status --json
```

```text
{"lock_available":true,"lock_state":"unlocked","revision":1880,"state":"running"}
```

## Objects

Each object the compositor exposes has a command named after its type, such as
`output`, `window` or `workspace`. Every object type takes the same verbs:

- `gnoblinctl OBJECT list` lists the live objects of that type.
- `gnoblinctl OBJECT get ID [PROPERTY]` prints one object, or one property of it.
- `gnoblinctl OBJECT watch [ID] [--count N]` prints observations as they arrive.
  `--count N` stops after N observations. An object that emits no signal prints
  nothing until it is stopped.
- `gnoblinctl OBJECT call ID METHOD [NAME=VALUE]...` calls a declared method and
  waits for its operation to finish.

```sh
gnoblinctl output list
gnoblinctl output get 6 width
```

```text
1024
```

`gnoblinctl OBJECT call --help` lists the methods that one type declares, with
their possible values, and `gnoblinctl OBJECT --help` names its signals. The
methods that exist are:

- `output`: `set_layout`, `apply`
- `window`: `focus`, `present`, `kill`, `close`, `set_minimized`,
  `set_maximized`, `set_fullscreen`, `move`, `resize`, `set_workspace`,
  `set_output`, `add_effect`, `request_menu`
- `workspace`: `set_layout`, `layout_command`
- `layers`: `register`

`session` declares no methods. Its signals are `ready`, `bell` and
`reload_requested`. The `window` signal `menu_requested` and the `workspace`
signals `layout_changed` and `layout_failed` are declared in the same way.

Calling a method takes its parameters by name. Leaving one out is refused with an
error that names the ones the method needs:

```sh
gnoblinctl output call 6 set_layout
```

## Config

- `gnoblinctl config get` prints the configuration the compositor last accepted.
- `gnoblinctl config reload` asks the runtime to re-read its configuration.
- `gnoblinctl config validate FILE` checks a file with the runtime's parser. It
  needs no session.

```sh
gnoblinctl config validate FILE
```

```text
valid
```

A file that does not parse exits 1 with `invalid-config`, and names the line:

```text
gnoblinctl: invalid-config: /home/node/cli-e2e/bad.lua:1: syntax error near 'is'
```

## Logs

`gnoblinctl log` prints the records in `gnoblin.jsonl`, oldest first. The file
is read from `--dir`, which defaults to `$XDG_STATE_HOME/gnoblin/log`.

- `--level LEVEL` keeps records at this level or above: `TRACE`, `DEBUG`,
  `INFO`, `WARN` or `ERROR`.
- `--role ROLE` keeps records written by one process role, such as `compositor`.
- `--target TARGET` keeps records whose event target matches exactly.
- `--since TIME` and `--until TIME` bound the records by time. `TIME` is an
  RFC 3339 timestamp such as `2026-11-06T09:00:00Z`, or an age such as `10m`,
  `2h` or `1d`. Anything else exits 2.
- `--limit N` prints the last N records that match, in the order they were written. The default is 100.
- `--follow` keeps printing new records as they are written. It polls every
  `--interval`, which is `500ms` or `2s` and defaults to `500ms`, until it is
  interrupted.

```sh
gnoblinctl log --level WARN --since 10m --limit 50
gnoblinctl log --follow --interval 2s
```

With `--json`, each record is one JSON object with the keys `component`,
`fields`, `level`, `message`, `role`, `target` and `time_ms`:

```sh
gnoblinctl --json log --limit 1
```

If the log file does not exist, the command exits 1 and names the file it
looked for:

```sh
gnoblinctl log --dir /nonexistent
```

```text
gnoblinctl: log: no log file at /nonexistent/gnoblin.jsonl
```

## Output for scripts

Add `--json` before or after any command for machine-readable output. Object
commands return an `objects` array, or a `value` for a single property:

```sh
gnoblinctl --json output get 6 width
```

```text
{"id":6,"property":"width","value":1024}
```

Read exit codes, not the text, to decide whether a command succeeded.

## IPC dump

`gnoblinctl ipc dump` decodes recorded IPC frames into readable lines. Secret
payloads are dropped. It reads standard input when no file is given, or when
the file is `-`:

```sh
gnoblinctl ipc dump FILE
gnoblinctl ipc dump -
```

## Shell completion

`gnoblinctl completions SHELL` writes a completion script for `bash`, `zsh` or
`fish` to standard output. `--output PATH` writes it to a file instead:

```sh
gnoblinctl completions bash --output PATH
gnoblinctl completions zsh --output PATH
```

Source the file from your shell's startup file, or copy it into the directory
your shell reads completions from.

## Help

`gnoblinctl --help` lists the commands. `gnoblinctl COMMAND --help` describes one
command and its arguments.
