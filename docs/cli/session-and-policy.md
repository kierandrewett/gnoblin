# Session and Policy Commands

Inspect session state, privacy, launch feedback, and request session actions.

Session commands require a running compositor. System power actions depend on
host session services.

## Session and policy commands

| Command                                                           | Use                                                                      |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `status`                                                          | Read live session state and lock availability (API 1.29+)                |
| `logout`                                                          | End the session and return to the login manager (API 1.32+)              |
| `session activity`                                                | Read the idle-monitor sample (API 1.24+)                                 |
| `session lock`                                                    | Ask a subscribed shell client to lock the session (API 1.21+)            |
| `ping`                                                            | Check whether the compositor control socket responds                     |
| `version`                                                         | Read the running compositor build identity                               |
| `config path`, `config default`, `config show`, `config reload`   | Find, print, inspect, or reload the active configuration                 |
| `reload`                                                          | Alias for `config reload`                                                |
| `capabilities`                                                    | List compositor and protocol capabilities                                |
| `focus history [--workspace-id ID] [--monitor-id ID] [--limit N]` | List recent windows in focus order (API 1.19+)                           |
| `focus policy`                                                    | Read the committed focus behavior (API 1.19+)                            |
| `input list`, `input current`                                     | Inspect configured and selected keyboard sources                         |
| `input select TYPE ID`                                            | Select an exact source from `input list`                                 |
| `privacy`                                                         | Read the privacy activity sources available in this session              |
| `privacy stop-sharing`                                            | Ask Mutter to stop tracked screen-sharing sessions (API 1.31+)           |
| `privacy stop-recording`                                          | Ask Mutter to stop tracked recording sessions (API 1.31+)                |
| `launch status`                                                   | List pending launch feedback and its revision (API 1.39+)                |
| `launch begin TOKEN APP [MILLISECONDS]`, `launch end TOKEN`       | Start or end busy-cursor feedback                                        |
| `shortcut capture [--timeout SECONDS]`                            | Briefly grab the keyboard and print a GTK accelerator or `Super` binding |

## Accepted values

| Argument                     | Accepted values                              | Default        |
| ---------------------------- | -------------------------------------------- | -------------- |
| `focus history --limit N`    | Integer from 1 to 256                        | 50             |
| `input select TYPE`          | `xkb` or `ibus`                              | None; required |
| `input select ID`            | The `id` of a source printed by `input list` | None; required |
| `launch begin TOKEN`         | 1 to 128 characters                          | None; required |
| `launch begin APP`           | Application ID, at most 512 characters       | None; required |
| `launch begin MILLISECONDS`  | Integer from 1 to 60000 on the CLI           | 3000           |
| `shortcut capture --timeout` | Integer from 1 to 60 seconds                 | 30 seconds     |

The compositor clamps `launch begin` milliseconds to 100 through 10000 after
the CLI accepts a value. A value of 60000 is therefore accepted by the CLI
but runs for 10000 ms.

`--workspace-id` and `--monitor-id` take the stable ID of a workspace or
monitor, as printed by the matching list command.

## Session state

`session activity` reports the native idle-monitor sample. If the
session has no idle monitor, the response reports that activity data is
unavailable.

`session lock` requests a lock from a subscribed external shell client. A
successful request means the client received it. It does not mean the screen
is locked. Check the lock state from `status` to confirm.

`status` prints `state`, `lock_available`, and `lock_state` when lock state is
known. `state` is `running` while the compositor answers. `lock_state` is
`locked` or `unlocked`. The command also reports a `revision` for the lock-state
snapshot.

On a terminal, `gnoblinctl status` prints the session state as labeled fields:

```text
STATE           running
LOCK STATE      unlocked
LOCK AVAILABLE  yes
```

## Configuration

In a standalone native session, `gnoblinctl config reload` validates and
applies the complete configuration while the session is running. Native
settings, policies, autostart entries, and Lua callbacks are updated as one
reload transaction.

If loading or applying the candidate fails, Gnoblin keeps the previous
configuration active. The native open-animation matcher uses updated rules for
windows mapped after reload. Reload does not replay open animations for windows
already mapped.

`gnoblinctl config show` prints one dotted key per line in alphabetical order.
For example, `compositor.audible_bell` appears before `input_sources.per_window`.

## Input sources

Use a source from `input list`. Pass its `type` as `xkb` or `ibus` and its `id`
exactly as listed:

```sh
gnoblinctl input list --json
gnoblinctl input select xkb us
```

## Launch feedback

`launch begin` shows busy-cursor feedback for the given token and application.
It does not start an application. Call `launch end` with the same token when
the launch is cancelled or the application has started. See
[launch feedback](/shell-api/launch-feedback).

## Privacy

Run `gnoblinctl privacy` to see activity and availability for each source.
Boolean values print as `yes` or `no`. A source that is unavailable has no
activity value.

Add `--json` to print the `PrivacyState` record. It contains a `revision`, an
availability flag for each source, and an activity value only when that source
is available.

| Source         | Availability field            | Activity field      |
| -------------- | ----------------------------- | ------------------- |
| Screen sharing | `available.screen_sharing`    | `screen_sharing`    |
| Microphone     | `available.microphone_in_use` | `microphone_in_use` |
| Camera         | `available.camera_in_use`     | `camera_in_use`     |
| Location       | `available.location_in_use`   | `location_in_use`   |

Camera activity follows running PipeWire nodes marked with the `Camera` media
role.

Location activity follows GeoClue's service state and `InUse` property. If it
is unavailable, check GeoClue's `[agent]` whitelist.

On Fedora, install `gnoblin-geoclue-integration`. For source builds and other
distributions, add Gnoblin to the existing whitelist and keep its other IDs.

The privacy stop commands ask Mutter to stop matching tracked sessions. Their
`requested` count reports how many stop calls were issued. It does not confirm
that the sessions have closed.

The command uses the shared Lua read in API 1.47 and requires the Lua
supervisor.

## Shortcut capture

`shortcut capture` waits for one key press and prints the normalized
accelerator. Escape cancels. For shortcut configuration examples, see the
[shortcuts guide](/guides/shortcuts#key-names).

## Capabilities

Lists use the headed table style in a terminal. `gnoblinctl capabilities`
prints each capability's ID, availability, and description in aligned columns.
