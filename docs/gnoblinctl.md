# gnoblinctl

`gnoblinctl` controls the compositor and Gnoblin shell. Bingux panels and app
menus remain the responsibility of `binguxctl`.

Use `gnoblinctl --help`, `gnoblinctl help window`, or any command's `--help`.
Existing commands such as `enable`, `disable`, `reload-config`, `reload-ext`
and `set-input-source` remain available.

## Configuration fragments

Installations can ship a Gnoblin TOML fragment without overwriting the user's
configuration. Add one to the active TOML configuration and reload it with:

```sh
gnoblinctl load-config /usr/share/bingux/gnoblin.toml
```

The include is idempotent and the file update is atomic. If Gnoblin is not
running yet, the command reports `reload: pending`; the next shell start will
load the fragment. If validation fails, the user's previous configuration is
restored and the actual setting and fragment path are reported. Legacy
`gnoblin.conf` files must be migrated to `gnoblin.toml` before using this
command.

Remove the include before uninstalling its provider:

```sh
gnoblinctl unload-config /usr/share/bingux/gnoblin.toml
```

Unloading is idempotent. The include is removed even if the live reload finds
an unrelated existing configuration error; fix that error and run
`gnoblinctl reload-config` afterward.

## Output and errors

Structured results use tables in a terminal and JSON in a pipe. Use `--json`
(or `-j`) to request JSON explicitly, or `--format table` to inspect a table
from a script. `ping` and `version` retain plain text unless JSON is requested.
Options can appear before or after the command.

```sh
gnoblinctl status
gnoblinctl features
gnoblinctl features --json
gnoblinctl input-sources
gnoblinctl privacy
```

JSON now contains named fields rather than textual GVariant tuples. Scripts
that parsed the old tuple output must migrate to the JSON fields. For example,
`features` returns `{"features":[{"id":"osd","description":"...","enabled":false}]}`.
Read commands write results to stdout. Errors go to stderr with exit code 1;
invalid arguments use exit code 2. `--timeout SECONDS` accepts 1-60 seconds
and defaults to 5. An uncertain action is never retried automatically.

## Windows

```sh
gnoblinctl windows
gnoblinctl windows --focused
gnoblinctl windows --app-id org.gnome.Nautilus.desktop
gnoblinctl windows --title "project"
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
gnoblinctl workspaces
gnoblinctl workspaces switch 2
gnoblinctl window workspace 42 2
gnoblinctl monitors
gnoblinctl window monitor 42 0
```

Workspace IDs are one-based positions; logical monitor IDs are zero-based.
Commands accept existing destinations only. GNOME can remove and renumber
empty dynamic workspaces, so refresh `workspaces` before using an old index.
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
`gnoblinctl scripts` and update/reload `compositor-bridge.js`.

The bridge accepts `{"op":"command","id":"REQUEST_ID","command":"windows"}`
and replies with `{"event":"reply","id":"REQUEST_ID","result":{"windows":[]}}`.
Errors retain the request ID and use `event: "error"` with a `message`.
Other commands are `monitors`, `workspaces`, `workspace-switch` (with
`workspace`), and `window` (with `action`, `window`, and the action's arguments).
Existing shortcut and streaming-window requests retain their protocol.

## Permission policy

`gnoblinctl permissions list` shows the active policy and its configuration path.
Use `permissions set`, `permissions remove`, and `permissions default` to change
it. `permissions check <capability> <identity>` explains a decision. All these
commands support JSON output. See [Portal permissions](permissions.md) for
examples and the supported gates.
