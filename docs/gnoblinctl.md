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

`gnoblinctl config validate FILE`, `gnoblinctl config init`, `gnoblinctl check FILE` and
`gnoblinctl types init` do not connect to a session. They work before Gnoblin
is running. `gnoblinctl console` does not use the compositor's socket either: it
connects to the JavaScript shell's console (see [Console](#console)).

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
  [possible values: name, make, model, modes, mode, scale, transform, primary, enabled, x, y, width, height]
```

## Session commands

- `gnoblinctl ping` exits 0 and prints `pong` when the compositor answers the
  handshake.
- `gnoblinctl status` prints the session state, its revision and the lock. The lock is read from the compositor's `lock`
  node: `locked` from the moment a lock is taken until the cover is gone, `unlocked` otherwise, and `unavailable` when the
  compositor has no lock node or this connection may not read it.
- `gnoblinctl lock` locks the session and waits until the cover is up, then prints `locked`. It exits 0 at once on a
  session that is already locked, and exits 1 with `not-locked` when the lock falls back or does not confirm within
  5 seconds. It never unlocks: only the lock screen's password check does.
- `gnoblinctl version` prints the Gnoblin and IPC versions.

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
`output`, `window` or `workspace`. Every object type takes these verbs, except
that a type with no methods has no `call`:

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

- `output`: `apply`
- `window`: `focus`, `present`, `kill`, `close`, `set_minimized`,
  `set_maximized`, `set_fullscreen`, `move`, `resize`, `set_opacity`,
  `set_scale`, `set_offset`, `reset_visuals`, `set_workspace`, `set_output`,
  `request_menu`
- `session`: `logout`, `poweroff`, `reboot`, `suspend`, `hibernate`

`workspace` declares no methods, so it has no `call` verb. The `session` signals
are `ready`, `bell`, `reload_requested` and `lock_state_changed`. The `window`
signal `menu_requested` is declared in the same way.

Calling a method takes its parameters by name. Leaving one out is refused with an
error that names the ones the method needs:

```sh
gnoblinctl output call 6 apply
```

## Config

- `gnoblinctl config get` prints the configuration the compositor last accepted.
- `gnoblinctl config reload` asks the runtime to re-read its configuration. When the runtime runs the
  [TypeScript shell](typescript-shell.md), the request reloads its `init.ts` as well.
- `gnoblinctl config validate FILE` checks a file with the runtime's parser. It
  needs no session.
- `gnoblinctl config init [DIR] [--force]` writes a starter `init.ts` for the
  [TypeScript shell](typescript-shell.md). It needs no session.

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

`config reload` with the TypeScript shell reloads the shell first and then the configuration document, and the reply is
one answer for both:

```sh
gnoblinctl config reload
```

```text
reloaded: revision 3
```

A `init.ts` that fails to load refuses the whole request before the document is touched. The running shell keeps
its config, the command exits 1, and the message names the file, line and column in the `.ts` source:

```text
gnoblinctl: config-rejected: the shell's configuration was not reloaded, and the running one stays: init.ts:3:22: Unexpected token
```

A document that is refused after the shell reloaded leaves the new shell config running: the two are separate files
and the shell cannot take its swap back. A session with no `init.ts` has nothing to reload on the shell side, and the
request reloads the document alone.

### Write a starter config

`gnoblinctl config init` writes a short, commented `init.ts` into the config directory. That is `$XDG_CONFIG_HOME/gnoblin`
or `~/.config/gnoblin`, and `DIR` names another.

It also runs `gnoblinctl types init`, so an editor completes the keys. Then it prints three steps: open the file, check
it, and save it to reload.

```sh
gnoblinctl config init
```

```text
ok  init.ts written: /home/me/.config/gnoblin/init.ts
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json

Next:
  1. Open /home/me/.config/gnoblin/init.ts in your editor.
  2. Check it with `gnoblinctl check --types /home/me/.config/gnoblin/init.ts`.
  3. Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request.
```

A directory that already has an `init.ts` or an `init.js` is left as it is, and the command exits 1 with an error block
that names the file. An `init.js` counts because the shell reads `init.ts` first, so a new `init.ts` would hide it. Add
`--force` to write the starter over `init.ts`. The first line of the output says `replaced` when it did.

With `--json` it prints `init`, `replaced`, `types`, `tsconfig`, `tsconfig_created` and `next`.

## Check and types

`gnoblinctl check FILE` checks a configuration file without a session. The
extension decides what it does:

- A `.ts`, `.mts`, `.js` or `.mjs` file is loaded through the shell runtime's
  own module loader.
- Any other file is checked as `gnoblinctl config validate FILE` checks it.

The exit code is 0 when the file is fine, 1 when it found a problem and 2 for a
wrong command line.

```sh
gnoblinctl check init.ts
```

```text
ok  init.ts  module graph loads (2 files)
```

### What a module check finds

The module loads in a throwaway runtime with a one second budget. It reports:

- a syntax error
- an import that does not resolve
- an import cycle that fails
- a throw at the top level

Each is `file:line:column` in the `.ts` text you wrote, in an error block on
standard error:

```text
error: init.ts:2:16: Unexpected token
hint:  fix the syntax at that position; types are stripped here, not checked
       (see `--types`)
```

A missing import names the specifier and the file that wrote it:

```text
error: lib/helper.ts:1:22: `./gone.ts` not found under the config root
       (tried /home/me/.config/gnoblin/lib/gone.ts)
hint:  import `./` or `../` paths under the config directory, or `gnoblin`
       modules
```

An import that nothing in the file uses is dropped when types are stripped, as
TypeScript drops it, so the check never loads it.

The throwaway runtime has no compositor. The `gnoblin` imports and the global
`gnoblin` are bound to an inert value that accepts any use. The check covers
your own code and not the names you take from the shell. A wrong name from
`gnoblin` is for `--types` to find.

### Type checking

`--types` also runs an external TypeScript compiler over the file. Gnoblin does
not ship one. The first of these that exists is used:

- `$GNOBLIN_TSC`
- `node_modules/.bin/tsc` beside the file
- `tsc`, `tsc6` or `tsgo` on `PATH`

The compiler gets the generated `gnoblin.d.ts` and a `tsconfig.json` made for
the run in a temporary directory, so nothing is written next to your
configuration. Its output is passed through unchanged, and a compiler that
reports errors makes the command exit 1.

```sh
gnoblinctl check init.ts --types
```

When no compiler is found, the step prints one line and exits 0:

```text
type check skipped: no TypeScript compiler found; install one with `npm i -g typescript` or set GNOBLIN_TSC
```

`--types` on a file that is not a module is refused with exit 2.

With `--json`, `check` prints one document:

- `ok` and `problems`, each with `kind`, `file`, `line`, `column` and `message`
- with `--types`, a `types` object that carries the compiler's output as text

### Editor files

`gnoblinctl types init [DIR]` writes the files an editor needs, so completion
and diagnostics work with no build step. `DIR` defaults to the config
directory, `$XDG_CONFIG_HOME/gnoblin` or `~/.config/gnoblin`.

- `gnoblin.d.ts` is generated from the object schema and rewritten each time,
  so an upgrade reaches the editor.
- `tsconfig.json` is written only when it is absent. Once it exists it is
  yours, and a second run leaves it alone.

```sh
gnoblinctl types init
```

```text
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json
```

When `tsconfig.json` is already there, the second line is an `info` line that
says it was kept, with its path. A directory that cannot be written exits 1 with
an error block.

With `--json` it prints `types`, `tsconfig` and `tsconfig_created`.

## Console

`gnoblinctl console` opens a live prompt into the running JavaScript shell. Each
line is evaluated in the shell's own runtime, between frames, with the same
`gnoblin` global as `init.ts`, and its value is printed. `.exit` or Ctrl-D
leaves. It needs the JavaScript shell, the default (see
[Which shell runs](typescript-shell.md#which-shell-runs)); the guide is
[Console and inspector](js-console.md).

```sh
gnoblinctl console
gnoblinctl console --eval 'gnoblin.clients.length'
```

- `--eval EXPR` evaluates one expression and prints its value as JSON. It exits
  0 when the expression gave a value and 1 when it failed. `undefined` prints
  as `null`.
- `--json` prints each reply as the shell sent it: `ok`, `type`, `value`,
  `text` and `truncated`, or `error` with `kind`, `message`, `location` and
  `stack`.

With no shell listening, `console` exits 1 with an error block that names the
socket it tried and a hint:

```text
error: no JavaScript shell is running
why:   cannot connect to /run/user/1000/gnoblin/js.sock: No such file or directory
hint:  the session must run the JavaScript shell, which is the default: unset
       `GNOBLIN_SHELL_VM` (or set it to `js`), keep an `init.ts` rather than
       only an `init.lua`, and check that `GNOBLIN_JS_CONSOLE` is not `0`
```

The prompt prints each result's text on standard output and each error as an
error block on standard error, and goes on. When standard input is not a
terminal there are no prompts and a failing line makes the exit code 1.

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
