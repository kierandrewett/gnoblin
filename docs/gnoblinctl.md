# gnoblinctl

`gnoblinctl` controls the compositor and Gnoblin shell. Bingux panels and app
menus remain the responsibility of `binguxctl`.

Use `gnoblinctl --help`, `gnoblinctl help window`, or any command's `--help`.
Commands use a group and an action, such as `window list` or `feature enable`.
Running a group on its own shows its available actions.

## Command map

| Group         | Actions                                                                                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `config`      | `path`, `reload`                                                                                                                                                         |
| `window`      | `list`, `focus`, `close`, `minimize`, `restore`, `restore-or-minimize`, `maximize`, `unmaximize`, `fullscreen`, `unfullscreen`, `move`, `resize`, `workspace`, `monitor` |
| `workspace`   | `list`, `switch`                                                                                                                                                         |
| `monitor`     | `list`                                                                                                                                                                   |
| `input`       | `list`, `current`, `select`                                                                                                                                              |
| `feature`     | `list`, `show`, `enable`, `disable`                                                                                                                                      |
| `script`      | `list`, `reload`                                                                                                                                                         |
| `permissions` | `list`, `check`                                                                                                                                                          |
| `grant`       | `list`, `revoke`                                                                                                                                                         |
| `launch`      | `status`, `begin`, `end`                                                                                                                                                 |

`status`, `ping`, `version`, `privacy`, and `reload` are direct commands.
`completion SHELL` prints setup for Bash, Zsh, or Fish. `launch` controls busy-cursor
feedback for shell integrations; it does not start applications.

## Configuration

Keep includes and settings in `~/.config/gnoblin/init.lua`. The CLI shows the
selected file and reloads it:

```sh
gnoblinctl config path
gnoblinctl config reload
```

Edit Lua files directly. To load package and user settings, add these lines to
the main file:

```lua
local g = require("gnoblin")
g.load("/usr/share/gnoblin/conf.d/*.lua")
g.load("conf.d/**/*.lua")
```

See [Configuration](configuration.md) for file watching and load order.

## Output and errors

Structured results use tables in a terminal and JSON in a pipe. Use `--json`
(or `-j`) to request JSON explicitly, or `--format table` to inspect a table
from a script. `ping` and `version` retain plain text unless JSON is requested.
Options can appear before or after the command.

```sh
gnoblinctl status
gnoblinctl feature list
gnoblinctl feature list --json
gnoblinctl input list
gnoblinctl privacy
```

JSON uses named fields. For example, `feature list` returns `{"features":[{"id":"notifications","description":"...","enabled":false}]}`.
Read commands write results to stdout. Errors go to stderr with exit code 1;
invalid arguments use exit code 2. `--timeout SECONDS` accepts 1-60 seconds
and defaults to 5. An uncertain action is never retried automatically.

## Windows

```sh
gnoblinctl window list
gnoblinctl window list --focused
gnoblinctl window list --app-id org.gnome.Nautilus.desktop
gnoblinctl window list --title "project"
gnoblinctl window focus 42
gnoblinctl window minimize 42
gnoblinctl window restore 42
gnoblinctl window maximize active
gnoblinctl window unmaximize active
gnoblinctl window fullscreen 42
gnoblinctl window unfullscreen 42
gnoblinctl window move 42 100 80
gnoblinctl window resize 42 900 600
gnoblinctl window close 42
```

Window IDs come from Mutter and remain stable for a window's lifetime. Refresh
the list after a new login. `active` selects the focused window when the
compositor processes the request. Actions without geometry or a destination
use `active` when their ID is omitted. Each action affects one window.

`restore` removes minimisation. Use `unmaximize` or `unfullscreen` to remove
those states. Move and resize use logical screen coordinates and frame sizes,
including decorations. Applications can constrain their final size. These
operations reject maximised, fullscreen or non-resizable windows as applicable.
`close` requests a normal application close; unsaved-work prompts still apply.

The JSON window list includes full titles, app IDs, geometry, workspace,
monitor index, focus and state. Terminal tables shorten long fields to fit.

## Workspaces and monitors

```sh
gnoblinctl workspace list
gnoblinctl workspace switch 2
gnoblinctl window workspace 42 2
gnoblinctl monitor list
gnoblinctl window monitor 42 0
```

Workspace IDs are one-based positions; logical monitor IDs are zero-based.
Commands accept existing destinations only. GNOME can remove and renumber
empty dynamic workspaces, so refresh `workspace list` before using an old index.
Moving a window does not automatically follow it. `window focus ID` switches
to its workspace and activates it.

A mutation reply with `pending: true` confirms that the compositor accepted
the request. List windows or workspaces again to observe the final state.
Window mutations are rejected while the session is locked.

## Shell completion

```sh
# Bash: add to ~/.bashrc
eval "$(gnoblinctl completion bash)"

# Zsh: run after compinit in ~/.zshrc
eval "$(gnoblinctl completion zsh)"

# Fish
gnoblinctl completion fish > ~/.config/fish/completions/gnoblinctl.fish
```

## Transport and installation

Shell settings use D-Bus through `busctl`. Keyboard calls can use the dedicated
InputSources service when the shell lacks that interface; failed or timed-out
mutations do not trigger fallback. The CLI requires Python 3 and `busctl`.
Nix pins both runtime paths; the RPM session package declares both dependencies.

Window commands use the user-private compositor bridge socket. Override it with
`--socket PATH` or `GNOBLIN_COMPOSITOR_SOCKET`. The default is
`$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`. If the bridge is unavailable, check
`gnoblinctl script list` and update/reload `compositor-bridge.js`.

The bridge accepts `{"op":"command","id":"REQUEST_ID","command":"windows"}`
and replies with `{"event":"reply","id":"REQUEST_ID","result":{"windows":[]}}`.
Errors retain the request ID and use `event: "error"` with a `message`.
Other commands are `monitors`, `workspaces`, `workspace-switch` (with
`workspace`), and `window` (with `action`, `window`, and the action's arguments).
Existing shortcut and streaming-window requests retain their protocol.

## Permission policy

`gnoblinctl permissions list` shows the active policy and its configuration path.
Use `permissions check CAPABILITY IDENTITY` to explain a decision. Edit the
`permissions` table in your Lua configuration to change the policy, then save or
run `gnoblinctl config reload`. Both inspection commands support JSON output.
See [Portal permissions](permissions.md) for examples and supported gates.

## CLI design rules

Keep new commands within this design:

- Use `gnoblinctl GROUP ACTION`; use singular group names and plain verbs.
  Keep common diagnostics at the top level. Do not add aliases or another
  nesting level to expose the same operation.
- A bare group shows contextual help without connecting to the compositor.
  Missing arguments for an action produce a usage error and a useful hint.
- Define names, descriptions, arguments, and choices once in the argument parser.
  Help and completion use that same command tree.
- Keep tables compact and readable, with consistent human labels. Preserve full
  values and named fields in JSON. Never place terminal styling in JSON output.
- Put results on stdout and failures on stderr. Exit 0 for success, 1 for a
  runtime failure, and 2 for invalid usage. Do not retry uncertain mutations.
- Keep configuration in Lua files. Config commands locate and reload that file;
  they do not maintain a second configuration store or rewrite executable Lua.

The local CLI tests exercise command names, help, output, validation, and
transport failures. `tests/test-gnoblinctl.py` checks the installed command
against a private compositor session.
