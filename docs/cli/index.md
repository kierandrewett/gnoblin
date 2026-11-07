# gnoblinctl Reference

`gnoblinctl` is the command-line client for a running Gnoblin session. It talks
to the compositor over the IPC socket at `$GNOBLIN_IPC_SOCKET`, or
`$XDG_RUNTIME_DIR/gnoblin/ipc.sock` when that is unset. It speaks one IPC
version and refuses a compositor at another version, naming both versions.

Add `--json` to any command for machine-readable output.

## Session commands

| Command                   | Use it to                                                  |
| ------------------------- | ---------------------------------------------------------- |
| `gnoblinctl ping`         | Exit 0 when the compositor answers the handshake           |
| `gnoblinctl status`       | Read the session state, its revision and the lock          |
| `gnoblinctl version`      | Read the Gnoblin version and the IPC version in use        |
| `gnoblinctl capabilities` | List every capability and whether this connection holds it |

## Configuration

| Command                           | Use it to                                                                   |
| --------------------------------- | --------------------------------------------------------------------------- |
| `gnoblinctl config get`           | Print the configuration the compositor last accepted                        |
| `gnoblinctl config reload`        | Re-read the configuration in place; a refused document is not applied       |
| `gnoblinctl config validate FILE` | Check a configuration document with the runtime's parser, without a session |

`config validate` does not connect to a session, so it works before Gnoblin is
running.

## Objects

Each object the compositor exposes has its own command, generated from the
schema. Every object takes the same verbs:

- `gnoblinctl OBJECT list` lists the live objects of that type.
- `gnoblinctl OBJECT get` reads their properties.
- `gnoblinctl OBJECT watch` prints changes as they happen.
- `gnoblinctl OBJECT call METHOD` calls a method.

Run `gnoblinctl --help` for the objects this build knows, and
`gnoblinctl OBJECT --help` for the properties, methods and signals of one.

## Logs and IPC

- `gnoblinctl log` prints stored log records, oldest first. Filter them with
  `--level`, `--role` and `--store`. The [logging guide](/guides/logging) covers
  following a live store.
- `gnoblinctl ipc dump [FILE]` decodes recorded IPC frames into readable lines.
  Secret payloads are dropped. Without a file it reads standard input.
- `gnoblinctl completions SHELL` writes a shell completion script. Use
  `--output PATH` to write it to a file.

## Help

`gnoblinctl --help` lists the commands. A mistyped command names the closest
match. `man gnoblinctl` has the same reference.
