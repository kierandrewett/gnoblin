# Logging

Gnoblin logs through the `gnoblin-log` facade. A process that logs through it writes structured lines, which keep their own format.

Anything else a child prints, such as a panic, a Lua `print`, or output from a C library or Xwayland, is read by the guardian and logged as component `raw`. Each such line is tagged with the child's role and the stream it came from.

## Which children are captured

Raw capture is an allowlist by role. A role that is not listed is read and discarded, so its output never reaches the log.

| Role                                                                   | Captured                                                        |
| ---------------------------------------------------------------------- | --------------------------------------------------------------- |
| `compositor`, `runtime`, `portal`                                      | yes                                                             |
| `services`                                                             | only when the guardian starts with `GNOBLIN_LOG_RAW_SERVICES=1` |
| `front`, autostart commands, and any other child without a listed role | no                                                              |

## Scrubbing is best effort

Each captured line goes through heuristics that replace secret-looking text with `[redacted]`. They recognise `password=...`, `"token": "..."`, `Authorization: Bearer ...`, PEM private key blocks, and long opaque tokens. Key names such as password, secret, token, passphrase, answer, key and credential are matched.

This scrubbing is best effort. It cannot recognise every secret, for example a password written as prose or a token that matches none of the rules. It never guarantees that a logged line is free of secrets.

The real protection is the allowlist above:

- Do not capture the output of a process that may print secrets.
- Setting `GNOBLIN_LOG_RAW_SERVICES=1` opts the services role in. Do that only when its output is known to be safe.
