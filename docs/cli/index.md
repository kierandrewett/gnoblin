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
```

`config validate` and `config init` need no running session. `config init` writes a starter `init.ts`. `config reload` also reloads the `init.ts` of the
[TypeScript shell](../typescript-shell.md), and exits 1 with `file:line:column` when that file fails to load.

## TypeScript configuration

```sh
gnoblinctl check init.ts
gnoblinctl check init.ts --types
gnoblinctl types init [DIR]
```

- `check` loads the module graph and reports a syntax error, a missing import
  or a top-level throw at its `.ts` line.
- `check --types` also runs an installed TypeScript compiler.
- `types init` writes `gnoblin.d.ts` and `tsconfig.json` for your editor.

Neither needs a running session. See the
[gnoblinctl reference](../gnoblinctl.md#check-and-types).

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
