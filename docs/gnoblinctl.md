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

These commands do not connect to a session, and work before Gnoblin is running:

- `gnoblinctl config validate FILE`, `gnoblinctl config init` and `gnoblinctl config migrate`.
- `gnoblinctl check FILE` and `gnoblinctl types init`.

`gnoblinctl console` does not use the compositor's socket either: it connects to the JavaScript shell's console (see
[Console](#console)).

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

Calling a method takes its parameters by name. Leaving out a parameter the method
requires is refused with an error that names the ones it needs. A parameter the schema
marks optional may be left out, and the method keeps the value it has: `output apply`
takes only the settings it changes.

```sh
gnoblinctl window call 6 set_opacity
gnoblinctl output call 6 apply supersample=2
```

## Config

- `gnoblinctl config get` prints the configuration the compositor last accepted.
- `gnoblinctl config reload` asks the runtime to re-read its configuration. When the runtime runs the
  [TypeScript shell](typescript-shell.md), the request reloads its `shell.ts` as well.
- `gnoblinctl config validate FILE` checks a file with the runtime's parser. It
  needs no session.
- `gnoblinctl config init [DIR] [--force]` writes a starter `shell.ts` for the
  [TypeScript shell](typescript-shell.md). It needs no session.
- `gnoblinctl config migrate [FILE] [-o OUT] [--force] [--dry-run]` converts an `init.lua` to a `shell.ts`. It needs
  no session. See [Convert an init.lua](#convert-an-init-lua).

```sh
gnoblinctl config validate FILE
```

```text
valid
```

A `.ts` or `.js` file is checked as `gnoblinctl check` checks it, so a syntax error exits 1 with the
`error: file:line:column` block shown under [What a module check finds](#what-a-module-check-finds). A JSON document
that does not parse exits 1 with `invalid-config`.

`config reload` with the TypeScript shell reloads the shell first and then the configuration document, and the reply is
one answer for both:

```sh
gnoblinctl config reload
```

```text
reloaded: revision 3
```

A `shell.ts` that fails to load refuses the whole request before the document is touched. The running shell keeps
its config, the command exits 1, and the message names the file, line and column in the `.ts` source:

```text
gnoblinctl: config-rejected: the shell's configuration was not reloaded, and the running one stays: shell.ts:3:22: Unexpected token
```

A document that is refused after the shell reloaded leaves the new shell config running: the two are separate files
and the shell cannot take its swap back. A session with no `shell.ts` has nothing to reload on the shell side, and the
request reloads the document alone.

## Shell

- `gnoblinctl shell list` prints every shell `gnoblin` can run, in the order it looks, and marks the one that would run. It needs
  no session, and it is the table `gnoblin --list-shells` prints.
- `gnoblinctl shell status` prints the running shell's generation, how many reloads committed and failed, the last outcome and
  the last failed build.
    - A shell that failed its start begins with `state: failed` and the error, with the file and line of a script, until a
      reload loads it.
    - `--json` adds `state` (`running`, `failed` or `stopped`) and `error`.
- `gnoblinctl shell reload` asks the shell to reload and waits until it has: a reload runs on the shell's own thread between its
  callbacks, so the command reads the status until one has finished.
- `gnoblinctl shell watch [PATH] [--once]` reloads the shell each time its module (default: the one `gnoblin` finds) is rebuilt,
  until interrupted; `--once` exits after the first reload, with that reload's exit code.

```sh
gnoblinctl shell reload
```

```text
reloaded: generation 3
```

A reload that cannot take over leaves the running shell serving, exits 1 and prints the shell host's own text:

```text
gnoblinctl: reload-failed: reload of /home/me/dev/tiler/target/debug/libtiler.so failed, still running generation 2: /home/me/dev/tiler/target/debug/libtiler.so: cannot load the shell library: file too short
```

The shell members are the session owner's, as `config reload` is. A session whose shell is not in the compositor's process (none
runs, or `--isolate`) answers `no-shell`.

### Write a starter config

`gnoblinctl config init` writes a short, commented `shell.ts` into the config directory. That is `$XDG_CONFIG_HOME/gnoblin`
or `~/.config/gnoblin`, and `DIR` names another.

The file is the stock desktop written as statements against the shell's objects: four floating workspaces added with
`gnoblin.workspaces.add`, the titlebar, window menu and alert, and the stock keys bound with `input.bind`. It is the
statements the shell runs when there is no `shell.ts`, so deleting one removes that part of the desktop.

It also runs `gnoblinctl types init`, so an editor completes the names. Then it prints three steps: open the file, check
it, and save it to reload.

```sh
gnoblinctl config init
```

```text
ok  shell.ts written: /home/me/.config/gnoblin/shell.ts
ok  library refreshed: /home/me/.config/gnoblin/.gnoblin/lib (200 files, the real sources of `gnoblin-core` and `gnoblin-std`)
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/.gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json

Next:
  1. Open /home/me/.config/gnoblin/shell.ts in your editor.
  2. Check it with `gnoblinctl check --types /home/me/.config/gnoblin/shell.ts`.
  3. Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request.
```

A directory that already has a `shell.ts`, a `shell.js` or any other `shell.*` file is left as it is, and the command
exits 1 with an error block that names the file. A second shell file would leave two equally preferred shells, and
Gnoblin does not choose between them.

Add `--force` to write the starter over `shell.ts`. The first line of the output says `replaced` when it did.

With `--json` it prints `init`, `replaced`, `next` and the fields of [`types init`](#editor-files).

### Convert an init.lua

`gnoblinctl config migrate [FILE] [-o OUT] [--force] [--dry-run]` converts a Lua config to TypeScript. `FILE` is the
`init.lua` to convert, and the command writes `shell.ts` beside it, or to `OUT`. It reads the Lua as text and
never runs it, so it needs no session.

The `shell.lua` beside an `init.lua`, and every file a `gnoblin.load` names, are read too and become one `shell.ts`, with
a comment for each file. The output is statements against the shell's objects, not a config object:

- Workspaces are `gnoblin.workspaces.add(new Workspace(...))` and shortcuts are `input.bind(...)`.
- A window rule is a style on a selection or a listener.
- The stock desktop is written out too, because a `shell.ts` replaces the stock default.

See [the converter](migrating-from-lua.md#the-converter) for what it converts, and
[the settings with no TypeScript home yet](migrating-from-lua.md#settings-with-no-typescript-home-yet) for what it
leaves out and why.

It runs `gnoblinctl types init` on the directory it wrote to, runs the converted file once in the dry-run shell (the
evaluation `gnoblinctl check` makes) before it writes, and prints what converted, what was left out and three steps.

```sh
gnoblinctl config migrate
```

```text
ok  shell.ts written: /home/me/.config/gnoblin/shell.ts
ok  library refreshed: /home/me/.config/gnoblin/.gnoblin/lib (200 files, the real sources of `gnoblin-core` and `gnoblin-std`)
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/.gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json

Converted
---------
workspaces  4
keys        7

Left out (2)
------------
warn  init.lua:31  gnoblin.on("gnoblin.window.created", function(w) ...
      an event handler runs code: write a `ClientListener` and attach it with `gnoblin.clients.addListener`
      see docs/migrating-from-lua.md#what-the-converter-cannot-do
warn  config/10-windows.lua:5  ["focus-mode"] = "click"
      `window-management.focus-mode` has no TypeScript home yet: the shell has no focus policy on the seat; the runtime keeps click focus, new windows focused and raise on click
      see docs/migrating-from-lua.md#settings-with-no-typescript-home-yet

Next:
  1. Review /home/me/.config/gnoblin/shell.ts: each UNMAPPED comment is something to port by hand.
  2. Check it with `gnoblinctl check --types /home/me/.config/gnoblin/shell.ts`.
  3. Log out and back in. From the next session the TypeScript shell runs and /home/me/.config/gnoblin/shell.ts takes precedence over /home/me/.config/gnoblin/init.lua.
```

The exit code says how it went:

- 0: everything converted.
- 1: the file was written and something was left out, or the converted file did not load. A file with something left out
  starts with `// INCOMPLETE: N unmapped settings, see comments`, and each setting is an `UNMAPPED` comment.
- 2: nothing was written. The command line was wrong, a file could not be read, the Lua does not parse, or a file is in
  the way. The command stops instead of replacing an existing `OUT`, and instead of writing a `shell.ts` beside another
  `shell.*` file, unless you add `--force`. `--force` replaces that file and nothing else: the Lua files are only read.

Three options change what it does:

- `--dry-run` prints the TypeScript on standard output and the report on standard error, and writes nothing. It does
  not stop at a file in the way.
- `-o OUT` writes the TypeScript to `OUT`, and the editor files beside it.
- With `--json` it prints `sources`, `converted`, `unmapped` (each with `file`, `line`, `summary`, `reason` and
  `pointer`), `notes`, `load_problem`, `written`, `dry_run`, and, after a write, `init`, `types`, `tsconfig`,
  `tsconfig_created` and `next`. With `--dry-run` it also prints `typescript`.

## Check and types

`gnoblinctl check FILE` checks a configuration file without a session. The
extension decides what it does:

- A `.ts`, `.mts`, `.js` or `.mjs` file is run in the [dry-run shell](#the-dry-run-shell), through the shell runtime's
  own module loader.
- Any other file is checked as `gnoblinctl config validate FILE` checks it.

The exit code is 0 when the file is fine, 1 when it found a problem and 2 for a
wrong command line.

```sh
gnoblinctl check shell.ts
```

```text
ok  shell.ts  loads in the dry-run shell
```

### What a module check finds

The module runs in a throwaway shell with a one second budget. It reports:

- a syntax error
- an import that does not resolve, or a name the `gnoblin-core` or `gnoblin-std` module does not export
- an import cycle that fails
- a value, trigger or object the shell refuses
- a throw at the top level

Each is `file:line:column` in the `.ts` text you wrote, in an error block on
standard error:

```text
error: shell.ts:2:16: Unexpected token
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
TypeScript drops it, so the check never loads it. `--types` does check it.

A name the `gnoblin-core` or `gnoblin-std` module does not export, and a value or
a trigger the shell refuses, are reported the way a session reports them:

```text
error: data/js/gnoblin-core/bindings/binding.ts:316:110: binding "bind-2:Super+k" conflicts with
       "bind-1:Super+k": both match Super+k at priority 0; give one a different
       priority or trigger
hint:  the module threw while loading, before any handler could run
```

### The dry-run shell

`check` runs the file in the dry-run shell: the same evaluation the services
role makes of a `shell.ts` before the runtime has committed anything. It is the
shell's own library and objects over a tree with a root and one seat, and no
windows, outputs or devices.

A file that builds workspaces, binds keys and attaches listeners loads as
written. Nothing starts: no program is spawned and no key is claimed.

A file that reads live state sees that empty world. Counting windows, walking
`gnoblin.clients` and asking for the focused window all work, and answer that
there is none.

A file that assumes a window or an output is there, and reads a property of it,
throws, as it does in a session with none. The `gnoblin-core` and `gnoblin-std`
modules are the real ones and not stand-ins, so the names you import from them
must exist.

### Type checking

`--types` also runs an external TypeScript compiler over the file. Gnoblin does
not ship one. The first of these that exists is used:

- `$GNOBLIN_TSC`
- `node_modules/.bin/tsc` beside the file
- `tsc`, `tsc6` or `tsgo` on `PATH`

The compiler gets the real library and the generated `gnoblin.d.ts`, written into a temporary directory with a
`tsconfig.json` made for the run, so nothing is written next to your configuration. Its output is passed through
unchanged, and a compiler that reports errors makes the command exit 1.

```sh
gnoblinctl check shell.ts --types
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

`gnoblinctl types init [DIR]` writes the files an editor needs, so completion, hover and go-to-definition work with no
build step. `DIR` defaults to the config directory, `$XDG_CONFIG_HOME/gnoblin` or `~/.config/gnoblin`.

The types of `import ... from "gnoblin-core"` and `"gnoblin-std"` are the library's own TypeScript source, not a description of it. The command
writes that source as files and points the compiler at them, so go-to-definition opens the file the shell runs, hover
shows the comment written above the class, and a class extends the class it really extends (`Camera3D` is a `Node`).

```text
~/.config/gnoblin/
  shell.ts
  tsconfig.json              yours: extends .gnoblin/tsconfig.json
  .gnoblin/                  generated, rewritten on every run
    tsconfig.json            the compiler options and the two `paths` that find the library
    gnoblin.d.ts             the `gnoblin` and `console` globals
    lib/                     the library, one directory a package: `gnoblin-core` is gnoblin-core/index.ts,
                             `gnoblin-std/scene` is gnoblin-std/scene.ts, `gnoblin-std/scene/node` is
                             gnoblin-std/scene/node.ts, and so on
    VERSION                  the release that wrote the directory
    .gitignore               so a repository that holds your config skips the directory
```

- `.gnoblin/` is rewritten each time, so an upgrade reaches the editor. A file whose bytes are already right is left
  alone, and a module the library does not have is removed, so that its import does not resolve. Do not edit it, and do
  not commit it.
- `tsconfig.json` is written only when it is absent. Once it exists it is yours, and a second run leaves it alone.
  It extends `.gnoblin/tsconfig.json`, so the options and the paths follow the release; add your own `include`,
  `exclude` or options beside the `extends`. If you keep a `tsconfig.json` that does not extend it, the command says
  so, because the editor cannot find the library without it.

```sh
gnoblinctl types init
```

```text
ok  library refreshed: /home/me/.config/gnoblin/.gnoblin/lib (200 files, the real sources of `gnoblin-core` and `gnoblin-std`)
ok  gnoblin.d.ts refreshed: /home/me/.config/gnoblin/.gnoblin/gnoblin.d.ts
ok  tsconfig.json created: /home/me/.config/gnoblin/tsconfig.json
```

When `tsconfig.json` is already there, the last line is an `info` line that says it was kept, with its path, or a `warn`
line when it does not extend the generated one. A directory that cannot be written exits 1 with an error block.

A release before this layout wrote `gnoblin.d.ts` and a `tsconfig.json` that mapped `gnoblin` to it. The command removes
that `gnoblin.d.ts` and replaces that `tsconfig.json`, when they are still the files it wrote. A file you changed is
left alone.

With `--json` it prints `types`, `library`, `library_files`, `tsconfig`, `tsconfig_created`, `tsconfig_replaced` and
`tsconfig_linked`.

## Console

`gnoblinctl console` opens a live prompt into the running JavaScript shell. Each
line is evaluated in the shell's own runtime, between frames, with the same
`gnoblin` global as `shell.ts`, and its value is printed. `.exit` or Ctrl-D
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
  `stack`, and in both `output`, the lines the expression printed through
  `console`.

What an expression prints through `console` comes out before its value:

- `log`, `info` and `debug` go to standard output.
- `warn` and `error` go to standard error.
- With `--eval` all of them go to standard error, so standard output stays the
  value alone.

With no shell listening, `console` exits 1 with an error block that names the
socket it tried and a hint:

```text
error: no JavaScript shell is running
why:   cannot connect to /run/user/1000/gnoblin/js.sock: No such file or directory
hint:  the session must be running, with the JavaScript shell: check that the
       session is up and that `GNOBLIN_JS_CONSOLE` is not `0`
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
