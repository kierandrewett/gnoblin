# gnoblinctl

[Configuration reference](/config/configure)

Control Gnoblin from a terminal or script. Configure shell panels with that
shell's own tools.

## Start here

Run commands from a terminal inside Gnoblin:

| Command                     | Use it to                              |
| --------------------------- | -------------------------------------- |
| `gnoblinctl window list`    | Find open windows and their IDs        |
| `gnoblinctl window match`   | Show values available to window rules  |
| `gnoblinctl layer list`     | Find layer-shell rule namespaces       |
| `gnoblinctl config path`    | Find the config file your session uses |
| `gnoblinctl config default` | Print the bundled default `init.lua`   |
| `gnoblinctl config reload`  | Apply edits and report config errors   |

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

To see the values used by `gnoblin.window_rule`, run `gnoblinctl window match`
for the focused window or pass an ID from `window list`. The output includes
the desktop-entry ID, GTK application ID, WM class, and the effective rule
`app_id`; use that `app_id` and the exact title in your rule. The desktop-entry
ID shown by `window list` can differ from the rule value.

`restore` removes minimisation. Use `unmaximize` and `unfullscreen`
for those states. `close` requests a normal close, including unsaved-work prompts.

Filter the list with `--focused`, `--app-id ID` or `--title TEXT`.

## Layer surfaces

List layer-shell surfaces and the namespaces available to window rules with:

```sh
gnoblinctl layer list
```

Use a listed `namespace` as the rule's `layer` value.
The `animation surfaces` command reports these targets for layer previews.

## Animations

Use `gnoblinctl` to inspect and step through a named animation on a window or
layer-shell surface. A preview starts paused and changes only the target's
visual transform; it does not minimize or close the target.

```sh
gnoblinctl animation list
gnoblinctl animation surfaces
gnoblinctl animation inspect gnome-open --window active
session=$(gnoblinctl animation preview gnome-open --window active --format json | python3 -c 'import json,sys; print(json.load(sys.stdin)["session"])')
gnoblinctl animation seek "$session" 50
gnoblinctl animation step "$session" 16
gnoblinctl animation play "$session"
gnoblinctl animation pause "$session"
gnoblinctl animation stop "$session"
```

Omitting `--window` uses the active window. For layer surfaces, choose
`--layer ID` or `--namespace NAME`; a namespace must resolve to exactly one
visible surface. `seek` accepts a percentage from 0 to 100; `step` advances by
milliseconds. `stop` restores the target's original visual state.

`animation list` marks entries that can be previewed against a window or layer
surface. Workspace, console, shadow, tile-preview, dialog-dimming, and
layer-companion animations run on internal compositor actors and effects, so
they cannot be previewed against an external target.

See the [animation guide](/guides/animations) for custom curves, events, and
the GNOME and Gnoblin presets.

## Move and resize

```sh
gnoblinctl window move 42 100 80
gnoblinctl window resize 42 900 600
gnoblinctl window workspace 42 2
gnoblinctl window monitor 42 0
```

The move example places the window at `(100, 80)` on the desktop; the resize
example sets its outer size to 900 × 600 logical pixels, including its frame.
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

## Window actions

Actions without extra arguments accept an optional window ID; they use
`active` if omitted. The geometry actions require the ID and numbers shown.

| Action                                   | Arguments after action | Effect                                                 |
| ---------------------------------------- | ---------------------- | ------------------------------------------------------ |
| `menu`                                   | `[ID]`                 | Open the window menu                                   |
| `interactive-move`, `interactive-resize` | `[ID]`                 | Begin pointer-driven move or resize                    |
| `above`, `unabove`                       | `[ID]`                 | Set or clear always-on-top                             |
| `stick`, `unstick`                       | `[ID]`                 | Show on all workspaces or only its own                 |
| `focus`, `close`, `minimize`             | `[ID]`                 | Focus, request close, or minimize                      |
| `restore-or-minimize`                    | `[ID]`                 | Restore a maximized/snapped window; otherwise minimize |
| `restore`, `maximize`, `unmaximize`      | `[ID]`                 | Change minimization or maximization                    |
| `fullscreen`, `unfullscreen`             | `[ID]`                 | Enter or leave fullscreen                              |
| `move`                                   | `ID X Y`               | Set frame position; each coordinate: −100000–100000    |
| `resize`                                 | `ID WIDTH HEIGHT`      | Set frame size; each dimension: 1–32768                |
| `workspace`                              | `ID WORKSPACE`         | Move to a one-based workspace: 1–1024                  |
| `monitor`                                | `ID MONITOR`           | Move to a zero-based monitor: 0–1024                   |

## Shell and policy commands

| Command                                                     | Use                                                                                |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `ping`, `version`, `status`                                 | Check the shell, build version and window bridge                                   |
| `reload`                                                    | Refresh the Shell, theme and installed/personal scripts while keeping windows      |
| `config path`, `config default`, `config reload`            | Find the active config, print the bundled example, or reload                       |
| `input list`, `input current`                               | Inspect configured and selected keyboard sources                                   |
| `input select TYPE ID`                                      | Select an exact source from `input list`                                           |
| `feature list`, `feature show ID`                           | Inspect live Shell switches                                                        |
| `feature enable ID`, `feature disable ID`                   | Change a switch                                                                    |
| `script list`                                               | List loaded package integrations and personal scripts                              |
| `privacy`                                                   | Read screen-sharing, microphone and location indicators                            |
| `permissions list`                                          | Read portal rules and capabilities                                                 |
| `permissions check CAPABILITY IDENTITY`                     | Explain a decision for `app-id:…` or `host-exe:…`                                  |
| `grant list`, `grant revoke KIND ID`                        | List or revoke persistent portal grants; kind is `screen-cast` or `remote-desktop` |
| `launch status`                                             | List pending launch feedback                                                       |
| `launch begin TOKEN APP [MILLISECONDS]`, `launch end TOKEN` | Start or end busy-cursor feedback; duration defaults to 3000 ms, range 1–60000 ms  |

For example, to inspect a portal decision and change keyboard source:

```sh
gnoblinctl permissions check screen-cast app-id:org.example.Recorder
gnoblinctl input list --json
gnoblinctl input select xkb us
```

Use a capability from `permissions list` and a source from `input list`.
See [permission policy](/guides/permissions) and [launch feedback](launch-feedback.md).
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

The bridge is built into current Gnoblin source builds, so `script list` does
not show it. Package integrations that add namespaced operations do appear in
`script list`; for example, Bingux installs its text-entry integration with
Bingux. Check `gnoblinctl status`, the running Gnoblin version and the session
log. Older installed builds may not include the built-in service yet. See
[CLI development](cli-development.md) for the transport contract.
