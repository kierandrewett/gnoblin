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
```

`config validate` needs no running session.

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
