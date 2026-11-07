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

## Console format

`GNOBLIN_LOG_FORMAT` chooses what a process prints. The database records every event whatever the format. An unknown value is ignored, the configured format applies, and a warning names the valid values.

| Value            | What it prints                                                              |
| ---------------- | --------------------------------------------------------------------------- |
| `auto` (default) | Journal, `<N>` lines, pretty or plain, chosen as described below            |
| `journal`        | Native journal entries; `<N>` lines on stderr if the journal is unreachable |
| `journal-stream` | `<N>component: message` lines, with the priority read by systemd            |
| `pretty`         | Coloured `LEVEL  role/component  message  key=value` lines                  |
| `plain`          | `LEVEL  role/component  message  key=value` lines, without colour           |
| `json`           | One JSON object per line                                                    |
| `off`            | Nothing; only the database records                                          |

`auto` looks at the environment. It uses the native journal when systemd connects stderr to the journal and the journal socket exists. It uses `journal-stream` when only stderr reaches the journal. Otherwise it uses `pretty` on a terminal and `plain` elsewhere.

## Reading the journal

Under systemd each record goes to the journal with `SYSLOG_IDENTIFIER=gnoblin-<role>`, a `PRIORITY` from its level, and the fields `GNOBLIN_ROLE`, `GNOBLIN_COMPONENT` and `GNOBLIN_TARGET`. For a user session service, read it with `--user`:

```sh
journalctl --user -t gnoblin-compositor GNOBLIN_COMPONENT=wayland -p warning -f
```

This follows warnings and errors from the compositor's `wayland` component. The journal keeps only what the console sends. The database keeps every event that passes `GNOBLIN_LOG`, so it is the place to look after a crash or when the journal has rotated.

## Reading the database

The database defaults to `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, and `GNOBLIN_LOG_DB` names another file or `off`. `gnoblinctl log` prints its records, oldest first, without the compositor running:

```sh
gnoblinctl log --role compositor --level warn --limit 50
```

`--store DIR` reads another store directory, and `--json` prints one JSON object per line. The command opens the log read-only, so it works while the session is down.

`--since` and `--until` bound the records by time, as an RFC 3339 timestamp or an age such as `2h`.

`--follow` keeps printing records as they are written, polling every `--interval` (default `500ms`) until interrupted. The full flag reference is in [gnoblinctl](../gnoblinctl.md#logs).

## Adding logging to a crate

Every crate logs through `gnoblin-log`, and never through `tracing` directly. A crate defines its component once, at module or crate root:

```rust
gnoblin_log::logger!(log = "wayland");

log::info!("surface {id} mapped", id = 3);
```

The name `log` is the convention, so call sites read `log::warn!` and `log::debug!` everywhere in the crate. A subsystem with its own name gets a second logger beside it, such as `logger!(wire = "wayland-wire")`. Use `pub(crate)` before the name to share the module across the crate.

Fields follow the message as `key = value`, and `%` or `?` select `Display` or `Debug`. Secrets are redacted before any sink sees the event, so a field needs no special handling.

Guard expensive argument building with `log::enabled!(Level::DEBUG)`. It returns whether that level is on for the component, and costs one relaxed atomic load.

Levels are filtered per component with `GNOBLIN_LOG`, a comma-separated list of `component:level` pairs. A `*` names every component that has no entry of its own:

```sh
GNOBLIN_LOG=wayland:debug,compositor:info,*:warn
```

A bad entry is refused with a named error and the default applies. The `logging` configuration table sets the same levels, and `GNOBLIN_LOG` overrides it.

Component ids are 1 to 24 bytes of `a-z`, `0-9` and `-`, with no leading or trailing dash. The build fails on a bad id. A disabled call formats nothing, and the first call to a component registers it, so `gnoblin_log::components()` lists every component that has logged.
