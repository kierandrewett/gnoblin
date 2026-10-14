# Workspace, Monitor, and Shortcut Commands

Inspect and manage workspaces, list monitors, and inspect or capture shortcuts.

Commands require a running Gnoblin session. Shortcut capture observes local key input until completion or cancellation.

## Workspaces and monitors

```sh
gnoblinctl workspace list
gnoblinctl workspace create --name "Build"
gnoblinctl workspace create --id build --name "Build" --activate
gnoblinctl workspace rename --id build --name "Compile"
gnoblinctl workspace remove --id build
gnoblinctl workspace switch --number 2
gnoblinctl workspace switch --id code
gnoblinctl workspace next
gnoblinctl workspace previous
gnoblinctl workspace move-active --id code --follow
gnoblinctl workspace move-active --number 2
gnoblinctl monitor list
```

Workspace numbers are one-based positions. They can change when dynamic
workspaces are removed.

Configured IDs follow their workspace when reordered. Generated IDs such as
`@session-1` last for the session. Names are display labels. Use
`workspace list` to see each ID, number, name, active state, and window count.

Monitor IDs are active connector names, such as `DP-1` or `eDP-1`. Copy the
exact ID from `gnoblinctl monitor list`.

Each monitor also has a current numeric `index`, which can change when outputs
are added or removed. Cloned outputs use the lexicographically first active
connector as their ID.

`gnoblinctl monitor list` reads the API 1.37 `monitors.list` snapshot. Its JSON
output keeps the `monitors` object, and each record includes its state revision.
In a terminal, it shows connector, index, display name, resolution, position,
scale, primary state, and enabled state:

```text
ID         INDEX  NAME               RESOLUTION  POSITION  SCALE  PRIMARY  ENABLED
---------  -----  -----------------  ----------  --------  -----  -------  -------
Virtual-1  0      Red Hat, Inc. 14`"  1920x1080   0+0       1.0    yes      yes
```

The standalone compositor supports workspace list, create, rename, remove,
switch, next, previous, and window moves by ID or number. The list includes
configured IDs, generated session IDs, active state, and window counts.

To remove a temporary workspace, switch away from it and move or close its
windows first. Remove a configured workspace by editing the config.

`workspace create` requires `--name`:

- `--id ID` assigns an ID for this session.
- `--activate` switches to the new workspace.
- Without `--id`, Gnoblin assigns a session-only ID such as `@session-1`.

Runtime-created workspaces are temporary. `workspace rename` and
`workspace remove` require exactly one of `--id` or `--number`. Gnoblin refuses
to remove a persistent, active, or occupied workspace.

Use `--number NUMBER` to select a workspace by its current one-based position.
The positional form `workspace switch NUMBER` is a legacy shorthand; prefer
the explicit selector.

`workspace move-active` moves the focused window. It switches to the destination
only when `--follow` is supplied. `window workspace` accepts an ID or a number:

```sh
gnoblinctl window workspace 42 --id code
gnoblinctl window workspace 42 --number 2
```

## Capture a shortcut

Run `gnoblinctl shortcut capture`, then press the key combination. The command
prints its GTK accelerator. Press Escape to cancel. Bare Super prints the
special `Super` binding.

The default timeout is 30 seconds. Set `--timeout` to an integer from 1 to 60
seconds to change it. If a session lock, input-capture session, or stage grab
starts during capture, the command reports cancellation and releases keyboard
input to that owner.

## List configured shortcuts

Run `gnoblinctl shortcut list` to list shortcuts registered by the native
compositor. External-client shortcuts and disabled declarations are not
included.

Each record includes its name, binding, enabled state, trigger, revision, and
either a command or built-in action. The terminal table includes the name,
binding, trigger, and enabled state; multiple accelerators appear as a JSON
array. The command reads the shared Lua snapshot through API 1.40.

```text
NAME      BINDING       TRIGGER  ENABLED
--------  ------------  -------  -------
terminal  <Super>e      press    yes
```

Use `--json` for structured output in a terminal or pipe:

```sh
gnoblinctl shortcut list --json
```

## List built-in shortcut actions

Run `gnoblinctl shortcut actions` to list actions that can be assigned in Lua
configuration. Pass `wm`, `mutter`, or `wayland` to show one group:

```sh
gnoblinctl shortcut actions wm
```

Use `--json` to preserve the complete action records in a pipe.
The command reads the shared Lua API through `shortcuts.actions` (API 1.41).
