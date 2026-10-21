# Session and Policy Commands

Inspect session state, privacy, launch feedback, and request session actions.

Session commands require a running compositor; system power actions depend on host session services.

## Session and policy commands

| Command                                                           | Use                                                                                      |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `status`                                                          | Read live session state and lock availability (API 1.29+)                                |
| `logout`                                                          | End the session and return to the login manager (API 1.32+)                              |
| `session activity`                                                | Read the latest idle-monitor sample (API 1.24+)                                          |
| `session lock`                                                    | Ask a subscribed shell client to lock the session (API 1.21+)                            |
| `ping`                                                            | Check whether the compositor control socket responds                                     |
| `version`                                                         | Read the running compositor build identity                                               |
| `config path`, `config default`, `config show`, `config reload`   | Find, print, inspect, or reload the active configuration                                 |
| `reload`                                                          | Alias for `config reload`                                                                |
| `capabilities`                                                    | List compositor and protocol capabilities                                                |
| `focus history [--workspace-id ID] [--monitor-id ID] [--limit N]` | List recent windows in focus order; limit is 1–256, default 50 (API 1.19+)               |
| `focus policy`                                                    | Read the committed focus behavior (API 1.19+)                                            |
| `input list`, `input current`                                     | Inspect configured and selected keyboard sources                                         |
| `input select TYPE ID`                                            | Select an exact source from `input list`                                                 |
| `privacy`                                                         | Read the privacy activity sources available in this session                              |
| `privacy stop-sharing`                                            | Ask Mutter to stop tracked screen-sharing sessions (API 1.31+)                           |
| `privacy stop-recording`                                          | Ask Mutter to stop tracked recording sessions (API 1.31+)                                |
| `launch status`                                                   | List pending launch feedback and its revision (API 1.39+)                                |
| `launch begin TOKEN APP [MILLISECONDS]`, `launch end TOKEN`       | Start or end busy-cursor feedback; duration defaults to 3000 ms, clamped to 100–10000 ms |
| `shortcut capture`                                                | Briefly grab the keyboard and print a GTK accelerator or `Super` binding                 |

`session activity` reports the latest native idle-monitor sample. If the
session has no idle monitor, the response reports that activity data is
unavailable. `session lock` requests a lock from a subscribed external shell
client. A successful request means the client received it; use the session
lock state to confirm that the screen is locked.

The privacy stop commands ask Mutter to stop matching tracked sessions. Their
`requested` count reports how many stop calls were issued; it does not confirm
that the sessions have closed.

In a standalone native session, `gnoblinctl config reload` validates and
applies the complete configuration while the session is running. Native
settings, policies, autostart entries, and Lua callbacks are updated as one
reload transaction. If loading or applying the candidate fails, Gnoblin keeps
the previous configuration active. The native open-animation matcher uses
updated rules for windows mapped after reload.

Reload does not replay open animations for windows already mapped.

For example, to inspect a portal decision and change keyboard source:

```sh
gnoblinctl input list --json
gnoblinctl input select xkb us
```

Use a source from `input list`. See [launch feedback](/shell-api/launch-feedback).
Launch feedback does not start an application.

Run `gnoblinctl privacy` to see activity and availability for each source.
Boolean values print as `yes` or `no`; unavailable sources have no activity
value. Camera activity follows running PipeWire nodes marked with the `Camera`
media role.

Location activity follows GeoClue's service state and `InUse` property. If it
is unavailable, check GeoClue's `[agent]` whitelist.

On Fedora, install `gnoblin-geoclue-integration`. For source builds and other
distributions, add Gnoblin to the existing whitelist and keep its other IDs.
Add `--json` to print the `PrivacyState` record. It contains a revision, an
availability flag for each source, and an activity value only when that source
is available.

The command uses the shared Lua read in API 1.47 and requires the Lua
supervisor.

| Source         | Availability field            | Optional activity field |
| -------------- | ----------------------------- | ----------------------- |
| Screen sharing | `available.screen_sharing`    | `screen_sharing`        |
| Microphone     | `available.microphone_in_use` | `microphone_in_use`     |
| Camera         | `available.camera_in_use`     | `camera_in_use`         |
| Location       | `available.location_in_use`   | `location_in_use`       |

Lists use the headed table style in a terminal. `gnoblinctl capabilities`
prints each capability's ID, availability, and description in aligned columns.

`gnoblinctl config show` prints one dotted key per line in alphabetical order.
For example, `compositor.audible_bell` appears before `input_sources.per_window`.

On a terminal, `gnoblinctl status` prints the session state as labeled fields:

```text
STATE           running
LOCK STATE      unlocked
LOCK AVAILABLE  yes
```

See the [shortcuts guide](/guides/shortcuts#key-names) for configuration
examples.
