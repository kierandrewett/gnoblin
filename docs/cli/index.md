# Command-line index

`gnoblinctl` is the command-line client for a running Gnoblin session. Its exit
codes, connection rules, logs and output formats are in the
[gnoblinctl reference](../gnoblinctl.md).

## Session

```sh
gnoblinctl ping
gnoblinctl status
gnoblinctl version
```

## Configuration

```sh
gnoblinctl config get
gnoblinctl config reload
gnoblinctl config validate FILE
gnoblinctl config init [DIR] [--force]
gnoblinctl config migrate [FILE] [-o OUT] [--force] [--dry-run]
```

`config validate`, `config init` and `config migrate` need no running session. `config init` writes a starter `shell.ts`:
the stock desktop as statements against the shell's objects.

`config migrate` converts an `init.lua` to a `shell.ts` in the same style:

- It exits 0 when everything converted, 1 when something was left out and 2 when nothing was written.
- A file with something left out starts with `// INCOMPLETE: N unmapped settings, see comments`.

See [Convert an init.lua](../gnoblinctl.md#convert-an-initlua).

`config reload` also reloads the `shell.ts` of the [TypeScript shell](../typescript-shell.md), and exits 1 with
`file:line:column` when that file fails to load.

## Shell

```sh
gnoblinctl shell list
gnoblinctl shell status
gnoblinctl shell reload
gnoblinctl shell watch [PATH] [--once]
```

- `shell list` prints the table `gnoblin --list-shells` prints and needs no session.
- `shell status` says which generation of the shell runs and how its reloads went.
- `shell reload` reloads the shell and waits for the outcome: exit 0 with `reloaded: generation N`, or exit 1 with the reason,
  the old shell still running.
- `shell watch` reloads the shell each time its module is rebuilt, until you interrupt it; `--once` exits after the first.

`status`, `reload` and `watch` reach a shell that runs in the compositor's process (the default). See
[Start Gnoblin and choose a shell](../starting-gnoblin.md#reload-and-watch).

## TypeScript configuration

```sh
gnoblinctl check shell.ts
gnoblinctl check shell.ts --types
gnoblinctl types init [DIR]
```

- `check` runs the file in the dry-run shell, the shell's own library over an empty tree, and reports a syntax error, a
  missing import or export, a refused value or trigger and a top-level throw at its `.ts` line.
- `check --types` also runs an installed TypeScript compiler.
- `types init` writes the library sources, `gnoblin.d.ts` and `tsconfig.json` for your editor.

Neither needs a running session. See the
[gnoblinctl reference](../gnoblinctl.md#check-and-types).

## Console

```sh
gnoblinctl console
gnoblinctl console --eval 'gnoblin.clients.length'
```

- `console` is a live prompt into the running JavaScript shell; `.exit` or Ctrl-D leaves.
- `console --eval EXPR` prints the value of one expression as JSON.

Both exit 1 when the expression fails or no JavaScript shell is running. See the
[console guide](../js-console.md) and the [gnoblinctl reference](../gnoblinctl.md#console).

## Objects

Every object type the compositor exposes has the same verbs:

```sh
gnoblinctl output list
gnoblinctl output get ID [PROPERTY]
gnoblinctl output watch [ID] [--count N]
gnoblinctl output call ID METHOD [NAME=VALUE]...
```

Run `gnoblinctl --help` for the object types this build knows, `gnoblinctl OBJECT
--help` for the verbs of one type and the signals it declares, and `gnoblinctl
OBJECT call --help` for its methods and their possible values.

## Logs and IPC

```sh
gnoblinctl log --level WARN --since 10m --limit 50
gnoblinctl log --follow --interval 2s
gnoblinctl ipc dump FILE
gnoblinctl completions bash --output PATH
```

## Machine-readable output

Add `--json` before or after any command:

```sh
gnoblinctl --json ping
gnoblinctl status --json
```
