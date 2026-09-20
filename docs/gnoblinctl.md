# gnoblinctl

[Configuration reference](configuration-reference.md)

Control Gnoblin from a terminal or script. Configure shell panels with that
shell's own tools.

## Start here

```sh
gnoblinctl window list
gnoblinctl config path
gnoblinctl config reload
```

Run `gnoblinctl --help`, `gnoblinctl help window`, or a command's
`--help` for accepted arguments. A bare group lists its actions.

## Windows

```sh
gnoblinctl window list
gnoblinctl window focus 42
gnoblinctl window minimize active
gnoblinctl window restore active
gnoblinctl window maximize active
gnoblinctl window close 42
```

Use an ID from `window list`, or `active` for the focused window.
IDs last for the window's lifetime, not across logins.

`restore` removes minimisation. Use `unmaximize` and `unfullscreen`
for those states. `close` requests a normal close, including unsaved-work prompts.

Filter the list with `--focused`, `--app-id ID` or `--title TEXT`.

## Move and resize

```sh
gnoblinctl window move 42 100 80
gnoblinctl window resize 42 900 600
gnoblinctl window workspace 42 2
gnoblinctl window monitor 42 0
```

Coordinates and frame sizes use logical pixels, including decorations.
Apps can constrain the result. Geometry operations reject incompatible states
such as fullscreen or non-resizable windows.

## Workspaces and monitors

```sh
gnoblinctl workspace list
gnoblinctl workspace switch 2
gnoblinctl monitor list
```

Workspace positions start at **1**; monitor IDs start at **0**.
Dynamic workspaces can be renumbered. Refresh the list before reusing an index.

Moving a window does not follow it. Focus that window to switch to its workspace.

## Other commands

| Group         | Actions                     |
| ------------- | --------------------------- |
| `config`      | path, reload                |
| `input`       | list, current, select       |
| `feature`     | list, show, enable, disable |
| `script`      | list                        |
| `permissions` | list, check                 |
| `grant`       | list, revoke                |
| `launch`      | status, begin, end          |

Direct commands include `ping`, `status`, `version`, `privacy` and
`reload`. Reload also refreshes the theme and user scripts.

See [permission policy](permissions.md) and [launch feedback](launch-feedback.md).
Launch feedback does not start an application.

## Output for scripts

```sh
gnoblinctl window list --json
gnoblinctl feature list --format table
```

Structured results use tables in a terminal and JSON in a pipe.
`--json` forces JSON. Options work before or after the command.

For example, `gnoblinctl window list --focused --json` returns this shape.
IDs, titles and geometry below are illustrative:

```json
{
    "windows": [
        {
            "id": "42",
            "title": "Notes",
            "appId": "org.example.Editor.desktop",
            "focused": true,
            "minimized": false,
            "workspace": 1,
            "monitorIndex": 0,
            "maximized": false,
            "fullscreen": false,
            "geometry": { "x": 100, "y": 80, "width": 900, "height": 600 },
            "lastUserTime": 123456,
            "parent": null,
            "monitor": { "x": 0, "y": 0 }
        }
    ]
}
```

With `jq` installed, print just the focused window ID:

```sh
gnoblinctl window list --focused --json | jq -r '.windows[].id'
```

A successful `gnoblinctl window minimize 42 --json` returns:

```json
{ "ok": true, "pending": true, "window": "42", "action": "minimize" }
```

For workspace lists, `gnoblinctl workspace list --json` returns:

```json
{
    "workspaces": [
        { "id": 1, "active": true, "windows": 2 },
        { "id": 2, "active": false, "windows": 0 }
    ]
}
```

| Exit code | Meaning                          |
| --------- | -------------------------------- |
| 0         | Success                          |
| 1         | Runtime failure; error on stderr |
| 2         | Invalid arguments                |

`--timeout SECONDS` accepts 1–60; default is 5.
Uncertain actions are not retried automatically.

A reply with `pending: true` means accepted, not finished.
List state again to confirm the result. Window changes are rejected while locked.

## Shell completion

```sh
# Bash: ~/.bashrc
eval "$(gnoblinctl completion bash)"

# Zsh: ~/.zshrc, after compinit
eval "$(gnoblinctl completion zsh)"

# Fish
gnoblinctl completion fish > ~/.config/fish/completions/gnoblinctl.fish
```

## Connection problems

The CLI needs Python 3 and `busctl`.
Settings use D-Bus; window commands use the
[compositor bridge](compositor-bridge.md).

The socket defaults to `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Override it with `--socket PATH` or `GNOBLIN_COMPOSITOR_SOCKET`.

If it is missing, check `gnoblinctl script list` and the bridge installation.
See [CLI development](cli-development.md) for the transport contract.
