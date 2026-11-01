# gnoblinctl Reference

Use `gnoblinctl` to inspect a running session, control Gnoblin, manage its
configuration, and run local Lua scripts.

Session commands require a running Gnoblin compositor. `gnoblinctl --version` can run without a session.

## Build identity

Run `gnoblinctl --version` to read the installed build identity without a
running session. It reports:

- The Gnoblin version.
- The UTC time of the build.
- The Git commit, marked `(modified)` when the source tree had changes.
- The Git remote.
- The Lua and native API versions.

`gnoblin --version` prints the same identity. Add `--json` to either command to
save the complete build identity as JSON, which also has the build ID and the
GNOME and component versions.

In a Git checkout, the command reads the source manifests, current commit and
tracking remote (or `origin`). In a release tarball, it reads the embedded
source provenance. The installed command reads the identity recorded at build
time. A modified source tree is marked beside its commit; the commit alone does
not identify those local edits.

`gnoblinctl version` reads the build identity through the running compositor
API. It requires an active Gnoblin session.

[Configuration reference](/config/configure)

Control Gnoblin from a terminal or Lua configuration. Configure shell panels
with the external shell's own tools.

## Start here

Run commands from a terminal inside Gnoblin:

| Command                                        | Use it to                                                    |
| ---------------------------------------------- | ------------------------------------------------------------ |
| `gnoblinctl status`                            | Check the running session and lock-state availability        |
| `gnoblinctl logout`                            | End the session and return to the login manager              |
| `gnoblinctl session suspend`                   | Suspend the machine                                          |
| `gnoblinctl session hibernate`                 | Hibernate the machine                                        |
| `gnoblinctl session reboot`                    | Restart the machine                                          |
| `gnoblinctl session poweroff`                  | Shut the machine down                                        |
| `gnoblinctl session activity`                  | Read the idle-monitor sample                                 |
| `gnoblinctl session lock`                      | Ask a subscribed external shell client to lock the session   |
| `gnoblinctl window list`                       | Find open windows and their IDs                              |
| `gnoblinctl window match`                      | Show the values a window rule can match                      |
| `gnoblinctl window thumbnail ID --output PATH` | Save a window thumbnail as a PNG                             |
| `gnoblinctl layer list`                        | Find layer-surface namespaces                                |
| `gnoblinctl input devices`                     | List detected input devices and capabilities                 |
| `gnoblinctl input orientation-lock [VALUE]`    | Read or set the orientation lock                             |
| `gnoblinctl workspace list`                    | Show workspace IDs, names, positions and windows             |
| `gnoblinctl config path`                       | Find the config file your session uses                       |
| `gnoblinctl config default`                    | Print the bundled default `init.lua`                         |
| `gnoblinctl init`                              | Create an editable default config tree from the binary       |
| `gnoblinctl config restore-default`            | Back up and replace the config folder with embedded defaults |
| `gnoblinctl config reload`                     | Validate and apply the complete configuration live           |
| `gnoblinctl shortcut list`                     | List shortcuts registered by the native compositor           |
| `gnoblinctl shortcut actions [GROUP]`          | List built-in shortcut actions, optionally by group          |
| `gnoblinctl shortcut capture`                  | Capture a key combination as a shortcut binding              |
| `gnoblinctl capabilities`                      | List compositor and protocol capabilities                    |
| `gnoblinctl focus history`                     | List windows in focus order                                  |
| `gnoblinctl focus policy`                      | Show the committed focus policy                              |
| `gnoblinctl config show`                       | Show the committed settings snapshot                         |
| `gnoblinctl lua`                               | Open a local Lua console for the session API                 |
| `gnoblinctl ping`                              | Check whether the compositor control socket responds         |

Run `gnoblinctl --help` for the commands and options, `gnoblinctl help window`
for the actions of a command, and `gnoblinctl help window thumbnail` for the
arguments of one action. Each command and action has a one-line description. A
bare group lists its actions. A mistyped command or action names the closest
match, as in `unknown command: windw (did you mean 'window'?)`.

`man gnoblinctl` has the same commands and actions, generated from the same
table as the help. `man gnoblin` covers the login command.

If a Lua configuration reload is already in progress, API commands retry that
temporary response until their `--timeout` expires. Other compositor errors are
reported immediately, and uncertain actions are not retried.

## Command groups

- [Windows](/cli/windows)
- [Input and animations](/cli/input-and-animations)
- [Workspaces, monitors, and shortcuts](/cli/workspaces-monitors-shortcuts)
- [Session and policy](/cli/session-and-policy)
- [Lua console](/cli/lua)
- [Graphical console](/cli/graphical-console)
- [Shell completion](/cli/completion)

Use `gnoblinctl --help` to list commands, `gnoblinctl help GROUP` to list actions, and `gnoblinctl help GROUP ACTION` to see an action synopsis. The help table in `src/cli/gnoblinctl-cli.c` defines command names and one-line descriptions.
