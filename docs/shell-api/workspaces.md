# Workspaces

> **Status:** not available yet: `:activate`, `:rename`, `gnoblin.workspaces.active`. The sections that use them describe the target, not current behaviour.

For window-level workspace moves, see [Windows](/shell-api/windows). For displays, see [Outputs](/shell-api/outputs).

## Read workspaces

These methods return immediately:

| Method                         | Arguments            | Result                          |
| ------------------------------ | -------------------- | ------------------------------- |
| `gnoblin.workspaces.list()`    | None                 | All current `Workspace` objects |
| `gnoblin.workspaces.by_id(id)` | Integer workspace ID | Matching workspace or `nil`     |

`gnoblin.find("Workspace", filter?)` filters the same objects by their fields. The fields are `id` and `layout`. See
[find](/shell-api/find).

```lua
local first = gnoblin.find("Workspace", {id = 1})[1]
```

Every read of one workspace returns the same object. `workspace.alive` turns `false` once the workspace is gone, and
the object keeps its last state.

## Workspace records

| Field    | Type    | Meaning                                                                                                                     |
| -------- | ------- | --------------------------------------------------------------------------------------------------------------------------- |
| `id`     | Integer | The workspace's identity, stable for its lifetime.                                                                          |
| `layout` | String  | The layout the workspace runs now: its own override when one was set, otherwise its output's default, otherwise `floating`. |

## Layouts

`workspace:set_layout(root, module, hash, options)` sets the workspace's own layout to a layout module. The reference
names the root it resolves under (`shipped` or `user`), the module name, the source hash and the options. The hash pins
the source, so an edit made after the configuration was checked is refused. An unknown module fails with
`unknown-layout`.

`workspace:layout_command(name, params)` sends a command to the layout the workspace runs. `params` is a JSON object,
`{}` when the command takes none. The layout answers with the placements the command produces. `refused` means the
layout does not know the command, and `unknown-layout` means the workspace has no layout to ask.

The shipped `gnoblin.layouts.strip` module places windows as columns in a strip wider than the screen, in the style of
niri. Each column holds one or more windows, stacked or tabbed.

A view offset decides which part of the strip is on screen, and columns outside the view keep their placements. A new
window opens as a column after the focused one.

Set the layout with `module = "gnoblin.layouts.strip"` and `root = "shipped"`. Its options are:

- `column_width`, a fraction of the area from 0 to 1 (default `0.5`);
- `centre`, one of `on-overflow` (default), `always` or `never`.

Its commands take `params` as a JSON object:

- `width`, with `preset` (`"1/3"`, `"1/2"`, `"2/3"` or `"1"`) or `px`, sets the focused column's width;
- `consume` and `expel`, with an optional `window`, move a window into the column on its left or out into a new column;
- `move_column`, with `direction` (`"left"` or `"right"`), swaps the focused column with its neighbour;
- `fit_all` gives every column an equal share and scrolls to the start;
- `toggle_tabbed` switches the focused column between stacked and tabbed;
- `focus`, with `direction` (`"left"`, `"right"`, `"up"` or `"down"`), moves focus to the neighbouring window. With no
  neighbour, focus stays where it is and the command still succeeds;
- `scroll_to`, with `x`, sets the view offset in pixels. The offset is clamped to the strip.

The workspace object signals:

- `layout_changed`, with `layout`, when the workspace's layout changes;
- `layout_failed`, with `layout` and `reason`, when a layout run fails. The workspace then runs floating until a run
  succeeds. `reason` is one of `error`, `timeout`, `disabled`, `vm-wedged`, `invalid-reply` or `invalid-params`.
- `set_activated`, with `set`, when the window manager makes a configured workspace set live. Every workspace the session holds raises it.

## Workspace sets

`gnoblin.workspaces.activate_set(name)` makes the configured workspace set `name` the live set. The sets are declared in the [window management](/config/configure/window_management) configuration. The call returns an operation that completes once the window manager shows the set's workspaces, including when the set is already live. An unknown name fails with `unknown-set`.

```lua
gnoblin.workspaces.activate_set("desk")
```

`gnoblinctl workspace activate-set desk` makes the same request from the command line.

## Not in the schema yet

These names appear in earlier design notes. The `workspace` object does not declare them, so they are not available
from Lua:

- `workspace:activate()`, `workspace:rename(name)` and `workspace:remove()`;
- `workspace:on(signal, fn)` with the `changed`, `activated` and `closed` signals, and `workspace:watch(field, fn)`;
- `gnoblin.workspaces.active()`, `create`, `next` and `previous`;
- the workspace fields `number`, `name`, `active`, `window_count`, `persistent` and `revision`;
- string workspace IDs, such as `"mail"`, and session IDs such as `@session-N`;
- selecting a workspace by its one-based number.
